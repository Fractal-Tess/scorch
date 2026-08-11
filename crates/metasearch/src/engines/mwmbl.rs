use super::http::read_limited;
use crate::{BoxSearchFuture, EngineOutput, Error, Result, SearchEngine, SearchHit, SearchQuery};
use reqwest::{Client, header};
use serde::Deserialize;
use std::time::{Duration, Instant};
use url::Url;
const NAME: &str = "mwmbl";
const MAX: usize = 2 * 1024 * 1024;
pub struct Mwmbl {
    client: Client,
}
impl Mwmbl {
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
        let st = Instant::now();
        let mut u = Url::parse("https://api.mwmbl.org/search").unwrap();
        u.query_pairs_mut().append_pair("s", q.query.trim());
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
        let p: Vec<Item> = serde_json::from_slice(&b).map_err(parseerr)?;
        let hits = p.into_iter().filter_map(Item::hit).take(q.limit).collect();
        Ok(EngineOutput {
            engine: NAME,
            hits,
            elapsed: st.elapsed(),
        })
    }
}
impl SearchEngine for Mwmbl {
    fn name(&self) -> &'static str {
        NAME
    }
    fn search<'a>(&'a self, q: &'a SearchQuery) -> BoxSearchFuture<'a> {
        Box::pin(self.execute(q))
    }
}
#[derive(Deserialize)]
struct Item {
    url: String,
    #[serde(default)]
    title: Vec<Part>,
    #[serde(default)]
    extract: Vec<Part>,
}
#[derive(Deserialize)]
struct Part {
    value: String,
}
impl Item {
    fn hit(self) -> Option<SearchHit> {
        let mut u = Url::parse(&self.url).ok()?;
        if !matches!(u.scheme(), "http" | "https") || u.host_str().is_none() {
            return None;
        }
        u.set_fragment(None);
        let title = norm(&self.title.into_iter().map(|p| p.value).collect::<String>());
        if title.is_empty() {
            return None;
        }
        let snippet = norm(
            &self
                .extract
                .into_iter()
                .map(|p| p.value)
                .collect::<String>(),
        );
        Some(SearchHit {
            title,
            url: u.to_string(),
            snippet: (!snippet.is_empty()).then_some(snippet),
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
    fn parses_highlight_parts() {
        let p=br#"[{"url":"https://example.com/#x","title":[{"value":"Rust"},{"value":" Search"}],"extract":[{"value":"Useful index"}]}]"#;
        let r: Vec<Item> = serde_json::from_slice(p).unwrap();
        let h = r.into_iter().next().unwrap().hit().unwrap();
        assert_eq!(h.title, "Rust Search");
        assert_eq!(h.url, "https://example.com/");
    }
    #[tokio::test]
    #[ignore = "queries the public Mwmbl API"]
    async fn live_search_returns_public_results() {
        let o = Mwmbl::new()
            .unwrap()
            .search(&SearchQuery::new("Rust programming language", 3))
            .await
            .unwrap();
        assert!(!o.hits.is_empty());
    }
}
