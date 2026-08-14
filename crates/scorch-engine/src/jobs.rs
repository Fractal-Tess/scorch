use std::{
    collections::{HashSet, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use dashmap::DashMap;
use futures_util::{FutureExt, StreamExt, stream::FuturesUnordered};
use robotstxt::DefaultMatcher;
use scorch_types::{
    CrawlError, CrawlJob, CrawlRequest, CrawlStatus, MapRequest, ScrapeFormat, ScrapeRequest,
};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use url::Url;
use uuid::Uuid;

use crate::{
    ScorchEngine,
    error::{EngineError, Result},
    log_origin,
    map::path_allowed,
};

struct JobEntry {
    job: CrawlJob,
    cancel: CancellationToken,
    retained_bytes: usize,
}

pub struct JobStore {
    jobs: DashMap<Uuid, JobEntry>,
    ttl: Duration,
    crawl_timeout: Duration,
    max_jobs: usize,
    max_job_bytes: usize,
    active: Arc<Semaphore>,
    admission: Mutex<()>,
}

impl JobStore {
    pub fn new(
        ttl: Duration,
        crawl_timeout: Duration,
        max_jobs: usize,
        max_active_crawls: usize,
        max_job_bytes: usize,
    ) -> Self {
        Self {
            jobs: DashMap::new(),
            ttl,
            crawl_timeout,
            max_jobs,
            max_job_bytes,
            active: Arc::new(Semaphore::new(max_active_crawls)),
            admission: Mutex::new(()),
        }
    }

    pub fn start(
        self: &Arc<Self>,
        engine: Arc<ScorchEngine>,
        request: CrawlRequest,
    ) -> Result<CrawlJob> {
        let root = Url::parse(&request.url)
            .map_err(|error| EngineError::InvalidRequest(format!("invalid crawl URL: {error}")))?;
        if !matches!(root.scheme(), "http" | "https") || root.host_str().is_none() {
            return Err(EngineError::InvalidRequest(
                "crawl URL must use HTTP or HTTPS and include a host".into(),
            ));
        }
        if !root.username().is_empty() || root.password().is_some() {
            return Err(EngineError::InvalidRequest(
                "credentials in crawl URLs are not allowed".into(),
            ));
        }
        if !(1..=engine.config().max_crawl_limit).contains(&request.limit) {
            return Err(EngineError::InvalidRequest(format!(
                "crawl limit must be between 1 and {}",
                engine.config().max_crawl_limit
            )));
        }
        if request.max_depth > engine.config().max_crawl_depth {
            return Err(EngineError::InvalidRequest(format!(
                "crawl depth cannot exceed {}",
                engine.config().max_crawl_depth
            )));
        }
        if request.concurrency == 0 || request.concurrency > engine.config().max_concurrency {
            return Err(EngineError::InvalidRequest(format!(
                "crawl concurrency must be between 1 and {}",
                engine.config().max_concurrency
            )));
        }

        let _admission = self
            .admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.cleanup();
        if self.jobs.len() >= self.max_jobs {
            return Err(EngineError::Capacity(format!(
                "at most {} crawl jobs may be retained",
                self.max_jobs
            )));
        }
        let id = Uuid::now_v7();
        let created_at_ms = now_ms();
        let expires_at_ms = created_at_ms.saturating_add(self.ttl.as_millis() as u64);
        let job = CrawlJob {
            id,
            status: CrawlStatus::Queued,
            created_at_ms,
            expires_at_ms,
            total: 1,
            completed: 0,
            documents: Vec::new(),
            errors: Vec::new(),
        };
        let cancel = CancellationToken::new();
        self.jobs.insert(
            id,
            JobEntry {
                job: job.clone(),
                cancel: cancel.clone(),
                retained_bytes: 0,
            },
        );
        info!(
            operation = "crawl",
            crawl_id = %id,
            origin = %log_origin(&request.url),
            limit = request.limit,
            max_depth = request.max_depth,
            concurrency = request.concurrency,
            "crawl queued"
        );
        let store = Arc::clone(self);
        tokio::spawn(async move {
            store.run(engine, id, request, cancel).await;
        });
        Ok(job)
    }

    pub fn get(&self, id: Uuid) -> Result<CrawlJob> {
        self.cleanup();
        self.jobs
            .get(&id)
            .map(|entry| entry.job.clone())
            .ok_or(EngineError::JobNotFound)
    }

    pub fn cancel_and_delete(&self, id: Uuid) -> bool {
        self.jobs.remove(&id).is_some_and(|(_, entry)| {
            entry.cancel.cancel();
            info!(operation = "crawl", crawl_id = %id, "crawl cancelled and removed");
            true
        })
    }

    async fn run(
        &self,
        engine: Arc<ScorchEngine>,
        id: Uuid,
        request: CrawlRequest,
        cancel: CancellationToken,
    ) {
        let failed_url = request.url.clone();
        if tokio::time::timeout(
            self.crawl_timeout,
            self.run_inner(engine, id, request, cancel.clone()),
        )
        .await
        .is_err()
        {
            cancel.cancel();
            self.fail(id, &failed_url, "crawl exceeded its absolute deadline");
        }
    }

    async fn run_inner(
        &self,
        engine: Arc<ScorchEngine>,
        id: Uuid,
        request: CrawlRequest,
        cancel: CancellationToken,
    ) {
        let _active_permit = tokio::select! {
            biased;
            () = cancel.cancelled() => {
                self.finish(id, CrawlStatus::Cancelled);
                return;
            }
            permit = Arc::clone(&self.active).acquire_owned() => {
                let Ok(permit) = permit else {
                    self.fail(id, &request.url, "crawl runtime is shutting down");
                    return;
                };
                permit
            }
        };
        if cancel.is_cancelled() {
            self.finish(id, CrawlStatus::Cancelled);
            info!(operation = "crawl", crawl_id = %id, "crawl cancelled before start");
            return;
        }
        self.mutate(id, |job| job.status = CrawlStatus::Running);
        let started = std::time::Instant::now();
        info!(operation = "crawl", crawl_id = %id, "crawl started");
        let Ok(root) = Url::parse(&request.url) else {
            self.fail(id, &request.url, "invalid crawl URL");
            return;
        };
        let robots = fetch_robots(&engine, &root).await;
        let mut queue = VecDeque::from([(root.to_string(), 0_usize)]);
        let mut seen = HashSet::from([normalized(&root)]);

        let map_request = MapRequest {
            url: root.to_string(),
            limit: request.limit,
            include_subdomains: false,
            include_paths: request.include_paths.clone(),
            exclude_paths: request.exclude_paths.clone(),
        };
        if let Ok(mapped) = engine.map(&map_request).await {
            for url in mapped.links {
                let Ok(candidate) = Url::parse(&url) else {
                    continue;
                };
                if same_origin(&root, &candidate) && seen.insert(normalized(&candidate)) {
                    queue.push_back((candidate.to_string(), 1));
                }
            }
        }

        while !queue.is_empty() && self.completed(id) < request.limit {
            if started.elapsed() >= self.crawl_timeout {
                self.fail(id, &request.url, "crawl exceeded its absolute deadline");
                return;
            }
            if cancel.is_cancelled() {
                self.finish(id, CrawlStatus::Cancelled);
                info!(
                    operation = "crawl",
                    crawl_id = %id,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "crawl cancelled"
                );
                return;
            }
            let remaining = request.limit.saturating_sub(self.completed(id));
            let batch_size = request.concurrency.min(remaining).min(queue.len());
            let mut batch = FuturesUnordered::new();
            for _ in 0..batch_size {
                let Some((url, depth)) = queue.pop_front() else {
                    break;
                };
                if !robots_allowed(robots.as_deref(), &url) {
                    self.record_error(id, url, "disallowed by robots.txt".into());
                    continue;
                }
                let mut options = request.scrape_options.clone();
                if depth < request.max_depth && !options.formats.contains(&ScrapeFormat::Links) {
                    options.formats.push(ScrapeFormat::Links);
                }
                let engine = Arc::clone(&engine);
                let task_cancel = cancel.clone();
                batch.push(
                    async move {
                        let scrape_request = ScrapeRequest {
                            url: url.clone(),
                            options,
                        };
                        let result = tokio::select! {
                            biased;
                            () = task_cancel.cancelled() => None,
                            result = engine.scrape(&scrape_request) => Some(result),
                        };
                        (url, depth, result)
                    }
                    .boxed(),
                );
            }
            self.mutate(id, |job| {
                let discovered = (job.completed + batch.len() + queue.len()).min(request.limit);
                job.total = job.total.max(discovered);
            });

            while let Some((url, depth, result)) = batch.next().await {
                let Some(result) = result else {
                    continue;
                };
                match result {
                    Ok(document) => {
                        if depth < request.max_depth
                            && let Some(links) = &document.links
                        {
                            for link in links {
                                if seen.len() >= request.limit.saturating_mul(10) {
                                    break;
                                }
                                let Ok(candidate) = Url::parse(&link.url) else {
                                    continue;
                                };
                                if same_origin(&root, &candidate)
                                    && path_allowed(
                                        candidate.path(),
                                        &request.include_paths,
                                        &request.exclude_paths,
                                    )
                                    && seen.insert(normalized(&candidate))
                                {
                                    queue.push_back((candidate.to_string(), depth + 1));
                                }
                            }
                        }
                        if !self.store_document(id, document) {
                            self.record_error(
                                id,
                                url,
                                format!(
                                    "crawl result exceeded the {} byte retained-data limit",
                                    self.max_job_bytes
                                ),
                            );
                        }
                    }
                    Err(error) => self.record_error(id, url, error.to_string()),
                }
            }
        }

        self.mutate(id, |job| job.total = job.completed);
        self.finish(
            id,
            if cancel.is_cancelled() {
                CrawlStatus::Cancelled
            } else {
                CrawlStatus::Completed
            },
        );
        if let Some(entry) = self.jobs.get(&id) {
            info!(
                operation = "crawl",
                crawl_id = %id,
                status = ?entry.job.status,
                completed = entry.job.completed,
                error_count = entry.job.errors.len(),
                retained_bytes = entry.retained_bytes,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "crawl finished"
            );
        }
    }

    fn completed(&self, id: Uuid) -> usize {
        self.jobs.get(&id).map_or(0, |entry| entry.job.completed)
    }

    fn store_document(&self, id: Uuid, document: scorch_types::ScrapeDocument) -> bool {
        let size = document_size(&document);
        let Some(mut entry) = self.jobs.get_mut(&id) else {
            return false;
        };
        if entry.retained_bytes.saturating_add(size) > self.max_job_bytes {
            return false;
        }
        entry.retained_bytes += size;
        entry.job.completed += 1;
        entry.job.documents.push(document);
        true
    }

    fn record_error(&self, id: Uuid, url: String, message: String) {
        self.mutate(id, |job| {
            job.completed += 1;
            job.errors.push(CrawlError { url, message });
        });
    }

    fn fail(&self, id: Uuid, url: &str, message: &str) {
        warn!(
            operation = "crawl",
            crawl_id = %id,
            origin = %log_origin(url),
            reason = message,
            "crawl failed"
        );
        self.mutate(id, |job| {
            job.errors.push(CrawlError {
                url: url.into(),
                message: message.into(),
            });
        });
        self.finish(id, CrawlStatus::Failed);
    }

    fn finish(&self, id: Uuid, status: CrawlStatus) {
        let expires_at_ms = now_ms().saturating_add(self.ttl.as_millis() as u64);
        self.mutate(id, |job| {
            job.status = status;
            job.expires_at_ms = expires_at_ms;
        });
    }

    fn mutate(&self, id: Uuid, update: impl FnOnce(&mut CrawlJob)) {
        if let Some(mut entry) = self.jobs.get_mut(&id) {
            update(&mut entry.job);
        }
    }

    fn cleanup(&self) {
        let now = now_ms();
        self.jobs.retain(|_, entry| {
            let retained = entry.job.expires_at_ms > now;
            if !retained {
                entry.cancel.cancel();
            }
            retained
        });
    }
}

async fn fetch_robots(engine: &ScorchEngine, root: &Url) -> Option<String> {
    let url = root.join("/robots.txt").ok()?;
    let response = engine
        .fetcher()
        .get(url.as_str(), Duration::from_secs(10))
        .await
        .ok()?;
    response
        .status
        .is_success()
        .then(|| String::from_utf8_lossy(&response.body).into_owned())
}

fn robots_allowed(robots: Option<&str>, url: &str) -> bool {
    robots.is_none_or(|robots| {
        DefaultMatcher::default().one_agent_allowed_by_robots(robots, "ScorchBot", url)
    })
}

fn same_origin(root: &Url, candidate: &Url) -> bool {
    root.scheme() == candidate.scheme()
        && root.host_str() == candidate.host_str()
        && root.port_or_known_default() == candidate.port_or_known_default()
}

fn normalized(url: &Url) -> String {
    let mut url = url.clone();
    url.set_fragment(None);
    url.to_string()
}

pub(crate) fn document_size(document: &scorch_types::ScrapeDocument) -> usize {
    document.markdown.as_ref().map_or(0, String::len)
        + document.html.as_ref().map_or(0, String::len)
        + document.text.as_ref().map_or(0, String::len)
        + document.links.as_ref().map_or(0, |links| {
            links
                .iter()
                .map(|link| link.url.len() + link.text.as_ref().map_or(0, String::len))
                .sum()
        })
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
