use std::time::{Duration, Instant};

use reqwest::{Client, header};
use serde::Deserialize;
use url::Url;

use crate::{BoxSearchFuture, EngineOutput, Error, Result, SearchEngine, SearchHit, SearchQuery};

use super::http::read_limited;

const ENGINE: &str = "jisho";
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

pub struct Jisho {
    client: Client,
}

impl Jisho {
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
                engine: ENGINE,
                message: error.to_string(),
            })?;
        Ok(Self { client })
    }

    async fn execute(&self, query: &SearchQuery) -> Result<EngineOutput> {
        let started = Instant::now();
        let mut url = Url::parse("https://jisho.org/api/v1/search/words").expect("static URL");
        url.query_pairs_mut()
            .append_pair("keyword", query.query.trim());

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
        let payload: Response = serde_json::from_slice(&body).map_err(|error| Error::Parse {
            engine: ENGINE,
            message: error.to_string(),
        })?;
        if payload.meta.status == 429 {
            return Err(Error::RateLimited { engine: ENGINE });
        }
        if payload.meta.status != 200 {
            return Err(Error::Parse {
                engine: ENGINE,
                message: format!("unexpected API status {}", payload.meta.status),
            });
        }

        let hits = payload
            .data
            .into_iter()
            .filter_map(Entry::into_hit)
            .take(query.limit)
            .collect();
        Ok(EngineOutput {
            engine: ENGINE,
            hits,
            elapsed: started.elapsed(),
        })
    }
}

impl SearchEngine for Jisho {
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
struct Response {
    meta: Meta,
    #[serde(default)]
    data: Vec<Entry>,
}

#[derive(Deserialize)]
struct Meta {
    status: u16,
}

#[derive(Deserialize)]
struct Entry {
    slug: String,
    #[serde(default)]
    japanese: Vec<Japanese>,
    #[serde(default)]
    senses: Vec<Sense>,
}

#[derive(Deserialize)]
struct Japanese {
    word: Option<String>,
    reading: Option<String>,
}

#[derive(Deserialize)]
struct Sense {
    #[serde(default)]
    english_definitions: Vec<String>,
    #[serde(default)]
    parts_of_speech: Vec<String>,
}

impl Entry {
    fn into_hit(self) -> Option<SearchHit> {
        let forms = self
            .japanese
            .into_iter()
            .filter_map(|form| match (form.word, form.reading) {
                (Some(word), Some(reading)) if word != reading => {
                    Some(format!("{word} ({reading})"))
                }
                (Some(word), _) => Some(word),
                (None, Some(reading)) => Some(reading),
                (None, None) => None,
            })
            .filter(|form| !form.trim().is_empty())
            .collect::<Vec<_>>();
        let title = normalize(&forms.join(", "));
        if title.is_empty() || self.slug.trim().is_empty() {
            return None;
        }

        let definitions = self
            .senses
            .into_iter()
            .filter(|sense| {
                !sense
                    .parts_of_speech
                    .iter()
                    .any(|part| part == "Wikipedia definition")
            })
            .flat_map(|sense| sense.english_definitions)
            .filter(|definition| !definition.trim().is_empty())
            .take(8)
            .collect::<Vec<_>>();
        let snippet = normalize(&definitions.join("; "));
        let mut url = Url::parse("https://jisho.org/word/").expect("static URL");
        url.path_segments_mut()
            .expect("HTTP URL supports path segments")
            .pop_if_empty()
            .push(self.slug.trim());

        Some(SearchHit {
            title,
            url: url.to_string(),
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
    fn parses_dictionary_entry() {
        let payload = r#"{
            "meta":{"status":200},
            "data":[{
                "slug":"錆",
                "japanese":[{"word":"錆","reading":"さび"}],
                "senses":[{"english_definitions":["rust"],"parts_of_speech":["Noun"]}]
            }]
        }"#;
        let response: Response = serde_json::from_str(payload).unwrap();
        let hit = response
            .data
            .into_iter()
            .next()
            .unwrap()
            .into_hit()
            .unwrap();
        assert_eq!(hit.title, "錆 (さび)");
        assert_eq!(hit.snippet.as_deref(), Some("rust"));
    }

    #[tokio::test]
    #[ignore = "queries the public Jisho API"]
    async fn live_search_returns_results() {
        let output = Jisho::new()
            .unwrap()
            .search(&SearchQuery::new("rust", 3))
            .await
            .unwrap();
        assert!(!output.hits.is_empty());
    }
}
