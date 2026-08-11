use super::http::read_limited;
use crate::{BoxSearchFuture, EngineOutput, Error, Result, SearchEngine, SearchHit, SearchQuery};
use reqwest::{Client, header};
use serde::Deserialize;
use std::time::{Duration, Instant};
use url::Url;
const NAME: &str = "openalex";
const ENDPOINT: &str = "https://api.openalex.org/works";
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
pub struct OpenAlex {
    client: Client,
}
impl OpenAlex {
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
        let mut u = Url::parse(ENDPOINT).unwrap();
        u.query_pairs_mut()
            .append_pair("search", q.query.trim())
            .append_pair("per-page", &q.limit.min(20).to_string())
            .append_pair(
                "select",
                "id,doi,display_name,publication_year,authorships,primary_location",
            );
        let r = self
            .client
            .get(u)
            .header(header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(request_error)?;
        let status = r.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(Error::RateLimited { engine: NAME });
        }
        if !status.is_success() {
            return Err(Error::HttpStatus {
                engine: NAME,
                status: status.as_u16(),
            });
        }
        let bytes = read_limited(r, NAME, MAX_RESPONSE_BYTES).await?;
        let p: OpenAlexResponse = serde_json::from_slice(&bytes).map_err(parse_error)?;
        let hits = p
            .results
            .into_iter()
            .filter_map(OpenAlexWork::into_hit)
            .take(q.limit)
            .collect();
        Ok(EngineOutput {
            engine: NAME,
            hits,
            elapsed: started.elapsed(),
        })
    }
}
impl SearchEngine for OpenAlex {
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
struct OpenAlexResponse {
    #[serde(default)]
    results: Vec<OpenAlexWork>,
}
#[derive(Deserialize)]
struct OpenAlexWork {
    id: String,
    doi: Option<String>,
    display_name: String,
    publication_year: Option<i32>,
    #[serde(default)]
    authorships: Vec<Authorship>,
    primary_location: Option<Location>,
}
#[derive(Deserialize)]
struct Authorship {
    author: Author,
}
#[derive(Deserialize)]
struct Author {
    display_name: String,
}
#[derive(Deserialize)]
struct Location {
    landing_page_url: Option<String>,
    source: Option<Source>,
}
#[derive(Deserialize)]
struct Source {
    display_name: String,
}
impl OpenAlexWork {
    fn into_hit(self) -> Option<SearchHit> {
        let title = norm(&self.display_name);
        if title.is_empty() {
            return None;
        }
        let url = self
            .primary_location
            .as_ref()
            .and_then(|l| l.landing_page_url.as_deref())
            .and_then(public_url)
            .or_else(|| self.doi.as_deref().and_then(public_url))
            .or_else(|| public_url(&self.id))?;
        let authors = self
            .authorships
            .iter()
            .take(3)
            .map(|a| norm(&a.author.display_name))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        let venue = self
            .primary_location
            .and_then(|l| l.source)
            .map(|s| norm(&s.display_name))
            .filter(|s| !s.is_empty());
        let mut parts = Vec::new();
        if !authors.is_empty() {
            parts.push(authors);
        }
        if let Some(y) = self.publication_year {
            parts.push(y.to_string());
        }
        if let Some(v) = venue {
            parts.push(v);
        }
        Some(SearchHit {
            title,
            url,
            snippet: (!parts.is_empty()).then(|| parts.join(" — ")),
        })
    }
}
fn public_url(raw: &str) -> Option<String> {
    let mut u = Url::parse(raw).ok()?;
    if !matches!(u.scheme(), "http" | "https") || u.host_str().is_none() {
        return None;
    }
    u.set_fragment(None);
    Some(u.to_string())
}
fn norm(v: &str) -> String {
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
    fn parses_works() {
        let p=br#"{"results":[{"id":"https://openalex.org/W1","doi":"https://doi.org/10.1/x","display_name":"RustBelt","publication_year":2017,"authorships":[{"author":{"display_name":"Ralf Jung"}}],"primary_location":{"landing_page_url":"https://doi.org/10.1/x","source":{"display_name":"PACMPL"}}}]}"#;
        let r: OpenAlexResponse = serde_json::from_slice(p).unwrap();
        let h = r.results.into_iter().next().unwrap().into_hit().unwrap();
        assert_eq!(h.title, "RustBelt");
        assert_eq!(h.snippet.as_deref(), Some("Ralf Jung — 2017 — PACMPL"));
    }
    #[tokio::test]
    #[ignore = "queries the public OpenAlex API"]
    async fn live_search_returns_public_results() {
        let o = OpenAlex::new()
            .unwrap()
            .search(&SearchQuery::new("Rust programming language", 3))
            .await
            .unwrap();
        assert!(!o.hits.is_empty());
    }
}
