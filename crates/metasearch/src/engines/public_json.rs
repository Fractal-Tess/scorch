use std::time::{Duration, Instant};

use reqwest::{Client, header};
use serde::Deserialize;
use url::Url;

use crate::{BoxSearchFuture, EngineOutput, Error, Result, SearchEngine, SearchHit, SearchQuery};

use super::http::read_limited;

const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

pub struct MicrosoftLearn {
    client: Client,
}

pub struct Steam {
    client: Client,
}

fn client(engine: &'static str) -> Result<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(6))
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

macro_rules! engine_impls {
    ($($engine:ident => $name:literal),+ $(,)?) => {
        $(
            impl $engine {
                pub fn new() -> Result<Self> {
                    Ok(Self { client: client($name)? })
                }
            }

            impl SearchEngine for $engine {
                fn name(&self) -> &'static str {
                    $name
                }

                fn weight(&self) -> f64 {
                    0.7
                }

                fn search<'a>(&'a self, query: &'a SearchQuery) -> BoxSearchFuture<'a> {
                    Box::pin(self.execute(query))
                }
            }
        )+
    };
}

engine_impls!(MicrosoftLearn => "microsoft-learn", Steam => "steam");

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
    let body = read_limited(response, engine, MAX_RESPONSE_BYTES).await?;
    serde_json::from_slice(&body).map_err(|error| Error::Parse {
        engine,
        message: error.to_string(),
    })
}

impl MicrosoftLearn {
    async fn execute(&self, query: &SearchQuery) -> Result<EngineOutput> {
        let started = Instant::now();
        let mut url = Url::parse("https://learn.microsoft.com/api/search").expect("static URL");
        url.query_pairs_mut()
            .append_pair("search", query.query.trim())
            .append_pair("locale", "en-us")
            .append_pair("$top", &query.limit.min(20).to_string())
            .append_pair("$skip", "0")
            .append_pair("expandScope", "true")
            .append_pair("includeQuestion", "false");
        let payload: LearnResponse = fetch(&self.client, "microsoft-learn", url).await?;
        let hits = payload
            .results
            .into_iter()
            .filter_map(|item| hit(&item.title, &item.url, item.description.as_deref()))
            .take(query.limit)
            .collect();
        Ok(EngineOutput {
            engine: "microsoft-learn",
            hits,
            elapsed: started.elapsed(),
        })
    }
}

#[derive(Deserialize)]
struct LearnResponse {
    #[serde(default)]
    results: Vec<LearnItem>,
}

#[derive(Deserialize)]
struct LearnItem {
    title: String,
    url: String,
    description: Option<String>,
}

impl Steam {
    async fn execute(&self, query: &SearchQuery) -> Result<EngineOutput> {
        let started = Instant::now();
        let mut url =
            Url::parse("https://store.steampowered.com/api/storesearch/").expect("static URL");
        url.query_pairs_mut()
            .append_pair("term", query.query.trim())
            .append_pair("cc", "US")
            .append_pair("l", "en");
        let payload: SteamResponse = fetch(&self.client, "steam", url).await?;
        let hits = payload
            .items
            .into_iter()
            .filter_map(SteamItem::into_hit)
            .take(query.limit)
            .collect();
        Ok(EngineOutput {
            engine: "steam",
            hits,
            elapsed: started.elapsed(),
        })
    }
}

#[derive(Deserialize)]
struct SteamResponse {
    #[serde(default)]
    items: Vec<SteamItem>,
}

#[derive(Deserialize)]
struct SteamItem {
    name: String,
    id: u64,
    #[serde(default)]
    platforms: SteamPlatforms,
    metascore: Option<String>,
}

#[derive(Default, Deserialize)]
struct SteamPlatforms {
    #[serde(default)]
    windows: bool,
    #[serde(default)]
    mac: bool,
    #[serde(default)]
    linux: bool,
}

impl SteamItem {
    fn into_hit(self) -> Option<SearchHit> {
        let title = normalize(&self.name);
        if title.is_empty() || self.id == 0 {
            return None;
        }
        let mut details = Vec::new();
        let mut platforms = Vec::new();
        if self.platforms.windows {
            platforms.push("Windows");
        }
        if self.platforms.mac {
            platforms.push("macOS");
        }
        if self.platforms.linux {
            platforms.push("Linux");
        }
        if !platforms.is_empty() {
            details.push(format!("Platforms: {}", platforms.join(", ")));
        }
        if let Some(score) = self.metascore.filter(|score| !score.trim().is_empty()) {
            details.push(format!("Metascore: {}", normalize(&score)));
        }
        Some(SearchHit {
            title,
            url: format!("https://store.steampowered.com/app/{}/", self.id),
            snippet: (!details.is_empty()).then(|| details.join(" · ")),
        })
    }
}

fn hit(title: &str, raw_url: &str, snippet: Option<&str>) -> Option<SearchHit> {
    let title = normalize(title);
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
        snippet: snippet.map(normalize).filter(|value| !value.is_empty()),
    })
}

fn normalize(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
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
    fn builds_steam_result() {
        let hit = SteamItem {
            name: "Rust".into(),
            id: 252490,
            platforms: SteamPlatforms {
                windows: true,
                mac: true,
                linux: false,
            },
            metascore: Some("69".into()),
        }
        .into_hit()
        .unwrap();
        assert_eq!(hit.url, "https://store.steampowered.com/app/252490/");
        assert_eq!(
            hit.snippet.as_deref(),
            Some("Platforms: Windows, macOS · Metascore: 69")
        );
    }

    #[tokio::test]
    #[ignore = "queries public search APIs"]
    async fn live_searches_return_results() {
        assert!(
            !MicrosoftLearn::new()
                .unwrap()
                .search(&SearchQuery::new("Rust", 3))
                .await
                .unwrap()
                .hits
                .is_empty()
        );
        assert!(
            !Steam::new()
                .unwrap()
                .search(&SearchQuery::new("Rust", 3))
                .await
                .unwrap()
                .hits
                .is_empty()
        );
    }
}
