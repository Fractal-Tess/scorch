use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use metasearch::{
    EngineOutput, MetaSearch, MetaSearchConfig, MetaSearchOutput, SearchEngine as _,
    SearchHit as MetaSearchHit, SearchQuery,
    engines::{Bing, Naver, Wikipedia},
};
use scorch_types::{SearchProvider, SearchRequest, SearchResponse, SearchResult};
use scraper::{ElementRef, Html, Selector};
use tokio::sync::Semaphore;
use url::Url;

use crate::{
    config::EngineConfig,
    error::{EngineError, Result},
    fetch::SafeFetcher,
};

pub struct SearchService {
    default_provider: SearchProvider,
    fetcher: SafeFetcher,
    metasearch: MetaSearch,
    bing: Arc<Bing>,
    naver: Arc<Naver>,
    wikipedia: Arc<Wikipedia>,
    limit: Arc<Semaphore>,
}

impl SearchService {
    pub fn new(fetcher: SafeFetcher, config: &EngineConfig) -> Result<Self> {
        let bing = Arc::new(Bing::new().map_err(search_error)?);
        let naver = Arc::new(Naver::new().map_err(search_error)?);
        let wikipedia = Arc::new(Wikipedia::new().map_err(search_error)?);
        let engines: Vec<Arc<dyn metasearch::SearchEngine>> =
            vec![bing.clone(), naver.clone(), wikipedia.clone()];
        let metasearch = MetaSearch::with_engines(
            MetaSearchConfig {
                per_engine_concurrency: config.max_concurrency,
                ..Default::default()
            },
            engines,
        );
        Ok(Self {
            default_provider: config.default_search_provider,
            fetcher,
            metasearch,
            bing,
            naver,
            wikipedia,
            limit: Arc::new(Semaphore::new(config.max_concurrency.max(1))),
        })
    }

