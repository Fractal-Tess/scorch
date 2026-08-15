use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use obscura_browser::{BrowserContext, Page, StealthHttpClient};
use tokio::{
    sync::{OwnedSemaphorePermit, mpsc, oneshot},
    time::timeout,
};
use tracing::debug;
use uuid::Uuid;

use crate::{
    browser::RenderedPage,
    config::EngineConfig,
    error::{EngineError, Result},
};

const VIEWPORT: (f32, f32) = (1280.0, 720.0);
const SLOT_RESOURCE_CACHE_MAX_ENTRIES: usize = 128;
const PROCESS_RESOURCE_CACHE_MAX_BYTES: usize = 128 * 1024 * 1024;

/// A pool of render slots, one per allowed concurrent render.
///
/// Each slot is a thread with its own Tokio runtime, browser context, and
/// stealth HTTP client, all of which outlive the renders that run on it. That
/// is the whole point: a connection pool belongs to the runtime whose reactor
/// drives it, so a runtime built per render throws its connections away with
/// itself and every render pays a fresh TCP and TLS handshake to the origin.
/// Across twelve pages this halved the median scrape time on the browser path
/// and raised throughput at concurrency eight by 53 percent, with identical
/// extracted Markdown. `examples/pool_probe.rs` is the measurement this came
/// from.
///
/// Renders that land on the same slot therefore share connections and TLS
/// sessions with each other, which is weaker isolation than a context per
/// render gave. Cookies are still isolated: the slot's jar is cleared before
/// each render. The only retained response bodies are explicitly public,
/// anonymous scripts accepted by Obscura's bounded HTTP cache; documents and
/// credentialed responses never enter it. Requests still go through the safe
/// proxy, and a reused tunnel is pinned to the address the policy already
/// approved when it was opened, so reuse cannot reach an address that was never
/// validated.
pub(super) struct ObscuraBackend {
    slots: Vec<mpsc::Sender<Job>>,
    /// Indices of slots that are not rendering. A worker publishes its own
    /// index here when it finishes, before releasing the permit that admitted
    /// the render, so a caller holding a permit always finds a free slot.
    idle: Arc<Mutex<Vec<usize>>>,
}

struct Job {
    url: String,
    request_timeout: Duration,
    wait_for: Duration,
    block_media: bool,
    reply: oneshot::Sender<Result<RenderedPage>>,
    /// Held by the worker for the whole render. A caller that times out and
    /// walks away must not let the next render into a slot that is still busy,
    /// so the permit is released by whoever actually finishes the work.
    permit: OwnedSemaphorePermit,
}

impl ObscuraBackend {
    pub fn new(config: EngineConfig, proxy_url: String) -> Self {
        let slot_count = config.max_concurrency;
        let idle = Arc::new(Mutex::new((0..slot_count).rev().collect::<Vec<_>>()));
        let slots = (0..slot_count)
            .map(|index| {
                // Capacity one is enough: a slot is only handed out while it is
                // idle, and it does not become idle again until the worker has
                // taken the previous job out of the channel and finished it.
                let (sender, receiver) = mpsc::channel(1);
                let config = config.clone();
                let proxy_url = proxy_url.clone();
                let idle = Arc::clone(&idle);
                thread::Builder::new()
                    .name(format!("scorch-render-{index}"))
                    .spawn(move || worker(index, config, proxy_url, receiver, idle))
                    .expect("the operating system can start a render thread");
                sender
            })
            .collect();
        Self { slots, idle }
    }

    pub async fn render(
        &self,
        permit: OwnedSemaphorePermit,
        url: &str,
        request_timeout: Duration,
        wait_for: Duration,
        block_media: bool,
    ) -> Result<RenderedPage> {
        let slot = self
            .idle
            .lock()
            .ok()
            .and_then(|mut idle| idle.pop())
            .ok_or_else(|| EngineError::Browser("no render slot is available".into()))?;
        let (reply, response) = oneshot::channel();
        let job = Job {
            url: url.to_owned(),
            request_timeout,
            wait_for,
            block_media,
            reply,
            permit,
        };
        // Sent without awaiting: the slot came off the idle list, so its channel
        // has room, and an await here would be a point at which a cancelled
        // request could drop the job after taking the slot but before anything
        // could hand it back. A failure means the worker thread is gone, which
        // under `panic = "abort"` cannot happen without taking the process with
        // it, and the slot is deliberately not returned because it has no worker
        // left to run anything.
        self.slots[slot]
            .try_send(job)
            .map_err(|_| EngineError::Browser("render slot stopped unexpectedly".into()))?;
        timeout(request_timeout, response)
            .await
            .map_err(|_| EngineError::Timeout)?
            .map_err(|_| EngineError::Browser("Obscura worker stopped unexpectedly".into()))?
    }
}

/// State a slot keeps between renders.
struct Slot {
    context: Arc<BrowserContext>,
    /// The stealth client minted by this slot's first page. `Page::new` builds a
    /// fresh one every time. Handing the first page's client to later pages is
    /// what reuses stealth connections across renders assigned to this slot.
    stealth: Option<Arc<StealthHttpClient>>,
}

