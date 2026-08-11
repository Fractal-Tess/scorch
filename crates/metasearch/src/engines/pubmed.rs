use super::http::read_limited;
use crate::{BoxSearchFuture, EngineOutput, Error, Result, SearchEngine, SearchHit, SearchQuery};
use reqwest::{Client, header};
use serde::Deserialize;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use url::Url;
const NAME: &str = "pubmed";
const MAX: usize = 2 * 1024 * 1024;
pub struct PubMed {
    client: Client,
}
impl PubMed {
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
        let mut u =
            Url::parse("https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esearch.fcgi").unwrap();
        u.query_pairs_mut()
            .append_pair("db", "pubmed")
            .append_pair("term", q.query.trim())
            .append_pair("retmode", "json")
            .append_pair("retmax", &q.limit.min(20).to_string());
        let search: Search = fetch(&self.client, u).await?;
        if search.esearchresult.idlist.is_empty() {
            return Ok(EngineOutput {
                engine: NAME,
                hits: Vec::new(),
                elapsed: started.elapsed(),
            });
        }
        let mut u =
            Url::parse("https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esummary.fcgi").unwrap();
        u.query_pairs_mut()
            .append_pair("db", "pubmed")
            .append_pair("id", &search.esearchresult.idlist.join(","))
            .append_pair("retmode", "json");
        let summary: Summary = fetch(&self.client, u).await?;
        let hits = summary
            .result
            .uids
            .iter()
            .filter_map(|id| summary.result.records.get(id).and_then(|r| r.hit(id)))
            .take(q.limit)
            .collect();
        Ok(EngineOutput {
            engine: NAME,
            hits,
            elapsed: started.elapsed(),
        })
    }
}
impl SearchEngine for PubMed {
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
async fn fetch<T: for<'de> Deserialize<'de>>(c: &Client, u: Url) -> Result<T> {
    let r = c
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
    serde_json::from_slice(&b).map_err(parseerr)
}
#[derive(Deserialize)]
struct Search {
    esearchresult: SearchResult,
}
#[derive(Deserialize)]
struct SearchResult {
    #[serde(default)]
    idlist: Vec<String>,
}
#[derive(Deserialize)]
struct Summary {
    result: SummaryResult,
}
#[derive(Deserialize)]
struct SummaryResult {
    #[serde(default)]
    uids: Vec<String>,
    #[serde(flatten)]
    records: HashMap<String, Record>,
}
#[derive(Deserialize)]
struct Record {
    title: String,
    source: Option<String>,
    pubdate: Option<String>,
    #[serde(default)]
    authors: Vec<Author>,
}
#[derive(Deserialize)]
struct Author {
    name: String,
}
impl Record {
    fn hit(&self, id: &str) -> Option<SearchHit> {
        if !id.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let title = norm(&self.title);
        if title.is_empty() {
            return None;
        }
        let authors = self
            .authors
            .iter()
            .take(3)
            .map(|a| norm(&a.name))
            .filter(|v| !v.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        let mut parts = Vec::new();
        if !authors.is_empty() {
            parts.push(authors);
        }
        if let Some(v) = self.source.as_deref().map(norm).filter(|v| !v.is_empty()) {
            parts.push(v);
        }
        if let Some(v) = self.pubdate.as_deref().map(norm).filter(|v| !v.is_empty()) {
            parts.push(v);
        }
        Some(SearchHit {
            title,
            url: format!("https://pubmed.ncbi.nlm.nih.gov/{id}/"),
            snippet: (!parts.is_empty()).then(|| parts.join(" — ")),
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
    fn parses_summary_records() {
        let p=br#"{"result":{"uids":["42"],"42":{"title":"Rust medicine","source":"Journal","pubdate":"2026","authors":[{"name":"Ada"}]}}}"#;
        let r: Summary = serde_json::from_slice(p).unwrap();
        let h = r.result.records["42"].hit("42").unwrap();
        assert_eq!(h.url, "https://pubmed.ncbi.nlm.nih.gov/42/");
        assert_eq!(h.snippet.as_deref(), Some("Ada — Journal — 2026"));
    }
    #[tokio::test]
    #[ignore = "queries the public PubMed E-utilities API"]
    async fn live_search_returns_public_results() {
        let o = PubMed::new()
            .unwrap()
            .search(&SearchQuery::new("rust programming language", 3))
            .await
            .unwrap();
        assert!(!o.hits.is_empty());
    }
}
