use super::http::read_limited;
use crate::{BoxSearchFuture, EngineOutput, Error, Result, SearchEngine, SearchHit, SearchQuery};
use reqwest::{Client, header};
use serde::Deserialize;
use std::time::{Duration, Instant};
use url::Url;

const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

pub struct GitLab {
    client: Client,
}
pub struct Hex {
    client: Client,
}
pub struct Packagist {
    client: Client,
}
pub struct ManKier {
    client: Client,
}

fn client(engine: &'static str) -> Result<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(5))
        .user_agent(concat!(
            "Scorch/",
            env!("CARGO_PKG_VERSION"),
            " (https://github.com/Fractal-Tess/scorch)"
        ))
        .build()
        .map_err(|error| Error::Request {
            engine,
            message: error.to_string(),
        })
}

macro_rules! constructors { ($($ty:ident => $name:literal),+) => {$(
impl $ty { pub fn new() -> Result<Self> { Ok(Self { client: client($name)? }) } }
impl SearchEngine for $ty {
 fn name(&self) -> &'static str { $name }
 fn weight(&self) -> f64 { 0.8 }
 fn search<'a>(&'a self, query: &'a SearchQuery) -> BoxSearchFuture<'a> { Box::pin(self.execute(query)) }
}
)+}; }
constructors!(GitLab => "gitlab", Hex => "hex", Packagist => "packagist", ManKier => "mankier");

async fn fetch<T: for<'de> Deserialize<'de>>(
    client: &Client,
    engine: &'static str,
    url: Url,
) -> Result<T> {
    let response = client
        .get(url)
        .header(header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|error| request_error(error, engine))?;
    let status = response.status();
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(Error::RateLimited { engine });
    }
    if !status.is_success() {
        return Err(Error::HttpStatus {
            engine,
            status: status.as_u16(),
        });
    }
    let bytes = read_limited(response, engine, MAX_RESPONSE_BYTES).await?;
    serde_json::from_slice(&bytes).map_err(|error| Error::Parse {
        engine,
        message: error.to_string(),
    })
}

impl GitLab {
    async fn execute(&self, q: &SearchQuery) -> Result<EngineOutput> {
        let started = Instant::now();
        let mut u = Url::parse("https://gitlab.com/api/v4/projects").unwrap();
        u.query_pairs_mut()
            .append_pair("search", q.query.trim())
            .append_pair("per_page", &q.limit.min(20).to_string())
            .append_pair("simple", "true");
        let items: Vec<GitLabItem> = fetch(&self.client, "gitlab", u).await?;
        let hits = items
            .into_iter()
            .filter_map(|i| hit(&i.name_with_namespace, &i.web_url, i.description.as_deref()))
            .take(q.limit)
            .collect();
        Ok(EngineOutput {
            engine: "gitlab",
            hits,
            elapsed: started.elapsed(),
        })
    }
}
#[derive(Deserialize)]
struct GitLabItem {
    name_with_namespace: String,
    web_url: String,
    description: Option<String>,
}

impl Hex {
    async fn execute(&self, q: &SearchQuery) -> Result<EngineOutput> {
        let started = Instant::now();
        let mut u = Url::parse("https://hex.pm/api/packages").unwrap();
        u.query_pairs_mut()
            .append_pair("search", q.query.trim())
            .append_pair("page", "1")
            .append_pair("per_page", &q.limit.min(20).to_string());
        let items: Vec<HexItem> = fetch(&self.client, "hex", u).await?;
        let hits = items
            .into_iter()
            .filter_map(|i| {
                hit(
                    &format!(
                        "{} {}",
                        i.name,
                        i.latest_stable_version
                            .or(i.latest_version)
                            .unwrap_or_default()
                    ),
                    &i.html_url,
                    i.meta.description.as_deref(),
                )
            })
            .take(q.limit)
            .collect();
        Ok(EngineOutput {
            engine: "hex",
            hits,
            elapsed: started.elapsed(),
        })
    }
}
#[derive(Deserialize)]
struct HexItem {
    name: String,
    html_url: String,
    latest_version: Option<String>,
    latest_stable_version: Option<String>,
    meta: HexMeta,
}
#[derive(Deserialize)]
struct HexMeta {
    description: Option<String>,
}

