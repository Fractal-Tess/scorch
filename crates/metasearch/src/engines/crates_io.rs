use std::time::{Duration, Instant};

use reqwest::{Client, header};
use serde::Deserialize;
use url::Url;

use super::http::read_limited;
use crate::{BoxSearchFuture, EngineOutput, Error, Result, SearchEngine, SearchHit, SearchQuery};

const NAME: &str = "crates-io";
const ENDPOINT: &str = "https://crates.io/api/v1/crates";
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

pub struct CratesIo {
    client: Client,
}
impl CratesIo {
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
            .append_pair("q", query.query.trim())
            .append_pair("per_page", &query.limit.min(20).to_string());
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
        let payload: CratesResponse = serde_json::from_slice(&bytes).map_err(parse_error)?;
        let hits = payload
            .crates
            .into_iter()
            .filter_map(CrateItem::into_hit)
            .take(query.limit)
            .collect();
        Ok(EngineOutput {
            engine: NAME,
            hits,
            elapsed: started.elapsed(),
        })
    }
}
impl SearchEngine for CratesIo {
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
struct CratesResponse {
    #[serde(default)]
    crates: Vec<CrateItem>,
}
#[derive(Deserialize)]
struct CrateItem {
    id: String,
    description: Option<String>,
    max_stable_version: Option<String>,
    max_version: String,
}
impl CrateItem {
    fn into_hit(self) -> Option<SearchHit> {
        let id = self.id.trim();
        if id.is_empty() {
            return None;
        }
        let version = self
            .max_stable_version
            .as_deref()
            .unwrap_or(&self.max_version);
        let title = if version.trim().is_empty() {
            id.to_owned()
        } else {
            format!("{id} {version}")
        };
        let snippet = self
            .description
            .map(|value| normalize_space(&value))
            .filter(|value| !value.is_empty());
        Some(SearchHit {
            title,
            url: format!("https://crates.io/crates/{id}"),
            snippet,
        })
    }
}
fn normalize_space(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
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
    fn parses_crates() {
        let payload = br#"{"crates":[{"id":"reqwest","description":"An HTTP client","max_stable_version":"0.13.0","max_version":"0.13.0"}]}"#;
        let response: CratesResponse = serde_json::from_slice(payload).unwrap();
        let hit = response
            .crates
            .into_iter()
            .next()
            .unwrap()
            .into_hit()
            .unwrap();
        assert_eq!(hit.title, "reqwest 0.13.0");
        assert_eq!(hit.url, "https://crates.io/crates/reqwest");
    }
    #[tokio::test]
    #[ignore = "queries the public crates.io API"]
    async fn live_search_returns_public_results() {
        let output = CratesIo::new()
            .unwrap()
            .search(&SearchQuery::new("rust async", 3))
            .await
            .unwrap();
        assert!(!output.hits.is_empty());
    }
}