fn worker(
    index: usize,
    config: EngineConfig,
    proxy_url: String,
    mut jobs: mpsc::Receiver<Job>,
    idle: Arc<Mutex<Vec<usize>>>,
) {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return;
    };
    // The whole loop runs inside one `block_on` so the reactor keeps running
    // while the slot waits for work. Pooled connections are owned by tasks on
    // this runtime; if it stopped between renders, those tasks would not see a
    // peer close its end until a later render tried to write to it.
    runtime.block_on(async move {
        let mut slot: Option<Slot> = None;
        while let Some(job) = jobs.recv().await {
            let Job {
                url,
                request_timeout,
                wait_for,
                block_media,
                reply,
                permit,
            } = job;
            let slot = slot.get_or_insert_with(|| Slot {
                context: Arc::new(BrowserContext::with_options(
                    format!("scorch-slot-{index}"),
                    Some(proxy_url.clone()),
                    crate::OBSCURA_STEALTH,
                )),
                stealth: None,
            });
            // Nothing of the previous render may be visible to this one.
            slot.context.cookie_jar.clear();
            let result =
                render_page(slot, &config, &url, request_timeout, wait_for, block_media).await;
            let _ = reply.send(result);
            if let Ok(mut idle) = idle.lock() {
                idle.push(index);
            }
            drop(permit);
        }
    });
}

async fn render_page(
    slot: &mut Slot,
    config: &EngineConfig,
    url: &str,
    request_timeout: Duration,
    wait_for: Duration,
    block_media: bool,
) -> Result<RenderedPage> {
    let started = Instant::now();
    let mut page = Page::new(
        format!("page-{}", Uuid::now_v7()),
        Arc::clone(&slot.context),
    );
    match &slot.stealth {
        Some(client) => page.stealth_client = Some(Arc::clone(client)),
        None => {
            if let Some(client) = &page.stealth_client {
                // Each long-lived render slot owns one cache. Divide one
                // process-wide budget between them so raising concurrency does
                // not multiply retained script bodies without bound.
                client.set_resource_cache_limits(
                    SLOT_RESOURCE_CACHE_MAX_ENTRIES,
                    PROCESS_RESOURCE_CACHE_MAX_BYTES / config.max_concurrency,
                );
            }
            slot.stealth = page.stealth_client.clone();
        }
    }
    let page_elapsed = started.elapsed();
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
    let cache_candidate = same_url(url, &final_url);
    if cache_candidate {
        page.sync_js_network_events();
    }
    let (status, content_type, headers, mut cache_ttl) = document_response(&page, cache_candidate);
    if cache_candidate && !slot.context.cookie_jar.get_all_cookies().is_empty() {
        cache_ttl = None;
    }
    cache_ttl = cache_ttl
        .and_then(|ttl| ttl.checked_sub(navigation_started.elapsed()))
        .filter(|ttl| !ttl.is_zero());
    let cache_observed_at = cache_ttl.map(|_| Instant::now());

    debug!(
        browser = "obscura",
        stealth = crate::OBSCURA_STEALTH,
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
        cache_ttl,
        cache_observed_at,
    })
}

/// Status and headers of the main document, read off the page's own navigation
/// record.
///
/// The browser clears these per navigation and appends one `Document` event per
/// hop, so the last one is the response the DOM was built from. A page that
/// never recorded one (`about:blank`, a navigation that produced no HTTP
/// response) reports 200, matching the synthesized response this replaces.
fn document_response(
    page: &Page,
    check_cache: bool,
) -> (
    u16,
    Option<String>,
    BTreeMap<String, String>,
    Option<Duration>,
) {
    let document_events = page
        .network_events
        .iter()
        .filter(|event| event.resource_type == "Document")
        .collect::<Vec<_>>();
    let Some(event) = document_events.last() else {
        return (
            200,
            Some("text/html; charset=utf-8".into()),
            BTreeMap::new(),
            None,
        );
    };
    let lowercased = lowercase_headers(&event.response_headers);
    let content_type = lowercased.get("content-type").cloned();
    let headers = crate::fetch::REPORTED_HEADERS
        .into_iter()
        .filter_map(|name| {
            lowercased
                .get(name)
                .map(|value| (name.to_owned(), value.clone()))
        })
        .collect();
    let cache_ttl = if check_cache {
        page_cache_ttl(page, document_events.len())
    } else {
        None
    };
    (event.status, content_type, headers, cache_ttl)
}

fn page_cache_ttl(page: &Page, document_count: usize) -> Option<Duration> {
    let has_private_response = page.network_events.iter().any(|event| {
        let headers = lowercase_headers(&event.response_headers);
        headers.contains_key("set-cookie")
            || headers
                .get("cache-control")
                .is_some_and(|value| crate::fetch::cache_control_is_private(value))
    });
    if has_private_response {
        return None;
    }

    page.network_events
        .iter()
        .filter(|event| {
            event.resource_type == "Document"
                || ((200..300).contains(&event.status)
                    && matches!(event.resource_type.as_str(), "Script" | "Fetch" | "XHR"))
        })
        .try_fold(crate::cache::CACHE_TTL, |ttl, event| {
            let headers = lowercase_headers(&event.response_headers);
            crate::fetch::response_cache_ttl(
                event.status,
                event.resource_type == "Document" && document_count > 1,
                false,
                headers.get("cache-control").map(String::as_str),
                headers.get("age").map(String::as_str),
                headers.get("expires").map(String::as_str),
                headers.get("vary").map(String::as_str),
            )
            .map(|event_ttl| ttl.min(event_ttl))
        })
}

fn lowercase_headers(
    headers: &std::collections::HashMap<String, String>,
) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
        .collect()
}

fn same_url(left: &str, right: &str) -> bool {
    match (url::Url::parse(left), url::Url::parse(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
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
