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
    browser: BrowserManager,
    jobs: Arc<JobStore>,
}

impl ScorchEngine {
    pub async fn new(config: EngineConfig) -> Result<Arc<Self>> {
        let security = SecurityPolicy;
        let fetcher = SafeFetcher::new(security.clone(), config.clone());
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
        validate_scrape_request(request)?;
        let started = Instant::now();
        let timeout = Duration::from_millis(request.options.timeout_ms.min(120_000));
        let wants_screenshot = request.options.formats.contains(&ScrapeFormat::Screenshot);

        let fetched = self.fetcher.get(&request.url, timeout).await;
        let should_render = match (&fetched, request.options.render) {
            (_, RenderMode::Always) => true,
            (_, _) if wants_screenshot => true,
            (Ok(response), RenderMode::Auto) => needs_browser(response),
            _ => false,
        };

        if should_render {
            let rendered = self
                .browser
                .render(
                    &request.url,
                    timeout,
                    Duration::from_millis(request.options.wait_for_ms.min(60_000)),
                    request.options.block_media && !wants_screenshot,
                    wants_screenshot,
                    request.options.full_page_screenshot,
                )
                .await?;
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
                    engine: ScrapeEngine::Browser,
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
        let mut response = search::search(&self.fetcher, request).await?;
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
        map::map(&self.fetcher, request).await
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
