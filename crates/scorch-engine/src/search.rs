use std::{sync::Arc, time::Duration};

use metasearch::{MetaSearch, MetaSearchConfig, MetaSearchOutput, SearchHit, SearchQuery};
use scorch_types::{
    SearchEngine as RequestedSearchEngine, SearchRequest, SearchResponse, SearchResult,
};
use tokio::sync::Semaphore;

use crate::{
    config::EngineConfig,
    error::{EngineError, Result},
};

pub struct SearchService {
    metasearch: MetaSearch,
    allowed_engines: Vec<metasearch::EngineKind>,
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
            allowed_engines: config.search_engines.clone(),
            limit: Arc::new(Semaphore::new(config.max_concurrency)),
            request_timeout: config.request_timeout,
        })
    }

    pub async fn search(&self, request: &SearchRequest) -> Result<SearchResponse> {
        validate(request)?;
        let engines = self.selected_engines(request)?;
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
            .search_with_engine_kinds(&query, &engines)
            .await
            .map(|output| response(request, output))
            .map_err(search_error)
    }

    fn selected_engines(&self, request: &SearchRequest) -> Result<Vec<metasearch::EngineKind>> {
        let requested = if request.engines.is_empty() {
            [
                metasearch::EngineKind::Bing,
                metasearch::EngineKind::DuckDuckGo,
            ]
            .into_iter()
            .filter(|engine| self.allowed_engines.contains(engine))
            .collect::<Vec<_>>()
        } else {
            request
                .engines
                .iter()
                .copied()
                .map(engine_kind)
                .collect::<Vec<_>>()
        };
        if requested.is_empty() {
            return Err(EngineError::InvalidRequest(
                "no default search engines are allowed; select an allowed engine explicitly".into(),
            ));
        }
        let mut selected = Vec::new();
        for engine in requested {
            if !self.allowed_engines.contains(&engine) {
                return Err(EngineError::InvalidRequest(format!(
                    "search engine {} is not allowed by server policy",
                    engine.as_str()
                )));
            }
            if !selected.contains(&engine) {
                selected.push(engine);
            }
        }
        Ok(selected)
    }
}

fn engine_kind(engine: RequestedSearchEngine) -> metasearch::EngineKind {
    match engine {
        RequestedSearchEngine::Bing => metasearch::EngineKind::Bing,
        RequestedSearchEngine::Brave => metasearch::EngineKind::Brave,
        RequestedSearchEngine::DuckDuckGo => metasearch::EngineKind::DuckDuckGo,
        RequestedSearchEngine::Google => metasearch::EngineKind::Google,
        RequestedSearchEngine::Naver => metasearch::EngineKind::Naver,
        RequestedSearchEngine::Wikipedia => metasearch::EngineKind::Wikipedia,
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
    let MetaSearchOutput {
        hits,
        engines_used,
        engine_failures,
        elapsed,
        ..
    } = output;
    SearchResponse {
        query: request.query.trim().into(),
        provider: "metasearch".into(),
        engines: engines_used,
        results: hits
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
        elapsed_ms: elapsed.as_millis() as u64,
        warnings: engine_failures,
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
            engines: Vec::new(),
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
            engines: Vec::new(),
        };
        assert!(validate(&request).is_err());
    }

    #[test]
    fn uses_defaults_and_allows_explicit_policy_subset() {
        let service = SearchService::new(&EngineConfig::default()).unwrap();
        let mut request = SearchRequest {
            query: "Rust".into(),
            limit: 5,
            scrape_options: None,
            country: "us".into(),
            language: "en".into(),
            engines: Vec::new(),
        };
        assert_eq!(
            service.selected_engines(&request).unwrap(),
            [
                metasearch::EngineKind::Bing,
                metasearch::EngineKind::DuckDuckGo
            ]
        );

        request.engines = vec![
            RequestedSearchEngine::Wikipedia,
            RequestedSearchEngine::Wikipedia,
        ];
        assert_eq!(
            service.selected_engines(&request).unwrap(),
            [metasearch::EngineKind::Wikipedia]
        );
    }

    #[test]
    fn rejects_engines_outside_server_policy() {
        let service = SearchService::new(&EngineConfig::default()).unwrap();
        let request = SearchRequest {
            query: "Rust".into(),
            limit: 5,
            scrape_options: None,
            country: "us".into(),
            language: "en".into(),
            engines: vec![RequestedSearchEngine::Brave],
        };
        let error = service.selected_engines(&request).unwrap_err();
        assert!(error.to_string().contains("not allowed by server policy"));
    }

    #[test]
    fn explicit_only_policy_requires_request_selection() {
        let config = EngineConfig {
            search_engines: vec![metasearch::EngineKind::Wikipedia],
            ..Default::default()
        };
        let service = SearchService::new(&config).unwrap();
        let mut request = SearchRequest {
            query: "Rust".into(),
            limit: 5,
            scrape_options: None,
            country: "us".into(),
            language: "en".into(),
            engines: Vec::new(),
        };
        assert!(service.selected_engines(&request).is_err());

        request.engines = vec![RequestedSearchEngine::Wikipedia];
        assert_eq!(
            service.selected_engines(&request).unwrap(),
            [metasearch::EngineKind::Wikipedia]
        );
    }
}