    pub async fn search(&self, request: &SearchRequest) -> Result<SearchResponse> {
        validate(request)?;
        let _permit = self
            .limit
            .acquire()
            .await
            .map_err(|_| EngineError::Capacity("search runtime is shutting down".into()))?;
        let provider = request.provider.unwrap_or(self.default_provider);
        let query = SearchQuery {
            query: request.query.trim().into(),
            limit: request.limit,
            country: request.country.clone(),
            language: request.language.clone(),
        };
        match provider {
            SearchProvider::Metasearch => self
                .metasearch
                .search(&query)
                .await
                .map(|output| from_metasearch(request, output))
                .map_err(search_error),
            SearchProvider::Bing => self
                .bing
                .search(&query)
                .await
                .and_then(require_hits)
                .map(|output| from_engine(request, output))
                .map_err(search_error),
            SearchProvider::Naver => self
                .naver
                .search(&query)
                .await
                .and_then(require_hits)
                .map(|output| from_engine(request, output))
                .map_err(search_error),
            SearchProvider::Wikipedia => self
                .wikipedia
                .search(&query)
                .await
                .and_then(require_hits)
                .map(|output| from_engine(request, output))
                .map_err(search_error),
            SearchProvider::Duckduckgo => duckduckgo(&self.fetcher, request).await,
        }
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
    Ok(())
}

fn require_hits(output: EngineOutput) -> metasearch::Result<EngineOutput> {
    if output.hits.is_empty() {
        return Err(metasearch::Error::AllEnginesFailed(format!(
            "{} returned no results",
            output.engine
        )));
    }
    Ok(output)
}

fn from_engine(request: &SearchRequest, output: EngineOutput) -> SearchResponse {
    SearchResponse {
        query: request.query.trim().into(),
        provider: output.engine.into(),
        results: output
            .hits
            .into_iter()
            .enumerate()
            .map(|(index, hit)| result(index, hit, vec![output.engine.into()]))
            .collect(),
        elapsed_ms: output.elapsed.as_millis() as u64,
        warnings: Vec::new(),
    }
}

fn from_metasearch(request: &SearchRequest, output: MetaSearchOutput) -> SearchResponse {
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
                    MetaSearchHit {
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

fn result(index: usize, hit: MetaSearchHit, sources: Vec<String>) -> SearchResult {
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

async fn duckduckgo(fetcher: &SafeFetcher, request: &SearchRequest) -> Result<SearchResponse> {
    let started = Instant::now();
    let mut url = Url::parse("https://html.duckduckgo.com/html/").expect("constant URL is valid");
    url.query_pairs_mut()
        .append_pair("q", request.query.trim())
        .append_pair(
            "kl",
            &format!(
                "{}-{}",
                request.country.to_ascii_lowercase(),
                request.language.to_ascii_lowercase()
            ),
        )
        .append_pair("kp", "1");
    let response = fetcher
        .get_with_user_agent(
            url.as_str(),
            Duration::from_secs(15),
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/120 Safari/537.36",
        )
        .await?;
    if !response.status.is_success() {
        return Err(EngineError::Search(format!(
            "duckduckgo returned HTTP {}",
            response.status
        )));
    }
    let html = String::from_utf8_lossy(&response.body);
    let mut hits = parse_duckduckgo(&html);
    hits.truncate(request.limit);
    if hits.is_empty() {
        return Err(EngineError::Search("duckduckgo returned no results".into()));
    }
    Ok(SearchResponse {
        query: request.query.trim().into(),
        provider: "duckduckgo".into(),
        results: hits
            .into_iter()
            .enumerate()
            .map(|(index, hit)| result(index, hit, vec!["duckduckgo".into()]))
            .collect(),
        elapsed_ms: started.elapsed().as_millis() as u64,
        warnings: Vec::new(),
    })
}

fn parse_duckduckgo(html: &str) -> Vec<MetaSearchHit> {
    let document = Html::parse_document(html);
    if Selector::parse(".anomaly-modal__modal")
        .ok()
        .is_some_and(|selector| document.select(&selector).next().is_some())
    {
        return Vec::new();
    }
    let result_selector = Selector::parse(".result.web-result").expect("valid selector");
    let link_selector = Selector::parse(".result__a").expect("valid selector");
    let snippet_selector = Selector::parse(".result__snippet").expect("valid selector");
    document
        .select(&result_selector)
        .filter_map(|block| build_duckduckgo_hit(&block, &link_selector, &snippet_selector))
        .collect()
}

fn build_duckduckgo_hit(
    block: &ElementRef<'_>,
    link_selector: &Selector,
    snippet_selector: &Selector,
) -> Option<MetaSearchHit> {
    let link = block.select(link_selector).next()?;
    let url = clean_duckduckgo_url(link.value().attr("href")?)?;
    let title = normalize_space(&link.text().collect::<Vec<_>>().join(" "));
    if title.is_empty() {
        return None;
    }
    let snippet = block
        .select(snippet_selector)
        .next()
        .map(|node| normalize_space(&node.text().collect::<Vec<_>>().join(" ")))
        .filter(|value| !value.is_empty());
    Some(MetaSearchHit {
        title,
        url,
        snippet,
    })
}

fn clean_duckduckgo_url(raw: &str) -> Option<String> {
    let parsed = Url::parse(raw)
        .or_else(|_| Url::parse("https://duckduckgo.com").and_then(|base| base.join(raw)))
        .ok()?;
    let candidate = parsed
        .query_pairs()
        .find(|(key, _)| key == "uddg")
        .map(|(_, value)| value.into_owned())
        .unwrap_or_else(|| parsed.to_string());
    let mut url = Url::parse(&candidate).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    url.set_fragment(None);
    Some(url.to_string())
}

fn normalize_space(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_duckduckgo_result() {
        let html = r#"<div class="result web-result"><a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2F">Example</a><a class="result__snippet">A result</a></div>"#;
        let results = parse_duckduckgo(html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://example.com/");
    }
}
