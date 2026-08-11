use super::http::read_limited;
use crate::{BoxSearchFuture, EngineOutput, Error, Result, SearchEngine, SearchHit, SearchQuery};
use reqwest::{Client, header};
use serde::Deserialize;
use std::time::{Duration, Instant};
use url::Url;
const NAME: &str = "wikidata";
const MAX: usize = 2 * 1024 * 1024;
pub struct Wikidata {
    client: Client,
}
impl Wikidata {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: Client::builder()
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
                })?,
        })
    }
    async fn execute(&self, q: &SearchQuery) -> Result<EngineOutput> {
        let started = Instant::now();
        let lang = q.language.to_ascii_lowercase();
        let mut u = Url::parse("https://www.wikidata.org/w/api.php").unwrap();
        u.query_pairs_mut()
            .append_pair("action", "wbsearchentities")
            .append_pair("format", "json")
            .append_pair("search", q.query.trim())
            .append_pair("language", &lang)
            .append_pair("uselang", &lang)
            .append_pair("limit", &q.limit.min(20).to_string());
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
            .search
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
impl SearchEngine for Wikidata {
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
    search: Vec<Item>,
}
#[derive(Deserialize)]
struct Item {
    id: String,
    label: String,
    description: Option<String>,
    concepturi: Option<String>,
}
impl Item {
    fn hit(self) -> Option<SearchHit> {
        if !self.id.starts_with('Q') && !self.id.starts_with('P') {
            return None;
        }
        let title = self.label.split_whitespace().collect::<Vec<_>>().join(" ");
        if title.is_empty() {
            return None;
        }
        let url = self
            .concepturi
            .as_deref()
            .and_then(public)
            .unwrap_or_else(|| format!("https://www.wikidata.org/wiki/{}", self.id));
        Some(SearchHit {
            title,
            url,
            snippet: self
                .description
                .map(|v| v.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|v| !v.is_empty()),
        })
    }
}
fn public(v: &str) -> Option<String> {
    let mut u = Url::parse(v).ok()?;
    if !matches!(u.scheme(), "http" | "https") || u.host_str().is_none() {
        return None;
    }
    u.set_fragment(None);
    Some(u.to_string())
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
    fn parses_entities() {
        let p=br#"{"search":[{"id":"Q575650","label":"Rust","description":"programming language","concepturi":"http://www.wikidata.org/entity/Q575650"}]}"#;
        let r: Response = serde_json::from_slice(p).unwrap();
        let h = r.search.into_iter().next().unwrap().hit().unwrap();
        assert_eq!(h.title, "Rust");
        assert_eq!(h.url, "http://www.wikidata.org/entity/Q575650");
    }
    #[tokio::test]
    #[ignore = "queries the public Wikidata API"]
    async fn live_search_returns_public_results() {
        let o = Wikidata::new()
            .unwrap()
            .search(&SearchQuery::new("Rust programming language", 3))
            .await
            .unwrap();
        assert!(!o.hits.is_empty());
    }
}
