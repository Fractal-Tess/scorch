use super::http::read_limited;
use crate::{BoxSearchFuture, EngineOutput, Error, Result, SearchEngine, SearchHit, SearchQuery};
use reqwest::{Client, header};
use serde::Deserialize;
use std::time::{Duration, Instant};
use url::Url;
const NAME: &str = "github";
const MAX: usize = 2 * 1024 * 1024;
pub struct GitHub {
    client: Client,
}
impl GitHub {
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
        let mut u = Url::parse("https://api.github.com/search/repositories").unwrap();
        u.query_pairs_mut()
            .append_pair("q", q.query.trim())
            .append_pair("per_page", &q.limit.min(20).to_string())
            .append_pair("sort", "stars");
        let r = self
            .client
            .get(u)
            .header(header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(reqerr)?;
        let s = r.status();
        if s == reqwest::StatusCode::TOO_MANY_REQUESTS
            || s == reqwest::StatusCode::FORBIDDEN
                && r.headers()
                    .get("x-ratelimit-remaining")
                    .is_some_and(|v| v == "0")
        {
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
            .items
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
impl SearchEngine for GitHub {
    fn name(&self) -> &'static str {
        NAME
    }
    fn weight(&self) -> f64 {
        0.9
    }
    fn search<'a>(&'a self, q: &'a SearchQuery) -> BoxSearchFuture<'a> {
        Box::pin(self.execute(q))
    }
}
#[derive(Deserialize)]
struct Response {
    #[serde(default)]
    items: Vec<Item>,
}
#[derive(Deserialize)]
struct Item {
    full_name: String,
    html_url: String,
    description: Option<String>,
    language: Option<String>,
    stargazers_count: u64,
}
impl Item {
    fn hit(self) -> Option<SearchHit> {
        let mut u = Url::parse(&self.html_url).ok()?;
        if u.scheme() != "https" || u.host_str() != Some("github.com") {
            return None;
        }
        u.set_fragment(None);
        let mut details = Vec::new();
        if let Some(d) = self.description.map(|v| norm(&v)).filter(|v| !v.is_empty()) {
            details.push(d);
        }
        let mut meta = Vec::new();
        if let Some(l) = self.language.filter(|v| !v.trim().is_empty()) {
            meta.push(l);
        }
        meta.push(format!("{} stars", self.stargazers_count));
        details.push(meta.join(" · "));
        Some(SearchHit {
            title: self.full_name,
            url: u.to_string(),
            snippet: Some(details.join(" — ")),
        })
    }
}
fn norm(v: &str) -> String {
    v.split_whitespace().collect::<Vec<_>>().join(" ")
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
        let p=br#"{"items":[{"full_name":"seanmonstar/reqwest","html_url":"https://github.com/seanmonstar/reqwest#x","description":"HTTP client","language":"Rust","stargazers_count":100}]}"#;
        let r: Response = serde_json::from_slice(p).unwrap();
        let h = r.items.into_iter().next().unwrap().hit().unwrap();
        assert_eq!(h.url, "https://github.com/seanmonstar/reqwest");
        assert_eq!(h.snippet.as_deref(), Some("HTTP client — Rust · 100 stars"));
    }
    #[tokio::test]
    #[ignore = "queries GitHub's public repository search API"]
    async fn live_search_returns_public_results() {
        let o = GitHub::new()
            .unwrap()
            .search(&SearchQuery::new("rust http client", 3))
            .await
            .unwrap();
        assert!(!o.hits.is_empty());
    }
}