impl Packagist {
    async fn execute(&self, q: &SearchQuery) -> Result<EngineOutput> {
        let started = Instant::now();
        let mut u = Url::parse("https://packagist.org/search.json").unwrap();
        u.query_pairs_mut()
            .append_pair("q", q.query.trim())
            .append_pair("per_page", &q.limit.min(20).to_string());
        let p: PackagistResponse = fetch(&self.client, "packagist", u).await?;
        let hits = p
            .results
            .into_iter()
            .filter_map(|i| hit(&i.name, &i.url, i.description.as_deref()))
            .take(q.limit)
            .collect();
        Ok(EngineOutput {
            engine: "packagist",
            hits,
            elapsed: started.elapsed(),
        })
    }
}
#[derive(Deserialize)]
struct PackagistResponse {
    #[serde(default)]
    results: Vec<PackagistItem>,
}
#[derive(Deserialize)]
struct PackagistItem {
    name: String,
    url: String,
    description: Option<String>,
}

impl ManKier {
    async fn execute(&self, q: &SearchQuery) -> Result<EngineOutput> {
        let started = Instant::now();
        let mut u = Url::parse("https://www.mankier.com/api/v2/mans/").unwrap();
        u.query_pairs_mut().append_pair("q", q.query.trim());
        let p: ManKierResponse = fetch(&self.client, "mankier", u).await?;
        let hits = p
            .results
            .into_iter()
            .filter_map(|i| {
                hit(
                    &format!("{}({})", i.name, i.section),
                    &i.url,
                    i.description.as_deref(),
                )
            })
            .take(q.limit)
            .collect();
        Ok(EngineOutput {
            engine: "mankier",
            hits,
            elapsed: started.elapsed(),
        })
    }
}
#[derive(Deserialize)]
struct ManKierResponse {
    #[serde(default)]
    results: Vec<ManKierItem>,
}
#[derive(Deserialize)]
struct ManKierItem {
    name: String,
    section: String,
    url: String,
    description: Option<String>,
}

fn hit(title: &str, raw_url: &str, snippet: Option<&str>) -> Option<SearchHit> {
    let title = norm(title);
    if title.is_empty() {
        return None;
    }
    let mut url = Url::parse(raw_url).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    url.set_fragment(None);
    Some(SearchHit {
        title,
        url: url.to_string(),
        snippet: snippet.map(norm).filter(|s| !s.is_empty()),
    })
}
fn norm(v: &str) -> String {
    v.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn request_error(error: reqwest::Error, engine: &'static str) -> Error {
    if error.is_timeout() {
        Error::Timeout { engine }
    } else {
        Error::Request {
            engine,
            message: error.without_url().to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sanitizes_hits() {
        let h = hit(
            " Test  Package ",
            "https://example.com/a#x",
            Some(" useful  package "),
        )
        .unwrap();
        assert_eq!(h.title, "Test Package");
        assert_eq!(h.url, "https://example.com/a");
        assert_eq!(h.snippet.as_deref(), Some("useful package"));
    }
    #[tokio::test]
    #[ignore = "queries public package and project APIs"]
    async fn live_searches_return_results() {
        let q = SearchQuery::new("http client", 3);
        assert!(
            !GitLab::new()
                .unwrap()
                .search(&q)
                .await
                .unwrap()
                .hits
                .is_empty()
        );
        assert!(
            !Hex::new()
                .unwrap()
                .search(&q)
                .await
                .unwrap()
                .hits
                .is_empty()
        );
        assert!(
            !Packagist::new()
                .unwrap()
                .search(&q)
                .await
                .unwrap()
                .hits
                .is_empty()
        );
        assert!(
            !ManKier::new()
                .unwrap()
                .search(&SearchQuery::new("curl", 3))
                .await
                .unwrap()
                .hits
                .is_empty()
        );
    }
}
