use super::http::read_limited;
use crate::{BoxSearchFuture, EngineOutput, Error, Result, SearchEngine, SearchHit, SearchQuery};
use reqwest::{Client, header};
use serde::Deserialize;
use std::time::{Duration, Instant};
use url::Url;
const NAME: &str = "open-library";
const ENDPOINT: &str = "https://openlibrary.org/search.json";
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
pub struct OpenLibrary {
    client: Client,
}
impl OpenLibrary {
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(5))
            .user_agent(concat!(
                "Scorch/",
                env!("CARGO_PKG_VERSION"),
                " (https://github.com/Fractal-Tess/scorch)"
            ))
            .build()
            .map_err(|e| Error::Request {
                engine: NAME,
                message: e.to_string(),
            })?;
        Ok(Self { client })
    }
    async fn execute(&self, q: &SearchQuery) -> Result<EngineOutput> {
        let started = Instant::now();
        let mut url = Url::parse(ENDPOINT).expect("constant URL is valid");
        url.query_pairs_mut()
            .append_pair("q", q.query.trim())
            .append_pair("limit", &q.limit.min(20).to_string())
            .append_pair(
                "fields",
                "key,title,author_name,first_publish_year,edition_count",
            );
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
        let payload: OpenLibraryResponse = serde_json::from_slice(&bytes).map_err(parse_error)?;
        let hits = payload
            .docs
            .into_iter()
            .filter_map(OpenLibraryDoc::into_hit)
            .take(q.limit)
            .collect();
        Ok(EngineOutput {
            engine: NAME,
            hits,
            elapsed: started.elapsed(),
        })
    }
}
impl SearchEngine for OpenLibrary {
    fn name(&self) -> &'static str {
        NAME
    }
    fn weight(&self) -> f64 {
        0.85
    }
    fn search<'a>(&'a self, q: &'a SearchQuery) -> BoxSearchFuture<'a> {
        Box::pin(self.execute(q))
    }
}
#[derive(Deserialize)]
struct OpenLibraryResponse {
    #[serde(default)]
    docs: Vec<OpenLibraryDoc>,
}
#[derive(Deserialize)]
struct OpenLibraryDoc {
    key: String,
    title: String,
    #[serde(default)]
    author_name: Vec<String>,
    first_publish_year: Option<i32>,
    edition_count: Option<u64>,
}
impl OpenLibraryDoc {
    fn into_hit(self) -> Option<SearchHit> {
        let title = normalize_space(&self.title);
        if title.is_empty() || !self.key.starts_with("/works/") {
            return None;
        }
        let mut details = Vec::new();
        if !self.author_name.is_empty() {
            details.push(
                self.author_name
                    .into_iter()
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }
        if let Some(y) = self.first_publish_year {
            details.push(y.to_string());
        }
        if let Some(c) = self.edition_count {
            details.push(format!("{c} editions"));
        }
        Some(SearchHit {
            title,
            url: format!("https://openlibrary.org{}", self.key),
            snippet: (!details.is_empty()).then(|| details.join(" — ")),
        })
    }
}
fn normalize_space(v: &str) -> String {
    v.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn request_error(e: reqwest::Error) -> Error {
    if e.is_timeout() {
        Error::Timeout { engine: NAME }
    } else {
        Error::Request {
            engine: NAME,
            message: e.without_url().to_string(),
        }
    }
}
fn parse_error(e: impl std::fmt::Display) -> Error {
    Error::Parse {
        engine: NAME,
        message: e.to_string(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_books() {
        let p=br#"{"docs":[{"key":"/works/OL1W","title":"The Rust Book","author_name":["Ada"],"first_publish_year":2018,"edition_count":4}]}"#;
        let r: OpenLibraryResponse = serde_json::from_slice(p).unwrap();
        let h = r.docs.into_iter().next().unwrap().into_hit().unwrap();
        assert_eq!(h.url, "https://openlibrary.org/works/OL1W");
        assert_eq!(h.snippet.as_deref(), Some("Ada — 2018 — 4 editions"));
    }
    #[tokio::test]
    #[ignore = "queries the public Open Library API"]
    async fn live_search_returns_public_results() {
        let o = OpenLibrary::new()
            .unwrap()
            .search(&SearchQuery::new("Rust programming language", 3))
            .await
            .unwrap();
        assert!(!o.hits.is_empty());
    }
}
