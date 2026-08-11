use std::time::{Duration, Instant};

use reqwest::{Client, header};
use scraper::Html;
use serde::Deserialize;
use url::Url;

use crate::{BoxSearchFuture, EngineOutput, Error, Result, SearchEngine, SearchHit, SearchQuery};

use super::http::read_limited;

const NAME: &str = "hacker-news";
const ENDPOINT: &str = "https://hn.algolia.com/api/v1/search";
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

pub struct HackerNews {
    client: Client,
}

impl HackerNews {
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(5))
            .user_agent(concat!("Scorch/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| Error::Request {
                engine: NAME,
                message: error.to_string(),
            })?;
        Ok(Self { client })
    }

    async fn execute(&self, query: &SearchQuery) -> Result<EngineOutput> {
        let started = Instant::now();
        let mut url = Url::parse(ENDPOINT).expect("constant URL is valid");
        url.query_pairs_mut()
            .append_pair("query", query.query.trim())
            .append_pair("hitsPerPage", &query.limit.min(20).to_string())
            .append_pair("tags", "story");
        let response = self
            .client
            .get(url)
            .header(header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(request_error)?;
        let status = response.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(Error::RateLimited { engine: NAME });
        }
        if !status.is_success() {
            return Err(Error::HttpStatus {
                engine: NAME,
                status: status.as_u16(),
            });
        }
        let bytes = read_limited(response, NAME, MAX_RESPONSE_BYTES).await?;
        let payload: HackerNewsResponse = serde_json::from_slice(&bytes).map_err(parse_error)?;
        let hits = payload
            .hits
            .into_iter()
            .filter_map(HackerNewsItem::into_hit)
            .take(query.limit)
            .collect();
        Ok(EngineOutput {
            engine: NAME,
            hits,
            elapsed: started.elapsed(),
        })
    }
}

impl SearchEngine for HackerNews {
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

#[derive(Deserialize)]
struct HackerNewsResponse {
    #[serde(default)]
    hits: Vec<HackerNewsItem>,
}

#[derive(Deserialize)]
struct HackerNewsItem {
    title: Option<String>,
    url: Option<String>,
    #[serde(rename = "objectID")]
    object_id: String,
    story_text: Option<String>,
}

impl HackerNewsItem {
    fn into_hit(self) -> Option<SearchHit> {
        let title = self
            .title
            .as_deref()
            .map(clean_text)
            .filter(|value| !value.is_empty())?;
        let url =
            self.url.as_deref().and_then(public_url).unwrap_or_else(|| {
                format!("https://news.ycombinator.com/item?id={}", self.object_id)
            });
        let snippet = self
            .story_text
            .as_deref()
            .map(clean_text)
            .filter(|value| !value.is_empty());
        Some(SearchHit {
            title,
            url,
            snippet,
        })
    }
}

fn public_url(raw: &str) -> Option<String> {
    let mut url = Url::parse(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    url.set_fragment(None);
    Some(url.to_string())
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

fn request_error(error: reqwest::Error) -> Error {
    if error.is_timeout() {
        Error::Timeout { engine: NAME }
    } else {
        Error::Request {
            engine: NAME,
            message: error.without_url().to_string(),
        }
    }
}
fn parse_error(error: impl std::fmt::Display) -> Error {
    Error::Parse {
        engine: NAME,
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_stories_and_falls_back_to_discussion_url() {
        let payload = br#"{"hits":[{"title":"Rust <em>News</em>","url":null,"objectID":"42","story_text":"A <b>story</b>."}]}"#;
        let response: HackerNewsResponse = serde_json::from_slice(payload).unwrap();
        let hit = response
            .hits
            .into_iter()
            .next()
            .unwrap()
            .into_hit()
            .unwrap();
        assert_eq!(hit.title, "Rust News");
        assert_eq!(hit.url, "https://news.ycombinator.com/item?id=42");
        assert_eq!(hit.snippet.as_deref(), Some("A story."));
    }
    #[tokio::test]
    #[ignore = "queries the public Hacker News Algolia API"]
    async fn live_search_returns_public_results() {
        let output = HackerNews::new()
            .unwrap()
            .search(&SearchQuery::new("Rust programming language", 3))
            .await
            .unwrap();
        assert!(!output.hits.is_empty());
    }
}
