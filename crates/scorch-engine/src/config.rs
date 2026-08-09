use std::{path::PathBuf, time::Duration};

use metasearch::{EngineCredentials, EngineKind};

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub browser_path: PathBuf,
    pub max_concurrency: usize,
    pub max_response_bytes: usize,
    pub search_engines: Vec<EngineKind>,
    pub search_engine_credentials: EngineCredentials,
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub job_ttl: Duration,
    pub crawl_timeout: Duration,
    pub max_jobs: usize,
    pub max_active_crawls: usize,
    pub max_job_bytes: usize,
    pub max_crawl_limit: usize,
    pub max_crawl_depth: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            browser_path: "chromium".into(),
            max_concurrency: 4,
            max_response_bytes: 5 * 1024 * 1024,
            search_engines: EngineKind::ALL.to_vec(),
            search_engine_credentials: EngineCredentials::default(),
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(30),
            job_ttl: Duration::from_secs(15 * 60),
            crawl_timeout: Duration::from_secs(5 * 60),
            max_jobs: 128,
            max_active_crawls: 4,
            max_job_bytes: 32 * 1024 * 1024,
            max_crawl_limit: 100,
            max_crawl_depth: 5,
        }
    }
}
