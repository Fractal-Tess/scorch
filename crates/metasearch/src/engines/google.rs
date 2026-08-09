use std::time::{Duration, Instant};

use reqwest::Client;
use scraper::Html;
use serde::Deserialize;
use url::Url;

use crate::{BoxSearchFuture, EngineOutput, Error, Result, SearchEngine, SearchHit, SearchQuery};

use super::http::read_limited;

const NAME: &str = "google";
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

pub struct Google {
    client: Client,
    api_key: String,
    search_engine_id: String,
}

impl Google {
    pub fn new(api_key: impl Into<String>, search_engine_id: impl Into<String>) -> Result<Self> {
        let api_key = api_key.into();
        let search_engine_id = search_engine_id.into();
        if api_key.trim().is_empty() || search_engine_id.trim().is_empty() {
            return Err(Error::InvalidConfiguration(
                "Google requires an API key and Programmable Search Engine ID".into(),
            ));
        }
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(5))
            .user_agent("Scorch/0.1")
            .build()
            .map_err(|error| Error::Request {
                engine: NAME,
                message: error.to_string(),
            })?;
        Ok(Self {
            client,
            api_key,
            search_engine_id,
        })
    }

    async fn execute(&self, query: &SearchQuery) -> Result<EngineOutput> {
        let started = Instant::now();
        let mut url = Url::parse("https://customsearch.googleapis.com/customsearch/v1")
            .expect("constant URL is valid");
        url.query_pairs_mut()
            .append_pair("key", &self.api_key)
            .append_pair("cx", &self.search_engine_id)
            .append_pair("q", query.query.trim())
            .append_pair("num", &query.limit.min(10).to_string())
            .append_pair("gl", &query.country.to_ascii_lowercase())
            .append_pair(
                "lr",
                &format!("lang_{}", query.language.to_ascii_lowercase()),
            )
            .append_pair("safe", "active");
        let response = self
            .client
            .get(url)
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
        let payload: GoogleResponse =
            serde_json::from_slice(&bytes).map_err(|error| Error::Parse {
                engine: NAME,
                message: error.to_string(),
            })?;
        let hits = payload
            .items
            .into_iter()
            .filter_map(GoogleItem::into_hit)
            .take(query.limit)
            .collect();
        Ok(EngineOutput {
            engine: NAME,
            hits,
            elapsed: started.elapsed(),
        })
    }
}

impl SearchEngine for Google {
    fn name(&self) -> &'static str {
        NAME
    }

    fn weight(&self) -> f64 {
        1.1
    }

    fn search<'a>(&'a self, query: &'a SearchQuery) -> BoxSearchFuture<'a> {
        Box::pin(self.execute(query))
    }
}

#[derive(Deserialize)]
struct GoogleResponse {
    #[serde(default)]
    items: Vec<GoogleItem>,
}

#[derive(Deserialize)]
struct GoogleItem {
    title: String,
    link: String,
    snippet: Option<String>,
}

impl GoogleItem {
    fn into_hit(self) -> Option<SearchHit> {
        let url = Url::parse(&self.link).ok()?;
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
                .snippet
                .map(|snippet| clean_text(&snippet))
                .filter(|snippet| !snippet.is_empty()),
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
            message: error.without_url().to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_custom_search_results() {
        let payload = br#"{
          "items": [
            {
              "title": "The Rust Programming Language",
              "link": "https://www.rust-lang.org/",
              "snippet": "A language empowering everyone."
            }
          ]
        }"#;
        let response: GoogleResponse = serde_json::from_slice(payload).unwrap();
        let hit = response
            .items
            .into_iter()
            .next()
            .unwrap()
            .into_hit()
            .unwrap();
        assert_eq!(hit.title, "The Rust Programming Language");
        assert_eq!(hit.url, "https://www.rust-lang.org/");
        assert_eq!(
            hit.snippet.as_deref(),
            Some("A language empowering everyone.")
        );
    }

    #[test]
    fn rejects_incomplete_credentials() {
        assert!(Google::new("", "engine-id").is_err());
        assert!(Google::new("api-key", "").is_err());
    }

    #[tokio::test]
    #[ignore = "requires Google API credentials and queries the Custom Search JSON API"]
    async fn live_search_returns_public_results() {
        let key =
            std::env::var("GOOGLE_SEARCH_API_KEY").expect("GOOGLE_SEARCH_API_KEY is required");
        let search_engine_id =
            std::env::var("GOOGLE_SEARCH_ENGINE_ID").expect("GOOGLE_SEARCH_ENGINE_ID is required");
        let output = Google::new(key, search_engine_id)
            .unwrap()
            .search(&SearchQuery::new("Rust programming language", 3))
            .await
            .unwrap();
        assert!(!output.hits.is_empty());
    }
}
