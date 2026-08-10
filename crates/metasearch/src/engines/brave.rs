use std::time::{Duration, Instant};

use reqwest::{Client, header};
use scraper::Html;
use serde::Deserialize;
use url::Url;

use crate::{BoxSearchFuture, EngineOutput, Error, Result, SearchEngine, SearchHit, SearchQuery};

use super::http::read_limited;

const NAME: &str = "brave";
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

pub struct Brave {
    client: Client,
    api_key: String,
}

impl Brave {
    pub fn new(api_key: impl Into<String>) -> Result<Self> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return Err(Error::InvalidConfiguration(
                "Brave requires a non-empty API key".into(),
            ));
        }
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(5))
            .user_agent(concat!("Scorch/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| Error::Request {
                engine: NAME,
                message: error.to_string(),
            })?;
        Ok(Self { client, api_key })
    }

    async fn execute(&self, query: &SearchQuery) -> Result<EngineOutput> {
        let started = Instant::now();
        let mut url = Url::parse("https://api.search.brave.com/res/v1/web/search")
            .expect("constant URL is valid");
        url.query_pairs_mut()
            .append_pair("q", query.query.trim())
            .append_pair("count", &query.limit.min(20).to_string())
            .append_pair("country", &query.country.to_ascii_lowercase())
            .append_pair("search_lang", &query.language.to_ascii_lowercase())
            .append_pair("safesearch", "moderate");
        let response = self
            .client
            .get(url)
            .header(header::ACCEPT, "application/json")
            .header("X-Subscription-Token", &self.api_key)
            .send()
            .await
            .map_err(|error| request_error(error, NAME))?;
        if !response.status().is_success() {
            return Err(Error::HttpStatus {
                engine: NAME,
                status: response.status().as_u16(),
            });
        }
        let bytes = read_limited(response, NAME, MAX_RESPONSE_BYTES).await?;
        let payload: BraveResponse =
            serde_json::from_slice(&bytes).map_err(|error| Error::Parse {
                engine: NAME,
                message: error.to_string(),
            })?;
        let hits = payload
            .web
            .map(|web| web.results)
            .unwrap_or_default()
            .into_iter()
            .filter_map(BraveResult::into_hit)
            .take(query.limit)
            .collect();
        Ok(EngineOutput {
            engine: NAME,
            hits,
            elapsed: started.elapsed(),
        })
    }
}

impl SearchEngine for Brave {
    fn name(&self) -> &'static str {
        NAME
    }

    fn weight(&self) -> f64 {
        1.05
    }

    fn search<'a>(&'a self, query: &'a SearchQuery) -> BoxSearchFuture<'a> {
        Box::pin(self.execute(query))
    }
}

#[derive(Deserialize)]
struct BraveResponse {
    web: Option<BraveWeb>,
}

#[derive(Deserialize)]
struct BraveWeb {
    #[serde(default)]
    results: Vec<BraveResult>,
}

#[derive(Deserialize)]
struct BraveResult {
    title: String,
    url: String,
    description: Option<String>,
}

impl BraveResult {
    fn into_hit(self) -> Option<SearchHit> {
        let url = Url::parse(&self.url).ok()?;
        if !matches!(url.scheme(), "http" | "https") {
            return None;
        }
        let title = clean_text(&self.title);
        if title.is_empty() {
            return None;
        }
        Some(SearchHit {
            title,
            url: url.to_string(),
            snippet: self
                .description
                .map(|description| clean_text(&description))
                .filter(|description| !description.is_empty()),
        })
    }
}

fn clean_text(value: &str) -> String {
    Html::parse_fragment(value)
        .root_element()
        .text()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn request_error(error: reqwest::Error, engine: &'static str) -> Error {
    if error.is_timeout() {
        Error::Timeout { engine }
    } else {
        Error::Request {
            engine,
            message: error.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_api_results_and_removes_markup() {
        let payload = br#"{
          "web": {"results": [
            {
              "title": "The <strong>Rust</strong> Programming Language",
              "url": "https://www.rust-lang.org/",
              "description": "A language empowering <strong>everyone</strong>."
            }
          ]}
        }"#;
        let response: BraveResponse = serde_json::from_slice(payload).unwrap();
        let hit = response.web.unwrap().results.remove(0).into_hit().unwrap();
        assert_eq!(hit.title, "The Rust Programming Language");
        assert_eq!(hit.url, "https://www.rust-lang.org/");
        assert_eq!(
            hit.snippet.as_deref(),
            Some("A language empowering everyone.")
        );
    }

    #[test]
    fn rejects_empty_api_keys() {
        assert!(Brave::new("  ").is_err());
    }

    #[tokio::test]
    #[ignore = "requires BRAVE_SEARCH_API_KEY and queries the Brave Search API"]
    async fn live_search_returns_public_results() {
        let key = std::env::var("BRAVE_SEARCH_API_KEY").expect("BRAVE_SEARCH_API_KEY is required");
        let output = Brave::new(key)
            .unwrap()
            .search(&SearchQuery::new("Rust programming language", 3))
            .await
            .unwrap();
        assert!(!output.hits.is_empty());
    }
}
