use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use reqwest::{Client, redirect::Policy};
use scraper::{ElementRef, Html, Selector};
use tracing::debug;
use url::Url;

use super::http::read_limited;
use crate::{BoxSearchFuture, EngineOutput, Error, Result, SearchEngine, SearchHit, SearchQuery};

const NAME: &str = "naver";
const ENDPOINT: &str = "https://search.naver.com/search.naver";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_RESPONSE_LIMIT: usize = 2 * 1024 * 1024;

pub struct Naver {
    client: Arc<Client>,
    timeout: Duration,
    response_limit: usize,
}

impl Naver {
    pub fn new() -> Result<Self> {
        Self::with_options(DEFAULT_TIMEOUT, DEFAULT_RESPONSE_LIMIT)
    }

    pub fn with_options(timeout: Duration, response_limit: usize) -> Result<Self> {
        let client = Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .user_agent("Mozilla/5.0")
            .build()
            .map_err(|error| Error::Request {
                engine: NAME,
                message: error.to_string(),
            })?;
        Ok(Self {
            client: Arc::new(client),
            timeout,
            response_limit,
        })
    }

    async fn execute(&self, query: &SearchQuery) -> Result<EngineOutput> {
        let search = query.query.trim();
        if search.is_empty() {
            return Err(Error::InvalidQuery("query cannot be empty".into()));
        }
        if !(1..=20).contains(&query.limit) {
            return Err(Error::InvalidQuery("limit must be between 1 and 20".into()));
        }

        let started = Instant::now();
        let mut endpoint = Url::parse(ENDPOINT).expect("constant endpoint is valid");
        endpoint.query_pairs_mut().append_pair("query", search);
        let response = tokio::time::timeout(self.timeout, self.client.get(endpoint).send())
            .await
            .map_err(|_| Error::Timeout { engine: NAME })?
            .map_err(|error| Error::Request {
                engine: NAME,
                message: error.to_string(),
            })?;
        if !response.status().is_success() {
            return Err(Error::HttpStatus {
                engine: NAME,
                status: response.status().as_u16(),
            });
        }
        let body = read_limited(response, NAME, self.response_limit).await?;
        let html = String::from_utf8_lossy(&body);
        let mut hits = parse(&html);
        hits.truncate(query.limit);
        debug!(
            engine = NAME,
            result_count = hits.len(),
            "search engine completed"
        );
        Ok(EngineOutput {
            engine: NAME,
            hits,
            elapsed: started.elapsed(),
        })
    }
}

impl Default for Naver {
    fn default() -> Self {
        Self::new().expect("Naver client configuration is valid")
    }
}

impl SearchEngine for Naver {
    fn name(&self) -> &'static str {
        NAME
    }

    fn weight(&self) -> f64 {
        0.85
    }

    fn search<'a>(&'a self, query: &'a SearchQuery) -> BoxSearchFuture<'a> {
        Box::pin(self.execute(query))
    }
}

fn parse(html: &str) -> Vec<SearchHit> {
    let document = Html::parse_document(html);
    let block_selector = Selector::parse(".fds-web-doc-root").expect("constant selector is valid");
    let link_selector =
        Selector::parse("a[data-heatmap-target='.link']").expect("constant selector is valid");
    let title_selector =
        Selector::parse("span.sds-comps-text-type-headline1").expect("constant selector is valid");
    let snippet_selector =
        Selector::parse("span.sds-comps-text-type-body1").expect("constant selector is valid");

    document
        .select(&block_selector)
        .filter_map(|block| build_hit(&block, &link_selector, &title_selector, &snippet_selector))
        .collect()
}

fn build_hit(
    block: &ElementRef<'_>,
    link_selector: &Selector,
    title_selector: &Selector,
    snippet_selector: &Selector,
) -> Option<SearchHit> {
    let link = block
        .select(link_selector)
        .find(|link| link.select(title_selector).next().is_some())?;
    let title_node = link.select(title_selector).next()?;
    let title = normalize_space(&title_node.text().collect::<Vec<_>>().join(" "));
    let url = public_url(link.value().attr("href")?)?;
    if title.is_empty() {
        return None;
    }
    let snippet = block
        .select(snippet_selector)
        .next()
        .map(|node| normalize_space(&node.text().collect::<Vec<_>>().join(" ")))
        .filter(|value| !value.is_empty());
    Some(SearchHit {
        title,
        url,
        snippet,
    })
}

fn public_url(raw: &str) -> Option<String> {
    let mut url = Url::parse(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
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
    fn parses_web_result_cards() {
        let html = r#"
          <div class="fds-web-doc-root">
            <a href="https://example.com/" data-heatmap-target=".link">
              <span class="sds-comps-text-type-headline1"><mark>Example</mark> Domain</span>
            </a>
            <span class="sds-comps-text-type-body1">A useful example page.</span>
          </div>
        "#;
        assert_eq!(
            parse(html),
            vec![SearchHit {
                title: "Example Domain".into(),
                url: "https://example.com/".into(),
                snippet: Some("A useful example page.".into()),
            }]
        );
    }

    #[tokio::test]
    #[ignore = "queries the public Naver service"]
    async fn live_search_returns_public_results() {
        let output = Naver::new()
            .unwrap()
            .search(&SearchQuery::new("Rust programming language", 3))
            .await
            .unwrap();
        assert!(!output.hits.is_empty());
        assert!(output.hits.iter().all(|hit| Url::parse(&hit.url).is_ok()));
    }
}
