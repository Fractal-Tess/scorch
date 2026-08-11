use super::http::read_limited;
use crate::{BoxSearchFuture, EngineOutput, Error, Result, SearchEngine, SearchHit, SearchQuery};
use reqwest::{Client, header};
use scraper::Html;
use serde::Deserialize;
use std::time::{Duration, Instant};
use url::Url;
const NAME: &str = "stack-overflow";
const MAX: usize = 2 * 1024 * 1024;
pub struct StackOverflow {
    client: Client,
}
impl StackOverflow {
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
        let mut u = Url::parse("https://api.stackexchange.com/2.3/search/advanced").unwrap();
        u.query_pairs_mut()
            .append_pair("q", q.query.trim())
            .append_pair("pagesize", &q.limit.min(20).to_string())
            .append_pair("site", "stackoverflow")
            .append_pair("sort", "relevance")
            .append_pair("order", "desc")
            .append_pair("filter", "withbody");
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
        if p.backoff.is_some() {
            return Err(Error::RateLimited { engine: NAME });
        }
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
impl SearchEngine for StackOverflow {
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
struct Response {
    #[serde(default)]
    items: Vec<Item>,
    backoff: Option<u64>,
}
#[derive(Deserialize)]
struct Item {
    title: String,
    link: String,
    body: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    score: i64,
    is_answered: bool,
}
impl Item {
    fn hit(self) -> Option<SearchHit> {
        let title = clean(&self.title);
        if title.is_empty() {
            return None;
        }
        let mut u = Url::parse(&self.link).ok()?;
        if u.scheme() != "https" || u.host_str() != Some("stackoverflow.com") {
            return None;
        }
        u.set_fragment(None);
        let body = self.body.as_deref().map(clean).filter(|v| !v.is_empty());
        let meta = format!(
            "{} · score {} · {}",
            self.tags.into_iter().take(5).collect::<Vec<_>>().join(", "),
            self.score,
            if self.is_answered {
                "answered"
            } else {
                "unanswered"
            }
        );
        Some(SearchHit {
            title,
            url: u.to_string(),
            snippet: Some(body.map_or(meta.clone(), |b| format!("{b} — {meta}"))),
        })
    }
}
fn clean(v: &str) -> String {
    Html::parse_fragment(v)
        .root_element()
        .text()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
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
    fn parses_questions() {
        let p=br#"{"items":[{"title":"How to use &lt;Rust&gt;?","link":"https://stackoverflow.com/questions/1/test","body":"<p>Useful body</p>","tags":["rust"],"score":5,"is_answered":true}],"has_more":false}"#;
        let r: Response = serde_json::from_slice(p).unwrap();
        let h = r.items.into_iter().next().unwrap().hit().unwrap();
        assert_eq!(h.title, "How to use <Rust>?");
        assert!(h.snippet.unwrap().contains("answered"));
    }
    #[tokio::test]
    #[ignore = "queries the public Stack Exchange API"]
    async fn live_search_returns_public_results() {
        let o = StackOverflow::new()
            .unwrap()
            .search(&SearchQuery::new("rust async http", 3))
            .await
            .unwrap();
        assert!(!o.hits.is_empty());
    }
}
