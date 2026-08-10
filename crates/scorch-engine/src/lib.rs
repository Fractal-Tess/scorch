mod browser;
mod config;
mod error;
mod extract;
mod fetch;
mod jobs;
mod map;
mod proxy;
mod search;
mod security;

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use futures_util::{StreamExt, stream};
use scorch_types::{
    CrawlJob, CrawlPage, CrawlRequest, CrawlStatusRequest, MapRequest, MapResponse, RenderMode,
    ScrapeDocument, ScrapeEngine, ScrapeFormat, ScrapeRequest, SearchRequest, SearchResponse,
};
use scraper::{Html, Selector};
use tracing::{debug, info, warn};

pub use config::EngineConfig;
pub use error::{EngineError, Result};
use extract::ExtractInput;
use fetch::FetchResponse;
use jobs::JobStore;
use security::SecurityPolicy;

use crate::{browser::BrowserManager, fetch::SafeFetcher};

pub struct ScorchEngine {
    config: EngineConfig,
    fetcher: SafeFetcher,
    search: search::SearchService,
    browser: BrowserManager,
    jobs: Arc<JobStore>,
}

impl ScorchEngine {
    pub async fn new(config: EngineConfig) -> Result<Arc<Self>> {
        let search_engines = config
            .search_engines
            .iter()
            .map(|engine| engine.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let allowed_browsers = config
            .allowed_browsers
            .iter()
            .map(|browser| browser.as_str())
            .collect::<Vec<_>>()
            .join(",");
        info!(
            browser = config.browser.as_str(),
            allowed_browsers,
            chromium_path = %config.browser_path.display(),
            obscura_stealth = config.obscura_stealth,
            max_concurrency = config.max_concurrency,
            max_response_bytes = config.max_response_bytes,
            search_provider = "metasearch",
            search_engines,
            max_crawl_limit = config.max_crawl_limit,
            job_ttl_seconds = config.job_ttl.as_secs(),
            "initializing Scorch engine"
        );
        let security = SecurityPolicy;
        let fetcher = SafeFetcher::new(security.clone(), config.clone());
        let search = search::SearchService::new(&config)?;
        let browser = BrowserManager::new(config.clone(), security).await?;
        let jobs = Arc::new(JobStore::new(
            config.job_ttl,
            config.crawl_timeout,
            config.max_jobs,
            config.max_active_crawls,
            config.max_job_bytes,
        ));
        Ok(Arc::new(Self {
            config,
            fetcher,
            search,
            browser,
            jobs,
        }))
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    pub(crate) fn fetcher(&self) -> &SafeFetcher {
        &self.fetcher
    }

    pub fn browser_available(&self) -> bool {
        self.browser.available()
    }

    pub async fn scrape(&self, request: &ScrapeRequest) -> Result<ScrapeDocument> {
        let started = Instant::now();
        let origin = log_origin(&request.url);
        info!(
            operation = "scrape",
            %origin,
            render = ?request.options.render,
            format_count = request.options.formats.len(),
            "scrape started"
        );
        let result = self.scrape_inner(request, started).await;
        match &result {
            Ok(document) => info!(
                operation = "scrape",
                %origin,
                engine = ?document.engine,
                status_code = document.metadata.status_code,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "scrape completed"
            ),
            Err(error) => {
                warn!(
                    operation = "scrape",
                    %origin,
                    error_code = error.code(),
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "scrape failed"
                );
                debug!(operation = "scrape", %origin, %error, "scrape failure details");
            }
        }
        result
    }

    async fn scrape_inner(
        &self,
        request: &ScrapeRequest,
        started: Instant,
    ) -> Result<ScrapeDocument> {
        validate_scrape_request(request)?;
        let timeout = Duration::from_millis(request.options.timeout_ms.min(120_000));
        let wants_screenshot = request.options.formats.contains(&ScrapeFormat::Screenshot);
        let forced_browser = (request.options.render == RenderMode::Always || wants_screenshot)
            .then(|| self.browser.resolve(request.options.browser))
            .transpose()?;
        let render = |browser| {
            self.browser.render(
                browser,
                &request.url,
                timeout,
                Duration::from_millis(request.options.wait_for_ms.min(60_000)),
                request.options.block_media && !wants_screenshot,
                wants_screenshot,
                request.options.full_page_screenshot,
            )
        };

        let (fetched, forced_rendered) = if let Some(browser) = forced_browser {
            let (fetched, rendered) =
                tokio::join!(self.fetcher.get(&request.url, timeout), render(browser));
            (fetched, Some((browser, rendered?)))
        } else {
            (self.fetcher.get(&request.url, timeout).await, None)
        };
        let should_render = forced_rendered.is_some()
            || matches!((&fetched, request.options.render), (Ok(response), RenderMode::Auto) if needs_browser(response));

        if should_render {
            let (browser, rendered) = match forced_rendered {
                Some(rendered) => rendered,
                None => {
                    let browser = self.browser.resolve(request.options.browser)?;
                    (browser, render(browser).await?)
                }
            };
            let response = match fetched {
                Ok(mut response) => {
                    response.final_url = rendered.final_url.clone();
                    response
                }
                Err(_) => FetchResponse {
                    final_url: rendered.final_url.clone(),
                    status: reqwest::StatusCode::OK,
                    content_type: Some("text/html; charset=utf-8".into()),
                    headers: Default::default(),
                    body: Vec::new(),
                },
            };
            return extract::extract(
                ExtractInput {
                    requested_url: &request.url,
                    html: rendered.html,
                    response: &response,
                    engine: browser.into(),
                    elapsed_ms: started.elapsed().as_millis() as u64,
                    screenshot: rendered.screenshot,
                },
                &request.options,
            );
        }

        let response = fetched?;
        ensure_supported_content(&response)?;
        let html = String::from_utf8_lossy(&response.body).into_owned();
        extract::extract(
            ExtractInput {
                requested_url: &request.url,
                html,
                response: &response,
                engine: ScrapeEngine::Fetch,
                elapsed_ms: started.elapsed().as_millis() as u64,
                screenshot: None,
            },
            &request.options,
        )
    }

    pub async fn search(&self, request: &SearchRequest) -> Result<SearchResponse> {
        let started = Instant::now();
        info!(
            operation = "search",
            provider = "metasearch",
            query_length = request.query.len(),
            limit = request.limit,
            enrich_results = request.scrape_options.is_some(),
            "search started"
        );
        let result = self.search_inner(request).await;
        match &result {
            Ok(response) => info!(
                operation = "search",
                provider = %response.provider,
                result_count = response.results.len(),
                elapsed_ms = started.elapsed().as_millis() as u64,
                "search completed"
            ),
            Err(error) => {
                warn!(
                    operation = "search",
                    error_code = error.code(),
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "search failed"
                );
                debug!(operation = "search", %error, "search failure details");
            }
        }
        result
    }

    async fn search_inner(&self, request: &SearchRequest) -> Result<SearchResponse> {
        let mut response = self.search.search(request).await?;
        let Some(options) = request.scrape_options.clone() else {
            return Ok(response);
        };
        let requests: Vec<_> = response
            .results
            .iter()
            .enumerate()
            .map(|(index, result)| {
                (
                    index,
                    ScrapeRequest {
                        url: result.url.clone(),
                        options: options.clone(),
                    },
                )
            })
            .collect();
        let mut scrape_tasks = stream::iter(requests)
            .map(|(index, request)| async move { (index, self.scrape(&request).await) })
            .buffer_unordered(self.config.max_concurrency);
        let mut aggregate_bytes = 0_usize;
        while let Some((index, result)) = scrape_tasks.next().await {
            match result {
                Ok(document) => {
                    let size = jobs::document_size(&document);
                    if aggregate_bytes.saturating_add(size) > self.config.max_job_bytes {
                        response.results[index].error = Some(format!(
                            "search enrichment exceeded the {} byte aggregate limit",
                            self.config.max_job_bytes
                        ));
                    } else {
                        aggregate_bytes += size;
                        response.results[index].document = Some(document);
                    }
                }
                Err(error) => response.results[index].error = Some(error.to_string()),
            }
        }
        Ok(response)
    }

    pub async fn map(&self, request: &MapRequest) -> Result<MapResponse> {
        let started = Instant::now();
        let origin = log_origin(&request.url);
        info!(
            operation = "map",
            %origin,
            limit = request.limit,
            include_subdomains = request.include_subdomains,
            "map started"
        );
        let result = map::map(&self.fetcher, request).await;
        match &result {
            Ok(response) => info!(
                operation = "map",
                %origin,
                link_count = response.links.len(),
                source_count = response.sources.len(),
                elapsed_ms = started.elapsed().as_millis() as u64,
                "map completed"
            ),
            Err(error) => {
                warn!(
                    operation = "map",
                    %origin,
                    error_code = error.code(),
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "map failed"
                );
                debug!(operation = "map", %origin, %error, "map failure details");
            }
        }
        result
    }

    pub fn start_crawl(self: &Arc<Self>, request: CrawlRequest) -> Result<CrawlJob> {
        self.jobs.start(Arc::clone(self), request)
    }

    pub fn crawl_status(&self, id: uuid::Uuid) -> Result<CrawlJob> {
        self.jobs.get(id)
    }

    pub fn crawl_page(&self, request: &CrawlStatusRequest) -> Result<CrawlPage> {
        if request.page_size == 0 || request.page_size > 50 {
            return Err(EngineError::InvalidRequest(
                "pageSize must be between 1 and 50".into(),
            ));
        }
        let job = self.jobs.get(request.id)?;
        let end = request
            .cursor
            .saturating_add(request.page_size)
            .min(job.documents.len());
        let documents = job
            .documents
            .get(request.cursor..end)
            .unwrap_or_default()
            .to_vec();
        let next_cursor = (end < job.documents.len()).then_some(end);
        Ok(CrawlPage {
            summary: (&job).into(),
            cursor: request.cursor,
            documents,
            errors: job.errors.clone(),
            next_cursor,
        })
    }

    pub fn delete_crawl(&self, id: uuid::Uuid) -> bool {
        self.jobs.cancel_and_delete(id)
    }
}

pub(crate) fn log_origin(input: &str) -> String {
    let Ok(url) = url::Url::parse(input) else {
        return "<invalid-url>".into();
    };
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return "<invalid-url>".into();
    }
    url.origin().ascii_serialization()
}

fn validate_scrape_request(request: &ScrapeRequest) -> Result<()> {
    if request.options.formats.is_empty() {
        return Err(EngineError::InvalidRequest(
            "at least one format is required".into(),
        ));
    }
    if request.options.formats.len() > 6 {
        return Err(EngineError::InvalidRequest("too many formats".into()));
    }
    if request.options.timeout_ms < 100 || request.options.timeout_ms > 120_000 {
        return Err(EngineError::InvalidRequest(
            "timeoutMs must be between 100 and 120000".into(),
        ));
    }
    if request.options.wait_for_ms > 60_000 {
        return Err(EngineError::InvalidRequest(
            "waitForMs cannot exceed 60000".into(),
        ));
    }
    Ok(())
}

fn needs_browser(response: &FetchResponse) -> bool {
    let content_type = response
        .content_type
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !content_type.is_empty() && !content_type.contains("html") {
        return false;
    }
    let html = String::from_utf8_lossy(&response.body);
    let document = Html::parse_document(&html);
    let body_text = Selector::parse("body")
        .ok()
        .and_then(|selector| {
            document
                .select(&selector)
                .next()
                .map(|body| body.text().collect::<Vec<_>>().join(" "))
        })
        .unwrap_or_default();
    let script_count =
        Selector::parse("script").map_or(0, |selector| document.select(&selector).count());
    let lower = body_text.to_ascii_lowercase();
    let source = html.to_ascii_lowercase();
    (body_text.split_whitespace().count() < 40 && script_count >= 2)
        || lower.contains("enable javascript")
        || lower.contains("javascript is required")
        || source.contains("document.write(")
        || source.contains("__next_data__")
        || source.contains("data-reactroot")
        || source.contains("ng-app")
        || source.contains("id=\"root\"></div>")
}

fn ensure_supported_content(response: &FetchResponse) -> Result<()> {
    let Some(content_type) = response.content_type.as_deref() else {
        return Ok(());
    };
    let content_type = content_type.to_ascii_lowercase();
    if content_type.contains("html")
        || content_type.starts_with("text/")
        || content_type.contains("xml")
    {
        Ok(())
    } else {
        Err(EngineError::UnsupportedContent(content_type))
    }
}

#[cfg(test)]
mod tests {
    use super::log_origin;

    #[test]
    fn log_origin_removes_sensitive_url_components() {
        assert_eq!(
            log_origin("https://user:secret@example.com:8443/private/token?api_key=secret#value"),
            "https://example.com:8443"
        );
        assert_eq!(
            log_origin("https://[2001:db8::1]/path"),
            "https://[2001:db8::1]"
        );
        assert_eq!(log_origin("not a URL"), "<invalid-url>");
        assert_eq!(log_origin("file:///tmp/secret"), "<invalid-url>");
    }
}
