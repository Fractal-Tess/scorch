use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use reqwest::{Client, redirect::Policy};
use serde::Deserialize;
use tracing::debug;
use url::Url;

use super::http::read_limited;
use crate::{BoxSearchFuture, EngineOutput, Error, Result, SearchEngine, SearchHit, SearchQuery};

const NAME: &str = "wikipedia";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(4);
const DEFAULT_RESPONSE_LIMIT: usize = 2 * 1024 * 1024;

pub struct Wikipedia {
    client: Arc<Client>,
    timeout: Duration,
    response_limit: usize,
}

impl Wikipedia {
    pub fn new() -> Result<Self> {
        Self::with_options(DEFAULT_TIMEOUT, DEFAULT_RESPONSE_LIMIT)
    }

    pub fn with_options(timeout: Duration, response_limit: usize) -> Result<Self> {
        let client = Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .user_agent("ScorchBot/0.1 metasearch")
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
        let language = normalized_language(&query.language);
        let mut endpoint = Url::parse(&format!("https://{language}.wikipedia.org/w/api.php"))
            .expect("validated language creates a valid endpoint");
        endpoint
            .query_pairs_mut()
            .append_pair("action", "query")
            .append_pair("generator", "search")
            .append_pair("gsrsearch", search)
            .append_pair("gsrlimit", &query.limit.to_string())
            .append_pair("prop", "extracts|info")
            .append_pair("exintro", "1")
            .append_pair("explaintext", "1")
            .append_pair("inprop", "url")
            .append_pair("format", "json")
            .append_pair("formatversion", "2");

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
        let payload: ApiResponse = serde_json::from_slice(&body).map_err(|error| Error::Parse {
            engine: NAME,
            message: error.to_string(),
        })?;
        let mut pages = payload.query.map_or_else(Vec::new, |query| query.pages);
        pages.sort_unstable_by_key(|page| page.index);
        let hits = pages
            .into_iter()
            .filter_map(|page| {
                let url = page.full_url.and_then(|url| public_url(&url))?;
                let title = normalize_space(&page.title);
                (!title.is_empty()).then(|| SearchHit {
                    title,
                    url,
                    snippet: page
                        .extract
                        .map(|extract| normalize_space(&extract))
                        .filter(|extract| !extract.is_empty()),
                })
            })
            .take(query.limit)
            .collect::<Vec<_>>();
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

impl Default for Wikipedia {
    fn default() -> Self {
        Self::new().expect("Wikipedia client configuration is valid")
    }
}

impl SearchEngine for Wikipedia {
    fn name(&self) -> &'static str {
        NAME
    }

    fn weight(&self) -> f64 {
        0.55
    }

    fn search<'a>(&'a self, query: &'a SearchQuery) -> BoxSearchFuture<'a> {
        Box::pin(self.execute(query))
    }
}

#[derive(Deserialize)]
struct ApiResponse {
    query: Option<ApiQuery>,
}

#[derive(Deserialize)]
struct ApiQuery {
    #[serde(default)]
    pages: Vec<ApiPage>,
}

#[derive(Deserialize)]
struct ApiPage {
    #[serde(default = "last_index")]
    index: i64,
    title: String,
    #[serde(rename = "fullurl")]
    full_url: Option<String>,
    extract: Option<String>,
}

fn last_index() -> i64 {
    i64::MAX
}

fn normalized_language(language: &str) -> &str {
    let language = language.trim();
    if (2..=12).contains(&language.len())
        && language
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        language
    } else {
        "en"
    }
}

fn public_url(raw: &str) -> Option<String> {
    let mut url = Url::parse(raw).ok()?;
    if url.scheme() != "https"
        || !url
            .host_str()
            .is_some_and(|host| host.ends_with(".wikipedia.org"))
    {
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
    fn parses_ranked_api_results() {
        let payload = br#"{
          "query": {"pages": [
            {"index": 2, "title": "Rust", "fullurl": "https://en.wikipedia.org/wiki/Rust", "extract": "A language"},
            {"index": 1, "title": "Rust (programming language)", "fullurl": "https://en.wikipedia.org/wiki/Rust_(programming_language)", "extract": "A systems language"}
          ]}
        }"#;
        let mut parsed: ApiResponse = serde_json::from_slice(payload).unwrap();
        let mut pages = parsed.query.take().unwrap().pages;
        pages.sort_unstable_by_key(|page| page.index);
        assert_eq!(pages[0].title, "Rust (programming language)");
    }

    #[test]
    fn language_is_restricted_to_safe_subdomains() {
        assert_eq!(normalized_language("de"), "de");
        assert_eq!(normalized_language("../../evil"), "en");
    }

    #[tokio::test]
    #[ignore = "queries the public Wikipedia service"]
    async fn live_search_returns_public_results() {
        let output = Wikipedia::new()
            .unwrap()
            .search(&SearchQuery::new("Rust programming language", 3))
            .await
            .unwrap();
        assert!(!output.hits.is_empty());
        assert!(
            output
                .hits
                .iter()
                .all(|hit| hit.url.contains(".wikipedia.org/wiki/"))
        );
    }
}
