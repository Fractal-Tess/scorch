use std::time::Duration;

use metasearch::{EngineCredentials, EngineKind};

use crate::error::{EngineError, Result};

#[derive(Debug, Clone)]
pub struct EngineConfig {
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

/// Default number of pages rendered at once.
///
/// A render is dominated by the browser engine's own script and layout work,
/// which occupies one thread for the duration, so a fixed default left most of
/// a multi-core host idle: measured on a 16-thread host, raising this from 4 to
/// 16 more than doubled scrape throughput while idle memory was unchanged.
/// This is also the number of resident render slots, each holding a browser
/// context and its connection pool for the life of the process, so peak memory
/// scales with the value and it stays clamped rather than tracking very large
/// hosts.
pub fn default_max_concurrency() -> usize {
    std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(4)
        .clamp(4, 16)
}

impl EngineConfig {
    pub fn validate(&self) -> Result<()> {
        let positive_values = [
            ("max_concurrency", self.max_concurrency),
            ("max_response_bytes", self.max_response_bytes),
            ("max_jobs", self.max_jobs),
            ("max_active_crawls", self.max_active_crawls),
            ("max_job_bytes", self.max_job_bytes),
            ("max_crawl_limit", self.max_crawl_limit),
        ];
        if let Some((name, _)) = positive_values.into_iter().find(|(_, value)| *value == 0) {
            return Err(EngineError::InvalidRequest(format!(
                "{name} must be greater than zero"
            )));
        }
        if self.connect_timeout.is_zero()
            || self.request_timeout.is_zero()
            || self.job_ttl.is_zero()
            || self.crawl_timeout.is_zero()
        {
            return Err(EngineError::InvalidRequest(
                "engine timeouts and TTLs must be greater than zero".into(),
            ));
        }
        if self.search_engines.is_empty() {
            return Err(EngineError::InvalidRequest(
                "at least one search engine is required".into(),
            ));
        }
        Ok(())
    }
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            max_concurrency: default_max_concurrency(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_capacity() {
        let config = EngineConfig {
            max_concurrency: 0,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }
}
