use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD},
};
use reqwest::{Client, redirect::Policy};
use scraper::{ElementRef, Html, Selector};
use tracing::debug;
use url::Url;

use super::http::read_limited;
use crate::{BoxSearchFuture, EngineOutput, Error, Result, SearchEngine, SearchHit, SearchQuery};

const NAME: &str = "bing";
const ENDPOINT: &str = "https://www.bing.com/search";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_RESPONSE_LIMIT: usize = 2 * 1024 * 1024;

pub struct Bing {
    client: Arc<Client>,
    timeout: Duration,
    response_limit: usize,
}

impl Bing {
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
        endpoint
            .query_pairs_mut()
            .append_pair("q", search)
            .append_pair("count", &query.limit.to_string())
            .append_pair("setlang", &query.language)
            .append_pair("cc", &query.country);
        let request = self.client.get(endpoint);
        let response = tokio::time::timeout(self.timeout, request.send())
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

impl Default for Bing {
    fn default() -> Self {
        Self::new().expect("Bing client configuration is valid")
    }
}

impl SearchEngine for Bing {
    fn name(&self) -> &'static str {
        NAME
    }

    fn search<'a>(&'a self, query: &'a SearchQuery) -> BoxSearchFuture<'a> {
        Box::pin(self.execute(query))
    }
}

fn parse(html: &str) -> Vec<SearchHit> {
    let document = Html::parse_document(html);
    let result_selector = Selector::parse("li.b_algo").expect("constant selector is valid");
    let link_selector = Selector::parse("h2 a").expect("constant selector is valid");
    let snippet_selector = Selector::parse(".b_caption p").expect("constant selector is valid");
    document
        .select(&result_selector)
        .filter_map(|block| build_hit(&block, &link_selector, &snippet_selector))
        .collect()
}

fn build_hit(
    block: &ElementRef<'_>,
    link_selector: &Selector,
    snippet_selector: &Selector,
) -> Option<SearchHit> {
    let link = block.select(link_selector).next()?;
    let url = clean_url(link.value().attr("href")?)?;
    let title = normalize_space(&link.text().collect::<Vec<_>>().join(" "));
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

fn clean_url(raw: &str) -> Option<String> {
    let parsed = Url::parse(raw).ok()?;
    if parsed
        .host_str()
        .is_some_and(|host| host.ends_with("bing.com"))
        && let Some(encoded) = parsed
            .query_pairs()
            .find(|(key, _)| key == "u")
            .map(|(_, value)| value.into_owned())
            .and_then(|value| value.strip_prefix("a1").map(ToOwned::to_owned))
        && let Some(decoded) = decode_redirect(&encoded)
    {
        return public_url(&decoded);
    }
    public_url(raw)
}

fn decode_redirect(encoded: &str) -> Option<String> {
    STANDARD_NO_PAD
        .decode(encoded)
        .or_else(|_| URL_SAFE_NO_PAD.decode(encoded))
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
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
    fn parses_results_and_decodes_redirects() {
        let html = r#"
            <li class="b_algo">
              <h2><a href="https://www.bing.com/ck/a?u=a1aHR0cHM6Ly9leGFtcGxlLmNvbS8">Example</a></h2>
              <div class="b_caption"><p>A useful result</p></div>
            </li>
        "#;
        assert_eq!(
            parse(html),
            vec![SearchHit {
                title: "Example".into(),
                url: "https://example.com/".into(),
                snippet: Some("A useful result".into()),
            }]
        );
    }

    #[tokio::test]
    #[ignore = "queries the public Bing service"]
    async fn live_search_returns_public_results() {
        let output = Bing::new()
            .unwrap()
            .search(&SearchQuery::new("Rust programming language", 3))
            .await
            .unwrap();
        assert!(!output.hits.is_empty());
        assert!(output.hits.iter().all(|hit| Url::parse(&hit.url).is_ok()));
    }
}
