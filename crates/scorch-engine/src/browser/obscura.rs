use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};

use obscura_browser::{BrowserContext, Page};
use tokio::{sync::OwnedSemaphorePermit, task::spawn_blocking, time::timeout};
use tracing::debug;
use uuid::Uuid;

use crate::{
    browser::RenderedPage,
    config::EngineConfig,
    error::{EngineError, Result},
};

const VIEWPORT: (f32, f32) = (1280.0, 720.0);

pub(super) struct ObscuraBackend {
    config: EngineConfig,
    proxy_url: String,
}

impl ObscuraBackend {
    pub fn new(config: EngineConfig, proxy_url: String) -> Self {
        Self { config, proxy_url }
    }

    pub async fn render(
        &self,
        permit: OwnedSemaphorePermit,
        url: &str,
        request_timeout: Duration,
        wait_for: Duration,
        block_media: bool,
    ) -> Result<RenderedPage> {
        let config = self.config.clone();
        let proxy_url = self.proxy_url.clone();
        let url = url.to_owned();
        let task = spawn_blocking(move || {
            let _permit = permit;
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| {
                    EngineError::Browser(format!("failed to start Obscura runtime: {error}"))
                })?;
            runtime.block_on(render_page(
                &config,
                &proxy_url,
                &url,
                request_timeout,
                wait_for,
                block_media,
            ))
        });
        timeout(request_timeout, task)
            .await
            .map_err(|_| EngineError::Timeout)?
            .map_err(|error| {
                EngineError::Browser(format!("Obscura worker stopped unexpectedly: {error}"))
            })?
    }
}

async fn render_page(
    config: &EngineConfig,
    proxy_url: &str,
    url: &str,
    request_timeout: Duration,
    wait_for: Duration,
    block_media: bool,
) -> Result<RenderedPage> {
    let started = Instant::now();
    let context = Arc::new(BrowserContext::with_options(
        format!("scorch-{}", Uuid::now_v7()),
        Some(proxy_url.to_owned()),
        config.obscura_stealth,
    ));
    let context_elapsed = started.elapsed();
    let page_started = Instant::now();
    let mut page = Page::new(format!("page-{}", Uuid::now_v7()), context);
    let page_elapsed = page_started.elapsed();
    page.set_viewport(VIEWPORT);
    page.set_navigation_timeout(request_timeout);
    page.set_blocked_urls(blocked_url_patterns(block_media));
    let navigation_started = Instant::now();
    page.navigate(url)
        .await
        .map_err(|error| EngineError::Browser(error.to_string()))?;
    let navigation_elapsed = navigation_started.elapsed();
    let remaining = remaining_time(started, request_timeout)?;
    if !wait_for.is_zero() {
        if wait_for >= remaining {
            return Err(EngineError::Timeout);
        }
        page.settle_for_duration(duration_millis(wait_for)).await;
    }

    let final_url = match page.url_string() {
        value if value.is_empty() => url.to_owned(),
        value => value,
    };
    let serialization_started = Instant::now();
    let html = page
        .evaluate_with_timeout(
            "document.documentElement.outerHTML",
            remaining_time(started, request_timeout)?,
        )
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| EngineError::Browser("Obscura could not serialize the page DOM".into()))?;
    let serialization_elapsed = serialization_started.elapsed();
    ensure_size(html.len(), config.max_response_bytes)?;
    let (status, content_type, headers) = document_response(&page);

    debug!(
        browser = "obscura",
        stealth = config.obscura_stealth,
        context_ms = context_elapsed.as_millis(),
        page_ms = page_elapsed.as_millis(),
        navigation_ms = navigation_elapsed.as_millis(),
        serialization_ms = serialization_elapsed.as_millis(),
        total_ms = started.elapsed().as_millis(),
        "browser render phases completed"
    );
    Ok(RenderedPage {
        html,
        final_url,
        status,
        content_type,
        headers,
    })
}

/// Status and headers of the main document, read off the page's own navigation
/// record.
///
/// The browser clears these per navigation and appends one `Document` event per
/// hop, so the last one is the response the DOM was built from. A page that
/// never recorded one (`about:blank`, a navigation that produced no HTTP
/// response) reports 200, matching the synthesized response this replaces.
fn document_response(page: &Page) -> (u16, Option<String>, BTreeMap<String, String>) {
    let Some(event) = page
        .network_events
        .iter()
        .rev()
        .find(|event| event.resource_type == "Document")
    else {
        return (
            200,
            Some("text/html; charset=utf-8".into()),
            BTreeMap::new(),
        );
    };
    let lowercased: BTreeMap<String, String> = event
        .response_headers
        .iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
        .collect();
    let content_type = lowercased.get("content-type").cloned();
    let headers = crate::fetch::REPORTED_HEADERS
        .into_iter()
        .filter_map(|name| {
            lowercased
                .get(name)
                .map(|value| (name.to_owned(), value.clone()))
        })
        .collect();
    (event.status, content_type, headers)
}

fn remaining_time(started: Instant, timeout: Duration) -> Result<Duration> {
    let remaining = timeout.saturating_sub(started.elapsed());
    if remaining.is_zero() {
        return Err(EngineError::Timeout);
    }
    Ok(remaining)
}

fn ensure_size(size: usize, limit: usize) -> Result<()> {
    if size > limit {
        return Err(EngineError::ResponseTooLarge(limit));
    }
    Ok(())
}

/// Stylesheets never reach the caller: extraction reads the serialized DOM, not
/// computed style, so fetching them only adds round trips. They are blocked on
/// every render.
const STYLESHEET_PATTERNS: [&str; 2] = ["*.css", "*.css?*"];

const MEDIA_PATTERNS: [&str; 11] = [
    "*.png", "*.jpg", "*.jpeg", "*.gif", "*.webp", "*.svg", "*.mp4", "*.webm", "*.mp3", "*.woff",
    "*.woff2",
];

fn blocked_url_patterns(block_media: bool) -> Vec<String> {
    let media = if block_media {
        MEDIA_PATTERNS.as_slice()
    } else {
        &[]
    };
    STYLESHEET_PATTERNS
        .iter()
        .chain(media)
        .map(|pattern| (*pattern).to_owned())
        .collect()
}

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}
