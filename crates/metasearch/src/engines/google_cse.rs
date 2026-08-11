use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use reqwest::Client;
use serde::Deserialize;
use tokio::sync::RwLock;
use url::Url;

use crate::{BoxSearchFuture, EngineOutput, Error, Result, SearchEngine, SearchHit, SearchQuery};

use super::http::read_limited;

const NAME: &str = "google-cse";
const CSE_ID: &str = "partner-pub-8993703457585266:4862972284";
const TOKEN_ENDPOINT: &str = "https://cse.google.com/cse/cse.js";
const SEARCH_ENDPOINT: &str = "https://cse.google.com/cse/element/v1";
const MAX_TOKEN_BYTES: usize = 512 * 1024;
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const TOKEN_TTL: Duration = Duration::from_secs(60 * 60);

pub struct GoogleCse {
    client: Client,
    token: Arc<RwLock<Option<CachedToken>>>,
}

impl GoogleCse {
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(5))
            .user_agent(concat!("Scorch/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| Error::Request {
                engine: NAME,
                message: error.to_string(),
            })?;
        Ok(Self {
            client,
            token: Arc::new(RwLock::new(None)),
        })
    }

    async fn execute(&self, query: &SearchQuery) -> Result<EngineOutput> {
        let started = Instant::now();
        let token = self.token().await?;
        let url = search_url(query, &token);
        let response = self
            .client
            .get(url)
            .header(reqwest::header::REFERER, "https://cse.google.com/")
            .header(reqwest::header::ACCEPT, "*/*")
            .header(reqwest::header::COOKIE, "CONSENT=YES+")
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
        let payload = parse_search_response(&bytes)?;
        if let Some(error) = payload.error {
            if error.code == Some(429) {
                return Err(Error::RateLimited { engine: NAME });
            }
            return Err(Error::Parse {
                engine: NAME,
                message: format!(
                    "upstream error: {}",
                    error.message.unwrap_or_else(|| "unknown error".into())
                ),
            });
        }
        let hits = payload
            .results
            .into_iter()
            .filter_map(GoogleCseItem::into_hit)
            .take(query.limit)
            .collect();
        Ok(EngineOutput {
            engine: NAME,
            hits,
            elapsed: started.elapsed(),
        })
    }

    async fn token(&self) -> Result<CseToken> {
        if let Some(token) = self.cached_token().await {
            return Ok(token);
        }
        let mut cache = self.token.write().await;
        if let Some(cached) = cache
            .as_ref()
            .filter(|cached| cached.created.elapsed() < TOKEN_TTL)
        {
            return Ok(cached.token.clone());
        }
        let mut url = Url::parse(TOKEN_ENDPOINT).expect("constant URL is valid");
        url.query_pairs_mut().append_pair("cx", CSE_ID);
        let response = self.client.get(url).send().await.map_err(request_error)?;
        if !response.status().is_success() {
            return Err(Error::HttpStatus {
                engine: NAME,
                status: response.status().as_u16(),
            });
        }
        let bytes = read_limited(response, NAME, MAX_TOKEN_BYTES).await?;
        let token = parse_token_response(&bytes)?;
        *cache = Some(CachedToken {
            token: token.clone(),
            created: Instant::now(),
        });
        Ok(token)
    }

    async fn cached_token(&self) -> Option<CseToken> {
        self.token
            .read()
            .await
            .as_ref()
            .filter(|cached| cached.created.elapsed() < TOKEN_TTL)
            .map(|cached| cached.token.clone())
    }
}

impl SearchEngine for GoogleCse {
    fn name(&self) -> &'static str {
        NAME
    }

    fn weight(&self) -> f64 {
        1.05
    }

    fn search<'a>(&'a self, query: &'a SearchQuery) -> BoxSearchFuture<'a> {
        Box::pin(self.execute(query))
    }
}

#[derive(Clone)]
struct CachedToken {
    token: CseToken,
    created: Instant,
}

#[derive(Clone, Deserialize)]
struct CseToken {
    #[serde(rename = "cse_token")]
    cse_token: String,
    #[serde(rename = "cselibVersion")]
    cselib_version: String,
    #[serde(default)]
    exp: Vec<String>,
}

#[derive(Deserialize)]
struct GoogleCseResponse {
    #[serde(default)]
    results: Vec<GoogleCseItem>,
    error: Option<GoogleCseError>,
}

