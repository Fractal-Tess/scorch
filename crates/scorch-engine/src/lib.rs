mod browser;
mod cache;
mod config;
mod error;
mod extract;
mod fetch;
mod jobs;
mod map;
mod proxy;
mod search;
mod security;

use futures_util::{StreamExt, stream};
use scorch_types::{
    CrawlJob, CrawlPage, CrawlRequest, CrawlStatusRequest, MapRequest, MapResponse, ScrapeDocument,
    ScrapeEngine, ScrapeOptions, ScrapeRequest, SearchRequest, SearchResponse,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tracing::{debug, info, warn};

pub use config::{EngineConfig, default_max_concurrency};
pub use error::{EngineError, Result};

/// Obscura's Chrome-like transport is the only scrape transport policy.
pub const OBSCURA_STEALTH: bool = true;
use extract::ExtractInput;
use fetch::FetchResponse;
use jobs::JobStore;
use security::SecurityPolicy;

use crate::{
    browser::BrowserManager,
    cache::{CacheWarmer, ScrapeCache, ScrapeCacheKey},
    fetch::SafeFetcher,
};

pub struct ScorchEngine {
    config: EngineConfig,
    fetcher: SafeFetcher,
    search: search::SearchService,
    browser: BrowserManager,
    scrape_cache: Arc<ScrapeCache>,
    cache_warmer: CacheWarmer,
    jobs: Arc<JobStore>,
}

impl ScorchEngine {
    pub async fn new(config: EngineConfig) -> Result<Arc<Self>> {
        config.validate()?;
        let search_engines = config
            .search_engines
            .iter()
            .map(|engine| engine.as_str())
            .collect::<Vec<_>>()
            .join(",");
        info!(
            browser = "obscura",
            obscura_stealth = OBSCURA_STEALTH,
            max_concurrency = config.max_concurrency,
            max_response_bytes = config.max_response_bytes,
            search_provider = "metasearch",
            search_engines,
            max_crawl_limit = config.max_crawl_limit,
            job_ttl_seconds = config.job_ttl.as_secs(),
            "initializing Scorch engine"
        );
        let security = SecurityPolicy::new();
        let fetcher = SafeFetcher::new(security.clone(), config.clone());
        let search = search::SearchService::new(&config)?;
        let browser = BrowserManager::new(config.clone(), security).await?;
        let scrape_cache = Arc::new(ScrapeCache::default());
        let cache_warmer = cache::start_warmer(Arc::clone(&scrape_cache));
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
            scrape_cache,
            cache_warmer,
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
        let cache_key = ScrapeCacheKey::new(&request.url, &request.options);
        let cached = if request.options.store_in_cache {
            self.scrape_cache.get(
                &cache_key,
                &request.options.formats,
                Duration::from_millis(request.options.max_age_ms),
            )
        } else {
            None
        };
        if let Some(cached) = cached {
            debug!(
                operation = "scrape",
                origin = %log_origin(&request.url),
                cache_hit = true,
                "serving extracted scrape result from memory"
            );
            return Ok(cache::project_document(
                &cached,
                &request.url,
                &request.options.formats,
                started.elapsed().as_millis() as u64,
            ));
        }

        let observation = if request.options.store_in_cache {
            self.scrape_cache.begin_observation(&cache_key)
        } else {
            0
        };
        let timeout = Duration::from_millis(request.options.timeout_ms.min(120_000));
        let rendered = self
            .browser
            .render(
                &request.url,
                timeout,
                Duration::from_millis(request.options.wait_for_ms.min(60_000)),
                request.options.block_media,
            )
            .await?;
        self.extract_and_cache(
            rendered_extract_input(&request.url, rendered),
            request.options.clone(),
            started,
            cache_key,
            observation,
        )
        .await
    }

    async fn extract_and_cache(
        &self,
        input: ExtractInput,
        options: ScrapeOptions,
        started: Instant,
        cache_key: ScrapeCacheKey,
        observation: u64,
    ) -> Result<ScrapeDocument> {
        let cache_ttl = input.cache_ttl.filter(|_| options.store_in_cache);
        let cache_observed_at = input.cache_observed_at.filter(|_| cache_ttl.is_some());
        if options.store_in_cache {
            self.scrape_cache
                .invalidate_observation(&cache_key, observation);
        }
        let warm_input = cache_ttl.map(|_| input.clone());
        let formats = options.formats.clone();
        let document = extract_document(input, options.clone(), started).await?;
        let (Some(warm_input), Some(cache_ttl), Some(observed_at)) =
            (warm_input, cache_ttl, cache_observed_at)
        else {
            return Ok(document);
        };
        if !self.scrape_cache.insert(
            cache_key.clone(),
            observation,
            observed_at,
            cache_ttl,
            document.clone(),
            &formats,
        ) {
            return Ok(document);
        }
        if cache::needs_warm(&formats)
            && !self.cache_warmer.try_send(cache::warm_job(
                cache_key,
                observation,
                warm_input,
                options,
            ))
        {
            debug!(
                operation = "scrape",
                "background format cache budget is full; retaining requested formats only"
            );
        }
        Ok(document)
    }

    pub async fn search(&self, request: &SearchRequest) -> Result<SearchResponse> {
        let started = Instant::now();
        info!(
            operation = "search",
            provider = "metasearch",
            query_length = request.query.len(),
            limit = request.limit,
            category_count = request.categories.len(),
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
        let result = tokio::time::timeout(
            self.config.request_timeout,
            map::map(&self.fetcher, request),
        )
        .await
        .map_err(|_| EngineError::Timeout)
        .and_then(std::convert::identity);
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
        if request.cursor > job.documents.len() {
            return Err(EngineError::InvalidRequest(
                "cursor cannot exceed the document count".into(),
            ));
        }
        let documents = job.documents[request.cursor..end].to_vec();
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

fn rendered_extract_input(requested_url: &str, rendered: browser::RenderedPage) -> ExtractInput {
    let cache_ttl = rendered.cache_ttl;
    let cache_observed_at = rendered.cache_observed_at;
    let response = FetchResponse {
        final_url: rendered.final_url,
        status: reqwest::StatusCode::from_u16(rendered.status).unwrap_or(reqwest::StatusCode::OK),
        content_type: rendered.content_type,
        headers: rendered.headers,
        body: Vec::new(),
    };
    ExtractInput {
        requested_url: requested_url.to_owned(),
        html: rendered.html.into(),
        response,
        engine: ScrapeEngine::Obscura,
        cache_ttl,
        cache_observed_at,
    }
}

async fn extract_document(
    input: ExtractInput,
    options: ScrapeOptions,
    request_started: Instant,
) -> Result<ScrapeDocument> {
    let extraction_started = Instant::now();
    let mut document = tokio::task::spawn_blocking(move || extract::extract(input, &options))
        .await
        .map_err(|error| EngineError::Extraction(format!("extraction worker failed: {error}")))??;
    let extraction_ms = extraction_started.elapsed().as_millis() as u64;
    document.elapsed_ms = request_started.elapsed().as_millis() as u64;
    debug!(
        operation = "scrape",
        extraction_ms, "content extraction completed"
    );
    Ok(document)
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
    let unique_formats = request
        .options
        .formats
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    if unique_formats.len() != request.options.formats.len() {
        return Err(EngineError::InvalidRequest(
            "scrape formats cannot contain duplicates".into(),
        ));
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

#[cfg(test)]
mod tests {
    use super::{log_origin, validate_scrape_request};
    use scorch_types::{ScrapeFormat, ScrapeOptions, ScrapeRequest};

    #[test]
    fn rejects_inconsistent_scrape_options() {
        let duplicate_formats = ScrapeRequest {
            url: "https://example.com".into(),
            options: ScrapeOptions {
                formats: vec![ScrapeFormat::Markdown, ScrapeFormat::Markdown],
                ..Default::default()
            },
        };
        assert!(validate_scrape_request(&duplicate_formats).is_err());
    }

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
