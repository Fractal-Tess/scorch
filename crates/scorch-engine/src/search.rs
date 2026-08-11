use std::{sync::Arc, time::Duration};

use metasearch::{MetaSearch, MetaSearchConfig, MetaSearchOutput, SearchHit, SearchQuery};
use scorch_types::{
    SearchCategory, SearchEngine as RequestedSearchEngine, SearchRequest, SearchResponse,
    SearchResult,
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
            query: categorized_query(request),
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
            [metasearch::EngineKind::DuckDuckGo]
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
        RequestedSearchEngine::BraveWeb => metasearch::EngineKind::BraveWeb,
        RequestedSearchEngine::CratesIo => metasearch::EngineKind::CratesIo,
        RequestedSearchEngine::Crossref => metasearch::EngineKind::Crossref,
        RequestedSearchEngine::DockerHub => metasearch::EngineKind::DockerHub,
        RequestedSearchEngine::DuckDuckGo => metasearch::EngineKind::DuckDuckGo,
        RequestedSearchEngine::GitHub => metasearch::EngineKind::GitHub,
        RequestedSearchEngine::Google => metasearch::EngineKind::Google,
        RequestedSearchEngine::GoogleCse => metasearch::EngineKind::GoogleCse,
        RequestedSearchEngine::HackerNews => metasearch::EngineKind::HackerNews,
        RequestedSearchEngine::HuggingFace => metasearch::EngineKind::HuggingFace,
        RequestedSearchEngine::Mwmbl => metasearch::EngineKind::Mwmbl,
        RequestedSearchEngine::Npm => metasearch::EngineKind::Npm,
        RequestedSearchEngine::Nvd => metasearch::EngineKind::Nvd,
        RequestedSearchEngine::Openalex => metasearch::EngineKind::OpenAlex,
        RequestedSearchEngine::OpenLibrary => metasearch::EngineKind::OpenLibrary,
        RequestedSearchEngine::Pubmed => metasearch::EngineKind::PubMed,
        RequestedSearchEngine::Wikidata => metasearch::EngineKind::Wikidata,
        RequestedSearchEngine::Wikipedia => metasearch::EngineKind::Wikipedia,
        RequestedSearchEngine::Yahoo => metasearch::EngineKind::Yahoo,
    }
}

fn validate(request: &SearchRequest) -> Result<()> {
    if request.query.trim().is_empty() {
        return Err(EngineError::InvalidRequest(
            "search query cannot be empty".into(),
        ));
    }
    if categorized_query(request).len() > 512 {
        return Err(EngineError::InvalidRequest(
            "search query cannot exceed 512 bytes after category filters".into(),
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

fn categorized_query(request: &SearchRequest) -> String {
    let mut query = request.query.trim().to_owned();
    if request.categories.contains(&SearchCategory::GitHub) {
        query.push_str(" (site:github.com)");
    }
    query
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
                let category = category_for_url(&hit.url, &request.categories);
                result(
                    index,
                    SearchHit {
                        title: hit.title,
                        url: hit.url,
                        snippet: hit.snippet,
                    },
                    hit.sources,
                    category,
                )
            })
            .collect(),
        elapsed_ms: elapsed.as_millis() as u64,
        warnings: engine_failures,
    }
}

fn result(
    index: usize,
    hit: SearchHit,
    sources: Vec<String>,
    category: Option<SearchCategory>,
) -> SearchResult {
    SearchResult {
        position: index + 1,
        title: hit.title,
        url: hit.url,
        description: hit.snippet,
        sources,
        category,
        document: None,
        error: None,
    }
}

fn category_for_url(url: &str, categories: &[SearchCategory]) -> Option<SearchCategory> {
    if !categories.contains(&SearchCategory::GitHub) {
        return None;
    }
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?.trim_end_matches('.');
    (host == "github.com" || host.ends_with(".github.com")).then_some(SearchCategory::GitHub)
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
            categories: Vec::new(),
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
            categories: Vec::new(),
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
            categories: Vec::new(),
        };
        assert_eq!(
            service.selected_engines(&request).unwrap(),
            [metasearch::EngineKind::DuckDuckGo]
        );

        request.engines = vec![
            RequestedSearchEngine::BraveWeb,
            RequestedSearchEngine::GoogleCse,
            RequestedSearchEngine::GoogleCse,
        ];
        assert_eq!(
            service.selected_engines(&request).unwrap(),
            [
                metasearch::EngineKind::BraveWeb,
                metasearch::EngineKind::GoogleCse
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
    fn github_category_restricts_and_labels_matching_results() {
        let request = SearchRequest {
            query: "Rust HTTP client".into(),
            limit: 5,
            scrape_options: None,
            country: "us".into(),
            language: "en".into(),
            engines: Vec::new(),
            categories: vec![SearchCategory::GitHub],
        };
        assert_eq!(
            categorized_query(&request),
            "Rust HTTP client (site:github.com)"
        );
        assert_eq!(
            category_for_url("https://github.com/user/repo", &request.categories),
            Some(SearchCategory::GitHub)
        );
        assert_eq!(
            category_for_url("https://docs.github.com/en/rest", &request.categories),
            Some(SearchCategory::GitHub)
        );
        assert_eq!(
            category_for_url("https://example.com", &request.categories),
            None
        );
    }

    #[test]
    fn rejects_categorized_queries_over_limit() {
        let request = SearchRequest {
            query: "x".repeat(512),
            limit: 5,
            scrape_options: None,
            country: "us".into(),
            language: "en".into(),
            engines: Vec::new(),
            categories: vec![SearchCategory::GitHub],
        };
        assert!(validate(&request).is_err());
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
            categories: Vec::new(),
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
            categories: Vec::new(),
        };
        assert!(service.selected_engines(&request).is_err());

        request.engines = vec![RequestedSearchEngine::Wikipedia];
        assert_eq!(
            service.selected_engines(&request).unwrap(),
            [metasearch::EngineKind::Wikipedia]
        );
    }
}