#[derive(Deserialize)]
struct GoogleCseError {
    code: Option<u16>,
    message: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleCseItem {
    unescaped_url: Option<String>,
    title_no_formatting: Option<String>,
    content_no_formatting: Option<String>,
}

impl GoogleCseItem {
    fn into_hit(self) -> Option<SearchHit> {
        let mut url = Url::parse(self.unescaped_url.as_deref()?).ok()?;
        if !matches!(url.scheme(), "http" | "https") {
            return None;
        }
        url.set_fragment(None);
        let title = normalize_space(self.title_no_formatting.as_deref().unwrap_or_default());
        if title.is_empty() {
            return None;
        }
        Some(SearchHit {
            title,
            url: url.to_string(),
            snippet: self
                .content_no_formatting
                .map(|snippet| normalize_space(&snippet))
                .filter(|snippet| !snippet.is_empty()),
        })
    }
}

fn search_url(query: &SearchQuery, token: &CseToken) -> Url {
    let mut url = Url::parse(SEARCH_ENDPOINT).expect("constant URL is valid");
    let language = query.language.to_ascii_lowercase();
    let country = query.country.to_ascii_uppercase();
    let mut pairs = url.query_pairs_mut();
    pairs
        .append_pair("rsz", "filtered_cse")
        .append_pair("num", &query.limit.min(20).to_string())
        .append_pair("hl", &language)
        .append_pair("cselibv", &token.cselib_version)
        .append_pair("cx", CSE_ID)
        .append_pair("q", query.query.trim())
        .append_pair("safe", "active")
        .append_pair("cse_tok", &token.cse_token)
        .append_pair("callback", "_")
        .append_pair("rurl", "")
        .append_pair("searchtype", "")
        .append_pair("lr", &format!("lang_{language}"))
        .append_pair("gl", &country);
    if !token.exp.is_empty() {
        pairs.append_pair("exp", &token.exp.join(","));
    }
    drop(pairs);
    url
}

fn parse_token_response(bytes: &[u8]) -> Result<CseToken> {
    let text = std::str::from_utf8(bytes).map_err(parse_error)?;
    let marker = "})({";
    let start = text.rfind(marker).ok_or_else(|| Error::Parse {
        engine: NAME,
        message: "token response did not contain an initialization object".into(),
    })? + marker.len()
        - 1;
    let end = text.rfind("});").ok_or_else(|| Error::Parse {
        engine: NAME,
        message: "token response was not terminated".into(),
    })? + 1;
    if start >= end {
        return Err(Error::Parse {
            engine: NAME,
            message: "token response bounds were invalid".into(),
        });
    }
    let token: CseToken = serde_json::from_str(&text[start..end]).map_err(parse_error)?;
    if token.cse_token.trim().is_empty() || token.cselib_version.trim().is_empty() {
        return Err(Error::Parse {
            engine: NAME,
            message: "token response omitted required fields".into(),
        });
    }
    Ok(token)
}

fn parse_search_response(bytes: &[u8]) -> Result<GoogleCseResponse> {
    let text = std::str::from_utf8(bytes).map_err(parse_error)?;
    let trimmed = text.trim();
    let payload = trimmed
        .strip_prefix("/*O_o*/")
        .unwrap_or(trimmed)
        .trim_start()
        .strip_prefix("_(")
        .and_then(|value| value.strip_suffix(");"))
        .ok_or_else(|| Error::Parse {
            engine: NAME,
            message: "response was not the expected JSONP callback".into(),
        })?;
    serde_json::from_str(payload).map_err(parse_error)
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
    fn parses_token_initialization_object() {
        let response = br#"prefix})({
          "cse_token": "temporary-token",
          "cselibVersion": "version-id",
          "exp": ["cc", "experiment"]
        });"#;
        let token = parse_token_response(response).unwrap();
        assert_eq!(token.cse_token, "temporary-token");
        assert_eq!(token.cselib_version, "version-id");
        assert_eq!(token.exp, ["cc", "experiment"]);
    }

    #[test]
    fn parses_jsonp_results_and_discards_invalid_items() {
        let response = br#"/*O_o*/
        _({"results":[
          {"unescapedUrl":"https://www.rust-lang.org/#start","titleNoFormatting":" Rust  Programming ","contentNoFormatting":"A  systems language."},
          {"unescapedUrl":"javascript:alert(1)","titleNoFormatting":"Unsafe"},
          {"titleNoFormatting":"Missing URL"}
        ]});"#;
        let payload = parse_search_response(response).unwrap();
        let hits = payload
            .results
            .into_iter()
            .filter_map(GoogleCseItem::into_hit)
            .collect::<Vec<_>>();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Rust Programming");
        assert_eq!(hits[0].url, "https://www.rust-lang.org/");
        assert_eq!(hits[0].snippet.as_deref(), Some("A systems language."));
    }

    #[test]
    fn builds_localized_bounded_request() {
        let query = SearchQuery {
            query: "времето в София".into(),
            limit: 20,
            country: "bg".into(),
            language: "bg".into(),
        };
        let token = CseToken {
            cse_token: "token".into(),
            cselib_version: "version".into(),
            exp: vec!["cc".into()],
        };
        let url = search_url(&query, &token);
        let params = url
            .query_pairs()
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(params.get("num").map(|value| value.as_ref()), Some("20"));
        assert_eq!(params.get("gl").map(|value| value.as_ref()), Some("BG"));
        assert_eq!(
            params.get("lr").map(|value| value.as_ref()),
            Some("lang_bg")
        );
    }

    #[tokio::test]
    #[ignore = "queries Google's public Programmable Search Element"]
    async fn live_search_returns_public_results() {
        let output = GoogleCse::new()
            .unwrap()
            .search(&SearchQuery::new("Rust programming language", 3))
            .await
            .unwrap();
        assert!(!output.hits.is_empty());
    }
}
