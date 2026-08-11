use std::time::{Duration, Instant};

use reqwest::{Client, header};
use scraper::Html;
use serde::Deserialize;
use url::Url;

use crate::{BoxSearchFuture, EngineOutput, Error, Result, SearchEngine, SearchHit, SearchQuery};

use super::http::read_limited;

const NAME: &str = "crossref";
const ENDPOINT: &str = "https://api.crossref.org/works";
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

pub struct Crossref {
    client: Client,
}

impl Crossref {
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
            .map_err(|error| Error::Request {
                engine: NAME,
                message: error.to_string(),
            })?;
        Ok(Self { client })
    }

    async fn execute(&self, query: &SearchQuery) -> Result<EngineOutput> {
        let started = Instant::now();
        let mut url = Url::parse(ENDPOINT).expect("constant URL is valid");
        url.query_pairs_mut()
            .append_pair("query.bibliographic", query.query.trim())
            .append_pair("rows", &query.limit.min(20).to_string())
            .append_pair(
                "select",
                "DOI,title,URL,abstract,author,container-title,publisher,published",
            );
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
        let payload: CrossrefResponse = serde_json::from_slice(&bytes).map_err(parse_error)?;
        if payload.status != "ok" {
            return Err(Error::Parse {
                engine: NAME,
                message: "upstream response status was not ok".into(),
            });
        }
        let hits = payload
            .message
            .items
            .into_iter()
            .filter_map(CrossrefItem::into_hit)
            .take(query.limit)
            .collect();
        Ok(EngineOutput {
            engine: NAME,
            hits,
            elapsed: started.elapsed(),
        })
    }
}

impl SearchEngine for Crossref {
    fn name(&self) -> &'static str {
        NAME
    }

    fn weight(&self) -> f64 {
        0.9
    }

    fn search<'a>(&'a self, query: &'a SearchQuery) -> BoxSearchFuture<'a> {
        Box::pin(self.execute(query))
    }
}

#[derive(Deserialize)]
struct CrossrefResponse {
    status: String,
    message: CrossrefMessage,
}

#[derive(Deserialize)]
struct CrossrefMessage {
    #[serde(default)]
    items: Vec<CrossrefItem>,
}

#[derive(Deserialize)]
struct CrossrefItem {
    #[serde(default)]
    title: Vec<String>,
    #[serde(rename = "URL")]
    url: Option<String>,
    #[serde(rename = "DOI")]
    doi: Option<String>,
    #[serde(rename = "abstract")]
    abstract_text: Option<String>,
    #[serde(default)]
    author: Vec<CrossrefAuthor>,
    #[serde(rename = "container-title", default)]
    container_title: Vec<String>,
    publisher: Option<String>,
}

impl CrossrefItem {
    fn into_hit(self) -> Option<SearchHit> {
        let title = self
            .title
            .first()
            .map(|title| clean_text(title))
            .filter(|title| !title.is_empty())?;
        let url = self
            .url
            .as_deref()
            .and_then(public_url)
            .or_else(|| self.doi.as_deref().and_then(doi_url))?;
        let snippet = self
            .abstract_text
            .as_deref()
            .map(clean_text)
            .filter(|text| !text.is_empty())
            .or_else(|| metadata_snippet(&self));
        Some(SearchHit {
            title,
            url,
            snippet,
        })
    }
}

#[derive(Deserialize)]
struct CrossrefAuthor {
    given: Option<String>,
    family: Option<String>,
}

fn metadata_snippet(item: &CrossrefItem) -> Option<String> {
    let authors = item
        .author
        .iter()
        .take(3)
        .filter_map(|author| {
            let name = normalize_space(
                &[
                    author.given.as_deref().unwrap_or_default(),
                    author.family.as_deref().unwrap_or_default(),
                ]
                .join(" "),
            );
            (!name.is_empty()).then_some(name)
        })
        .collect::<Vec<_>>()
        .join(", ");
    let venue = item
        .container_title
        .first()
        .or(item.publisher.as_ref())
        .map(|value| clean_text(value))
        .filter(|value| !value.is_empty());
    match (authors.is_empty(), venue) {
        (false, Some(venue)) => Some(format!("{authors} — {venue}")),
        (false, None) => Some(authors),
        (true, Some(venue)) => Some(venue),
        (true, None) => None,
    }
}

fn doi_url(doi: &str) -> Option<String> {
    let doi = doi.trim();
    if doi.is_empty() {
        None
    } else {
        public_url(&format!("https://doi.org/{doi}"))
    }
}

fn public_url(raw: &str) -> Option<String> {
    let mut url = Url::parse(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    url.set_fragment(None);
    Some(url.to_string())
}

fn clean_text(value: &str) -> String {
    normalize_space(
        &Html::parse_fragment(value)
            .root_element()
            .text()
            .collect::<String>(),
    )
}

fn normalize_space(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn request_error(error: reqwest::Error) -> Error {
    if error.is_timeout() {
        Error::Timeout { engine: NAME }
    } else {
        Error::Request {
            engine: NAME,
            message: error.without_url().to_string(),
        }
    }
}

fn parse_error(error: impl std::fmt::Display) -> Error {
    Error::Parse {
        engine: NAME,
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_works_and_removes_abstract_markup() {
        let payload = br#"{
          "status":"ok",
          "message":{"items":[{
            "DOI":"10.1000/rust",
            "title":["Safe <i>Rust</i> Systems"],
            "URL":"https://doi.org/10.1000/rust#fragment",
            "abstract":"<jats:p>A <b>safe</b> systems paper.</jats:p>",
            "author":[{"given":"Ada","family":"Lovelace"}],
            "container-title":["Systems Journal"]
          }]}
        }"#;
        let response: CrossrefResponse = serde_json::from_slice(payload).unwrap();
        let hit = response
            .message
            .items
            .into_iter()
            .next()
            .unwrap()
            .into_hit()
            .unwrap();
        assert_eq!(hit.title, "Safe Rust Systems");
        assert_eq!(hit.url, "https://doi.org/10.1000/rust");
        assert_eq!(hit.snippet.as_deref(), Some("A safe systems paper."));
    }

    #[test]
    fn builds_metadata_snippets_when_abstracts_are_missing() {
        let payload = br#"{
          "status":"ok",
          "message":{"items":[{
            "DOI":"10.1000/rust",
            "title":["Rust"],
            "author":[{"given":"Ada","family":"Lovelace"}],
            "container-title":["Systems Journal"]
          }]}
        }"#;
        let response: CrossrefResponse = serde_json::from_slice(payload).unwrap();
        let hit = response
            .message
            .items
            .into_iter()
            .next()
            .unwrap()
            .into_hit()
            .unwrap();
        assert_eq!(hit.url, "https://doi.org/10.1000/rust");
        assert_eq!(
            hit.snippet.as_deref(),
            Some("Ada Lovelace — Systems Journal")
        );
    }

    #[tokio::test]
    #[ignore = "queries the public Crossref API"]
    async fn live_search_returns_public_results() {
        let output = Crossref::new()
            .unwrap()
            .search(&SearchQuery::new("Rust programming language", 3))
            .await
            .unwrap();
        assert!(!output.hits.is_empty());
    }
}
