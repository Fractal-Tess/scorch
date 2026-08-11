use super::http::read_limited;
use crate::{BoxSearchFuture, EngineOutput, Error, Result, SearchEngine, SearchHit, SearchQuery};
use reqwest::{Client, header};
use serde::Deserialize;
use std::time::{Duration, Instant};
use url::Url;
const NAME: &str = "docker-hub";
const MAX: usize = 2 * 1024 * 1024;
pub struct DockerHub {
    client: Client,
}
impl DockerHub {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .connect_timeout(Duration::from_secs(3))
                .timeout(Duration::from_secs(5))
                .user_agent(concat!("Scorch/", env!("CARGO_PKG_VERSION")))
                .build()
                .map_err(|e| Error::Request {
                    engine: NAME,
                    message: e.to_string(),
                })?,
        })
    }
    async fn execute(&self, q: &SearchQuery) -> Result<EngineOutput> {
        let started = Instant::now();
        let mut u = Url::parse("https://hub.docker.com/v2/search/repositories/").unwrap();
        u.query_pairs_mut()
            .append_pair("query", q.query.trim())
            .append_pair("page_size", &q.limit.min(20).to_string());
        let r = self
            .client
            .get(u)
            .header(header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(reqerr)?;
        let s = r.status();
        if s == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(Error::RateLimited { engine: NAME });
        }
        if !s.is_success() {
            return Err(Error::HttpStatus {
                engine: NAME,
                status: s.as_u16(),
            });
        }
        let b = read_limited(r, NAME, MAX).await?;
        let p: Response = serde_json::from_slice(&b).map_err(parseerr)?;
        let hits = p
            .results
            .into_iter()
            .filter_map(Item::hit)
            .take(q.limit)
            .collect();
        Ok(EngineOutput {
            engine: NAME,
            hits,
            elapsed: started.elapsed(),
        })
    }
}
impl SearchEngine for DockerHub {
    fn name(&self) -> &'static str {
        NAME
    }
    fn weight(&self) -> f64 {
        0.8
    }
    fn search<'a>(&'a self, q: &'a SearchQuery) -> BoxSearchFuture<'a> {
        Box::pin(self.execute(q))
    }
}
#[derive(Deserialize)]
struct Response {
    #[serde(default)]
    results: Vec<Item>,
}
#[derive(Deserialize)]
struct Item {
    repo_name: String,
    short_description: Option<String>,
    repo_owner: Option<String>,
}
impl Item {
    fn hit(self) -> Option<SearchHit> {
        let name = self.repo_name.trim();
        if name.is_empty() {
            return None;
        }
        let path =
            if name.contains('/') || self.repo_owner.as_deref().unwrap_or_default().is_empty() {
                name.to_owned()
            } else {
                format!("{}/{name}", self.repo_owner.unwrap())
            };
        Some(SearchHit {
            title: path.clone(),
            url: format!("https://hub.docker.com/r/{path}"),
            snippet: self
                .short_description
                .map(|v| v.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|v| !v.is_empty()),
        })
    }
}
fn reqerr(e: reqwest::Error) -> Error {
    if e.is_timeout() {
        Error::Timeout { engine: NAME }
    } else {
        Error::Request {
            engine: NAME,
            message: e.without_url().to_string(),
        }
    }
}
fn parseerr(e: impl std::fmt::Display) -> Error {
    Error::Parse {
        engine: NAME,
        message: e.to_string(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_repositories() {
        let p=br#"{"results":[{"repo_name":"rust","short_description":"Rust image","repo_owner":""}]}"#;
        let r: Response = serde_json::from_slice(p).unwrap();
        let h = r.results.into_iter().next().unwrap().hit().unwrap();
        assert_eq!(h.url, "https://hub.docker.com/r/rust");
    }
    #[tokio::test]
    #[ignore = "queries Docker Hub's public API"]
    async fn live_search_returns_public_results() {
        let o = DockerHub::new()
            .unwrap()
            .search(&SearchQuery::new("rust", 3))
            .await
            .unwrap();
        assert!(!o.hits.is_empty());
    }
}
