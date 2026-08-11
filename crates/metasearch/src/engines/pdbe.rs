use std::time::{Duration, Instant};

use reqwest::{Client, header};
use serde::Deserialize;
use url::Url;

use crate::{BoxSearchFuture, EngineOutput, Error, Result, SearchEngine, SearchHit, SearchQuery};

use super::http::read_limited;

const ENGINE: &str = "pdbe";
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

pub struct Pdbe {
    client: Client,
}

impl Pdbe {
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(6))
            .user_agent(concat!(
                "Scorch/",
                env!("CARGO_PKG_VERSION"),
                " (https://github.com/Fractal-Tess/scorch)"
            ))
            .build()
            .map_err(|error| Error::Request {
                engine: ENGINE,
                message: error.to_string(),
            })?;
        Ok(Self { client })
    }

    async fn execute(&self, query: &SearchQuery) -> Result<EngineOutput> {
        let started = Instant::now();
        let rows = query.limit.min(20).to_string();
        let mut url =
            Url::parse("https://www.ebi.ac.uk/pdbe/search/pdb/select").expect("static URL");
        url.query_pairs_mut()
            .append_pair("q", query.query.trim())
            .append_pair("wt", "json")
            .append_pair("rows", &rows)
            .append_pair("fl", "pdb_id,title,abstracttext_unassigned");
        let response = self
            .client
            .get(url)
            .header(header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(request_error)?;
        let status = response.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(Error::RateLimited { engine: ENGINE });
        }
        if !status.is_success() {
            return Err(Error::HttpStatus {
                engine: ENGINE,
                status: status.as_u16(),
            });
        }

        let body = read_limited(response, ENGINE, MAX_RESPONSE_BYTES).await?;
        let payload: Payload = serde_json::from_slice(&body).map_err(|error| Error::Parse {
            engine: ENGINE,
            message: error.to_string(),
        })?;
        let hits = payload
            .response
            .docs
            .into_iter()
            .filter_map(Document::into_hit)
            .take(query.limit)
            .collect();
        Ok(EngineOutput {
            engine: ENGINE,
            hits,
            elapsed: started.elapsed(),
        })
    }
}

impl SearchEngine for Pdbe {
    fn name(&self) -> &'static str {
        ENGINE
    }

    fn weight(&self) -> f64 {
        0.7
    }

    fn search<'a>(&'a self, query: &'a SearchQuery) -> BoxSearchFuture<'a> {
        Box::pin(self.execute(query))
    }
}

#[derive(Deserialize)]
struct Payload {
    response: Response,
}

#[derive(Deserialize)]
struct Response {
    #[serde(default)]
    docs: Vec<Document>,
}

#[derive(Deserialize)]
struct Document {
    pdb_id: String,
    title: String,
    #[serde(default)]
    abstracttext_unassigned: Vec<String>,
}

impl Document {
    fn into_hit(self) -> Option<SearchHit> {
        let id = self.pdb_id.trim().to_ascii_lowercase();
        let title = normalize(&self.title);
        if id.is_empty()
            || !id
                .chars()
                .all(|character| character.is_ascii_alphanumeric())
            || title.is_empty()
        {
            return None;
        }
        let snippet = normalize(&self.abstracttext_unassigned.join(" "));
        Some(SearchHit {
            title: format!("{id}: {title}"),
            url: format!("https://www.ebi.ac.uk/pdbe/entry/pdb/{id}"),
            snippet: (!snippet.is_empty()).then_some(snippet),
        })
    }
}

fn normalize(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn request_error(error: reqwest::Error) -> Error {
    if error.is_timeout() {
        Error::Timeout { engine: ENGINE }
    } else {
        Error::Request {
            engine: ENGINE,
            message: error.without_url().to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_structure_result() {
        let document = Document {
            pdb_id: "7MQQ".into(),
            title: " Example structure ".into(),
            abstracttext_unassigned: vec!["A useful structure.".into()],
        };
        let hit = document.into_hit().unwrap();
        assert_eq!(hit.title, "7mqq: Example structure");
        assert_eq!(hit.url, "https://www.ebi.ac.uk/pdbe/entry/pdb/7mqq");
    }

    #[tokio::test]
    #[ignore = "queries the public PDBe API"]
    async fn live_search_returns_results() {
        let output = Pdbe::new()
            .unwrap()
            .search(&SearchQuery::new("rust", 3))
            .await
            .unwrap();
        assert!(!output.hits.is_empty());
    }
}
