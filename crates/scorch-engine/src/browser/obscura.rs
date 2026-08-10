use std::{sync::Arc, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use obscura_browser::{BrowserContext, CaptureRegion, Page};
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

    #[allow(clippy::too_many_arguments)]
    pub async fn render(
        &self,
        permit: OwnedSemaphorePermit,
        url: &str,
        request_timeout: Duration,
        wait_for: Duration,
        block_media: bool,
        screenshot: bool,
        full_page: bool,
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
                screenshot,
                full_page,
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

#[allow(clippy::too_many_arguments)]
async fn render_page(
    config: &EngineConfig,
    proxy_url: &str,
    url: &str,
    request_timeout: Duration,
    wait_for: Duration,
    block_media: bool,
    screenshot: bool,
    full_page: bool,
) -> Result<RenderedPage> {
    debug!(browser = "obscura", %url, "creating isolated browser page");
    let context = Arc::new(BrowserContext::with_options(
        format!("scorch-{}", Uuid::now_v7()),
        Some(proxy_url.to_owned()),
        true,
    ));
    let mut page = Page::new(format!("page-{}", Uuid::now_v7()), context);
    page.set_viewport(VIEWPORT);
    page.set_navigation_timeout(request_timeout);
    if block_media {
        page.set_blocked_urls(media_block_patterns());
    }
    page.navigate(url)
        .await
        .map_err(|error| EngineError::Browser(error.to_string()))?;
    if !wait_for.is_zero() {
        page.settle_for_duration(duration_millis(wait_for)).await;
    }

    let final_url = match page.url_string() {
        value if value.is_empty() => url.to_owned(),
        value => value,
    };
    let html = page
        .evaluate_with_timeout("document.documentElement.outerHTML", request_timeout)
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| EngineError::Browser("Obscura could not serialize the page DOM".into()))?;
    ensure_size(html.len(), config.max_response_bytes)?;

    let screenshot = if screenshot {
        page.prepare_screenshot_resources(duration_millis(request_timeout).min(1_000))
            .await;
        let bytes = capture_screenshot(&page, full_page)?;
        ensure_size(bytes.len(), config.max_response_bytes)?;
        Some(format!("data:image/png;base64,{}", STANDARD.encode(bytes)))
    } else {
        None
    };
    Ok(RenderedPage {
        html,
        final_url,
        screenshot,
    })
}

fn capture_screenshot(page: &Page, full_page: bool) -> Result<Vec<u8>> {
    if full_page {
        let (width, height) = page.prepared_content_size().ok_or_else(|| {
            EngineError::Browser("Obscura could not determine the document size".into())
        })?;
        return page
            .screenshot_region(CaptureRegion::new(0.0, 0.0, width, height, 1.0))
            .map_err(|error| {
                EngineError::Browser(format!("Obscura screenshot failed: {error:?}"))
            });
    }
    page.screenshot(VIEWPORT)
        .ok_or_else(|| EngineError::Browser("Obscura screenshot failed".into()))
}

fn ensure_size(size: usize, limit: usize) -> Result<()> {
    if size > limit {
        return Err(EngineError::ResponseTooLarge(limit));
    }
    Ok(())
}

fn media_block_patterns() -> Vec<String> {
    [
        "*.png", "*.jpg", "*.jpeg", "*.gif", "*.webp", "*.svg", "*.mp4", "*.webm", "*.mp3",
        "*.woff", "*.woff2",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}
