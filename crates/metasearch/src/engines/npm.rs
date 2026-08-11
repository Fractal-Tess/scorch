use super::http::read_limited;
use crate::{BoxSearchFuture, EngineOutput, Error, Result, SearchEngine, SearchHit, SearchQuery};
use reqwest::{Client, header};
use serde::Deserialize;
use std::time::{Duration, Instant};
use url::Url;
const NAME: &str = "npm";
const ENDPOINT: &str = "https://registry.npmjs.org/-/v1/search";
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
pub struct Npm {
    client: Client,
}
impl Npm {
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(5))
            .user_agent(concat!("Scorch/", env!("CARGO_PKG_VERSION")))
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
            .append_pair("text", q.query.trim())
            .append_pair("size", &q.limit.min(20).to_string());
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
        let payload: NpmResponse = serde_json::from_slice(&bytes).map_err(parse_error)?;
        let hits = payload
            .objects
            .into_iter()
            .filter_map(NpmObject::into_hit)
            .take(q.limit)
            .collect();
        Ok(EngineOutput {
            engine: NAME,
            hits,
            elapsed: started.elapsed(),
        })
    }
}
impl SearchEngine for Npm {
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
struct NpmResponse {
    #[serde(default)]
    objects: Vec<NpmObject>,
}
#[derive(Deserialize)]
struct NpmObject {
    package: NpmPackage,
}
#[derive(Deserialize)]
struct NpmPackage {
    name: String,
    version: String,
    description: Option<String>,
    links: Option<NpmLinks>,
}
#[derive(Deserialize)]
struct NpmLinks {
    npm: Option<String>,
}
impl NpmObject {
    fn into_hit(self) -> Option<SearchHit> {
        let name = self.package.name.trim();
        if name.is_empty() {
            return None;
        }
        let fallback = format!("https://www.npmjs.com/package/{name}");
        let url = self
            .package
            .links
            .and_then(|l| l.npm)
            .as_deref()
            .and_then(public_url)
            .unwrap_or(fallback);
        let snippet = self
            .package
            .description
            .map(|v| normalize_space(&v))
            .filter(|v| !v.is_empty());
        Some(SearchHit {
            title: format!("{} {}", name, self.package.version.trim()),
            url,
            snippet,
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
    fn parses_packages() {
        let p=br#"{"objects":[{"package":{"name":"wasm-pack","version":"0.15.0","description":"Rust to wasm","links":{"npm":"https://www.npmjs.com/package/wasm-pack"}}}]}"#;
        let r: NpmResponse = serde_json::from_slice(p).unwrap();
        let h = r.objects.into_iter().next().unwrap().into_hit().unwrap();
        assert_eq!(h.title, "wasm-pack 0.15.0");
        assert_eq!(h.url, "https://www.npmjs.com/package/wasm-pack");
    }
    #[tokio::test]
    #[ignore = "queries the public npm registry"]
    async fn live_search_returns_public_results() {
        let o = Npm::new()
            .unwrap()
            .search(&SearchQuery::new("rust wasm", 3))
            .await
            .unwrap();
        assert!(!o.hits.is_empty());
    }
}
