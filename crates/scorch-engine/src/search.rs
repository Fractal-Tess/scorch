use std::{sync::Arc, time::Duration};

use metasearch::{MetaSearch, MetaSearchConfig, MetaSearchOutput, SearchHit, SearchQuery};
use scorch_types::{SearchRequest, SearchResponse, SearchResult};
use tokio::sync::Semaphore;

use crate::{
    config::EngineConfig,
    error::{EngineError, Result},
};

pub struct SearchService {
    metasearch: MetaSearch,
    limit: Arc<Semaphore>,
    request_timeout: Duration,
}

impl SearchService {
    pub fn new(config: &EngineConfig) -> Result<Self> {
        let metasearch = MetaSearch::from_engine_kinds_with_credentials(
            MetaSearchConfig {
                per_engine_concurrency: config.max_concurrency,
                ..Default::default()
            },
            &config.search_engines,
            &config.search_engine_credentials,
        )
        .map_err(search_error)?;
        Ok(Self {
            metasearch,
            limit: Arc::new(Semaphore::new(config.max_concurrency)),
            request_timeout: config.request_timeout,
        })
    }

    pub async fn search(&self, request: &SearchRequest) -> Result<SearchResponse> {
        validate(request)?;
        let _permit = tokio::time::timeout(self.request_timeout, self.limit.acquire())
            .await
            .map_err(|_| EngineError::Timeout)?
            .map_err(|_| EngineError::Capacity("search runtime is shutting down".into()))?;
        let query = SearchQuery {
            query: request.query.trim().into(),
            limit: request.limit,
            country: request.country.clone(),
            language: request.language.clone(),
        };
        self.metasearch
            .search(&query)
            .await
            .map(|output| response(request, output))
            .map_err(search_error)
    }
}

fn validate(request: &SearchRequest) -> Result<()> {
    if request.query.trim().is_empty() {
        return Err(EngineError::InvalidRequest(
            "search query cannot be empty".into(),
        ));
    }
    if request.query.len() > 512 {
        return Err(EngineError::InvalidRequest(
            "search query cannot exceed 512 bytes".into(),
        ));
    }
    if !(1..=20).contains(&request.limit) {
        return Err(EngineError::InvalidRequest(
            "search limit must be between 1 and 20".into(),
        ));
    }
    for (name, value) in [
        ("country", request.country.as_str()),
        ("language", request.language.as_str()),
    ] {
        if !(2..=16).contains(&value.len())
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(EngineError::InvalidRequest(format!(
                "search {name} must be 2-16 ASCII letters, digits, or hyphens"
            )));
        }
    }
    Ok(())
}

fn response(request: &SearchRequest, output: MetaSearchOutput) -> SearchResponse {
    let warnings = output.engine_failures;
    SearchResponse {
        query: request.query.trim().into(),
        provider: "metasearch".into(),
        results: output
            .hits
            .into_iter()
            .enumerate()
            .map(|(index, hit)| {
                result(
                    index,
                    SearchHit {
                        title: hit.title,
                        url: hit.url,
                        snippet: hit.snippet,
                    },
                    hit.sources,
                )
            })
            .collect(),
        elapsed_ms: output.elapsed.as_millis() as u64,
        warnings,
    }
}

fn result(index: usize, hit: SearchHit, sources: Vec<String>) -> SearchResult {
    SearchResult {
        position: index + 1,
        title: hit.title,
        url: hit.url,
        description: hit.snippet,
        sources,
        document: None,
        error: None,
    }
}

fn search_error(error: metasearch::Error) -> EngineError {
    EngineError::Search(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_queries() {
        let request = SearchRequest {
            query: "  ".into(),
            limit: 5,
            scrape_options: None,
            country: "us".into(),
            language: "en".into(),
        };
        assert!(validate(&request).is_err());
    }

    #[test]
    fn rejects_invalid_locale_values() {
        let request = SearchRequest {
            query: "Rust".into(),
            limit: 5,
            scrape_options: None,
            country: "us&unsafe=true".into(),
            language: "en".into(),
        };
        assert!(validate(&request).is_err());
    }
}
