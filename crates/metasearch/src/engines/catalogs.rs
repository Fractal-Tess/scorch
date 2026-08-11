use super::http::read_limited;
use crate::{BoxSearchFuture, EngineOutput, Error, Result, SearchEngine, SearchHit, SearchQuery};
use reqwest::{Client, header};
use serde::Deserialize;
use std::time::{Duration, Instant};
use url::Url;
const MAX: usize = 4 * 1024 * 1024;
pub struct HuggingFace {
    client: Client,
}
pub struct Nvd {
    client: Client,
}
fn client(name: &'static str) -> Result<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(6))
        .user_agent(concat!(
            "Scorch/",
            env!("CARGO_PKG_VERSION"),
            " (https://github.com/Fractal-Tess/scorch)"
        ))
        .build()
        .map_err(|e| Error::Request {
            engine: name,
            message: e.to_string(),
        })
}
macro_rules! impls{($($t:ident=>$n:literal),+)=>{$(impl $t{pub fn new()->Result<Self>{Ok(Self{client:client($n)?})}}impl SearchEngine for $t{fn name(&self)->&'static str{$n}fn weight(&self)->f64{0.75}fn search<'a>(&'a self,q:&'a SearchQuery)->BoxSearchFuture<'a>{Box::pin(self.execute(q))}})+}}
impls!(HuggingFace=>"hugging-face",Nvd=>"nvd");
async fn fetch<T: for<'de> Deserialize<'de>>(c: &Client, n: &'static str, u: Url) -> Result<T> {
    let r = c
        .get(u)
        .header(header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| reqerr(e, n))?;
    let s = r.status();
    if s == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(Error::RateLimited { engine: n });
    }
    if !s.is_success() {
        return Err(Error::HttpStatus {
            engine: n,
            status: s.as_u16(),
        });
    }
    let b = read_limited(r, n, MAX).await?;
    serde_json::from_slice(&b).map_err(|e| Error::Parse {
        engine: n,
        message: e.to_string(),
    })
}
impl HuggingFace {
    async fn execute(&self, q: &SearchQuery) -> Result<EngineOutput> {
        let st = Instant::now();
        let mut u = Url::parse("https://huggingface.co/api/models").unwrap();
        u.query_pairs_mut()
            .append_pair("search", q.query.trim())
            .append_pair("limit", &q.limit.min(20).to_string())
            .append_pair("sort", "downloads")
            .append_pair("direction", "-1");
        let p: Vec<HfItem> = fetch(&self.client, "hugging-face", u).await?;
        let hits = p
            .into_iter()
            .filter_map(|i| {
                let id = i.id.trim();
                if id.is_empty() {
                    return None;
                }
                let tags = i
                    .tags
                    .into_iter()
                    .filter(|t| !t.contains(':'))
                    .take(5)
                    .collect::<Vec<_>>()
                    .join(", ");
                Some(SearchHit {
                    title: id.to_owned(),
                    url: format!("https://huggingface.co/{id}"),
                    snippet: Some(format!(
                        "{} downloads · {} likes{}",
                        i.downloads,
                        i.likes,
                        if tags.is_empty() {
                            String::new()
                        } else {
                            format!(" — {tags}")
                        }
                    )),
                })
            })
            .take(q.limit)
            .collect();
        Ok(EngineOutput {
            engine: "hugging-face",
            hits,
            elapsed: st.elapsed(),
        })
    }
}
#[derive(Deserialize)]
struct HfItem {
    id: String,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    likes: u64,
    #[serde(default)]
    tags: Vec<String>,
}
impl Nvd {
    async fn execute(&self, q: &SearchQuery) -> Result<EngineOutput> {
        let st = Instant::now();
        let mut u = Url::parse("https://services.nvd.nist.gov/rest/json/cves/2.0").unwrap();
        u.query_pairs_mut()
            .append_pair("keywordSearch", q.query.trim())
            .append_pair("resultsPerPage", &q.limit.min(20).to_string());
        let p: NvdResponse = fetch(&self.client, "nvd", u).await?;
        let hits = p
            .vulnerabilities
            .into_iter()
            .filter_map(|v| {
                let id = v.cve.id;
                let desc = v
                    .cve
                    .descriptions
                    .into_iter()
                    .find(|d| d.lang == "en")
                    .map(|d| norm(d.value))
                    .filter(|d| !d.is_empty());
                (!id.is_empty()).then(|| SearchHit {
                    title: id.clone(),
                    url: format!("https://nvd.nist.gov/vuln/detail/{id}"),
                    snippet: desc,
                })
            })
            .take(q.limit)
            .collect();
        Ok(EngineOutput {
            engine: "nvd",
            hits,
            elapsed: st.elapsed(),
        })
    }
}
#[derive(Deserialize)]
struct NvdResponse {
    #[serde(default)]
    vulnerabilities: Vec<Vulnerability>,
}
#[derive(Deserialize)]
struct Vulnerability {
    cve: Cve,
}
#[derive(Deserialize)]
struct Cve {
    id: String,
    #[serde(default)]
    descriptions: Vec<Description>,
}
#[derive(Deserialize)]
struct Description {
    lang: String,
    value: String,
}
fn norm(v: impl AsRef<str>) -> String {
    v.as_ref().split_whitespace().collect::<Vec<_>>().join(" ")
}
fn reqerr(e: reqwest::Error, n: &'static str) -> Error {
    if e.is_timeout() {
        Error::Timeout { engine: n }
    } else {
        Error::Request {
            engine: n,
            message: e.without_url().to_string(),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "queries public catalog APIs"]
    async fn live_searches_return_results() {
        assert!(
            !HuggingFace::new()
                .unwrap()
                .search(&SearchQuery::new("rust", 3))
                .await
                .unwrap()
                .hits
                .is_empty()
        );
        assert!(
            !Nvd::new()
                .unwrap()
                .search(&SearchQuery::new("rust", 3))
                .await
                .unwrap()
                .hits
                .is_empty()
        );
    }
}
