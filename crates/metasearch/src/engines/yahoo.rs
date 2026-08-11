use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use reqwest::{Client, redirect::Policy};
use scraper::{ElementRef, Html, Selector};
use url::Url;

use super::http::read_limited;
use crate::{BoxSearchFuture, EngineOutput, Error, Result, SearchEngine, SearchHit, SearchQuery};

const NAME: &str = "yahoo";
const ENDPOINT: &str = "https://search.yahoo.com/search";
const TIMEOUT: Duration = Duration::from_secs(6);
const RESPONSE_LIMIT: usize = 2 * 1024 * 1024;

pub struct Yahoo {
    client: Arc<Client>,
}

impl Yahoo {
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .redirect(Policy::limited(3))
            .connect_timeout(Duration::from_secs(3))
            .timeout(TIMEOUT)
            .no_proxy()
            .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/124 Safari/537.36 Scorch/0.5.0")
            .build()
            .map_err(|error| Error::Request { engine: NAME, message: error.to_string() })?;
        Ok(Self {
            client: Arc::new(client),
        })
    }

    async fn execute(&self, query: &SearchQuery) -> Result<EngineOutput> {
        let started = Instant::now();
        let mut url = Url::parse(ENDPOINT).expect("static URL");
        url.query_pairs_mut()
            .append_pair("p", query.query.trim())
            .append_pair("n", &query.limit.min(20).to_string())
            .append_pair("ei", "UTF-8")
            .append_pair("nojs", "1");
        let response = self.client.get(url).send().await.map_err(request_error)?;
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
        let body = read_limited(response, NAME, RESPONSE_LIMIT).await?;
        let html = String::from_utf8_lossy(&body);
        let lowercase = html.to_ascii_lowercase();
        if lowercase.contains("captcha") || lowercase.contains("verify you are human") {
            return Err(Error::RateLimited { engine: NAME });
        }
        let hits = parse(&html).into_iter().take(query.limit).collect();
        Ok(EngineOutput {
            engine: NAME,
            hits,
            elapsed: started.elapsed(),
        })
    }
}

impl SearchEngine for Yahoo {
    fn name(&self) -> &'static str {
        NAME
    }
    fn search<'a>(&'a self, query: &'a SearchQuery) -> BoxSearchFuture<'a> {
        Box::pin(self.execute(query))
    }
}

fn parse(html: &str) -> Vec<SearchHit> {
    let document = Html::parse_document(html);
    let result = Selector::parse("#web .algo-sr").expect("static selector");
    let link = Selector::parse(".compTitle > a[href]").expect("static selector");
    let title = Selector::parse("h3.title").expect("static selector");
    let snippet = Selector::parse(".compText p").expect("static selector");
    document
        .select(&result)
        .filter_map(|block| build_hit(&block, &link, &title, &snippet))
        .collect()
}

fn build_hit(
    block: &ElementRef<'_>,
    link: &Selector,
    title: &Selector,
    snippet: &Selector,
) -> Option<SearchHit> {
    let anchor = block.select(link).next()?;
    let url = clean_url(anchor.value().attr("href")?)?;
    let title = normalize(&block.select(title).next()?.text().collect::<String>());
    if title.is_empty() {
        return None;
    }
    let snippet = block
        .select(snippet)
        .next()
        .map(|node| normalize(&node.text().collect::<String>()))
        .filter(|text| !text.is_empty());
    Some(SearchHit {
        title,
        url,
        snippet,
    })
}

fn clean_url(raw: &str) -> Option<String> {
    let parsed = Url::parse(raw).ok()?;
    let candidate = if parsed
        .host_str()
        .is_some_and(|host| host.ends_with("search.yahoo.com"))
    {
        parsed
            .path()
            .split('/')
            .find_map(|part| part.strip_prefix("RU="))
            .and_then(percent_decode)?
    } else {
        raw.to_owned()
    };
    let mut url = Url::parse(&candidate).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    url.set_fragment(None);
    Some(url.to_string())
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3)?;
            let text = std::str::from_utf8(hex).ok()?;
            out.push(u8::from_str_radix(text, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}
fn normalize(value: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "queries public Yahoo search"]
    async fn live_search_returns_results() {
        let output = Yahoo::new()
            .unwrap()
            .search(&SearchQuery::new("Rust programming language", 3))
            .await
            .unwrap();
        assert!(!output.hits.is_empty());
    }
}
