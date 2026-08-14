use std::time::{Duration, Instant};

use reqwest::{
    Client, StatusCode,
    header::{ACCEPT, ACCEPT_ENCODING, ACCEPT_LANGUAGE, HeaderMap, HeaderValue},
};
use scraper::{ElementRef, Html, Selector};
use url::Url;

use crate::{BoxSearchFuture, EngineOutput, Error, Result, SearchEngine, SearchHit, SearchQuery};

use super::http::read_limited;

const NAME: &str = "duckduckgo";
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

pub struct DuckDuckGo {
    client: Client,
}

impl DuckDuckGo {
    pub fn new() -> Result<Self> {
        // DuckDuckGo serves its bot challenge to clients whose request headers do
        // not look like a browser's. A User-Agent on its own is not enough: with
        // only that header every request is answered with HTTP 202 and the
        // challenge page instead of results. Measured against the live endpoint,
        // `Accept-Encoding` plus either `Accept` or `Accept-Language` is what
        // flips it to HTTP 200 with results, so all three are sent.
        //
        // `identity` is deliberate. reqwest is built here without its `gzip`,
        // `brotli`, or `deflate` features, so it cannot decode a compressed body.
        // Advertising `gzip` does return HTTP 200, but the bytes stay compressed
        // and the parser quietly finds no results in them, which is the same
        // silent emptiness this header set exists to prevent.
        let mut headers = HeaderMap::new();
        headers.insert(
            ACCEPT,
            HeaderValue::from_static(
                "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
            ),
        );
        headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.9"));
        headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(5))
            .default_headers(headers)
            .user_agent(
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/131 Safari/537.36",
            )
            .build()
            .map_err(|error| Error::Request {
                engine: NAME,
                message: error.to_string(),
            })?;
        Ok(Self { client })
    }

    async fn execute(&self, query: &SearchQuery) -> Result<EngineOutput> {
        let started = Instant::now();
        let mut url =
            Url::parse("https://html.duckduckgo.com/html/").expect("constant URL is valid");
        url.query_pairs_mut()
            .append_pair("q", query.query.trim())
            .append_pair(
                "kl",
                &format!(
                    "{}-{}",
                    query.country.to_ascii_lowercase(),
                    query.language.to_ascii_lowercase()
                ),
            )
            .append_pair("kp", "1");
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|error| request_error(error, NAME))?;
        if !response.status().is_success() {
            return Err(Error::HttpStatus {
                engine: NAME,
                status: response.status().as_u16(),
            });
        }
        // The challenge is served as 202 Accepted, which `is_success` accepts, so
        // it has to be rejected on its own. Results always come back as 200.
        if response.status() == StatusCode::ACCEPTED {
            return Err(Error::RateLimited { engine: NAME });
        }
        let bytes = read_limited(response, NAME, MAX_RESPONSE_BYTES).await?;
        let html = String::from_utf8_lossy(&bytes);
        let mut hits = parse(&html)?;
        hits.truncate(query.limit);
        Ok(EngineOutput {
            engine: NAME,
            hits,
            elapsed: started.elapsed(),
        })
    }
}

impl SearchEngine for DuckDuckGo {
    fn name(&self) -> &'static str {
        NAME
    }

    fn weight(&self) -> f64 {
        0.95
    }

    fn search<'a>(&'a self, query: &'a SearchQuery) -> BoxSearchFuture<'a> {
        Box::pin(self.execute(query))
    }
}

fn parse(html: &str) -> Result<Vec<SearchHit>> {
    let document = Html::parse_document(html);
    // A challenge page is not an empty result set. Returning no hits here would
    // be indistinguishable from a query that genuinely matches nothing, and the
    // aggregator counts that as a successful search: the engine would keep its
    // clean health record, the breaker would never trip, the caller would see no
    // failure, and the empty answer would be cached for the rest of its TTL.
    let challenge = Selector::parse(".anomaly-modal__modal").expect("valid selector");
    if document.select(&challenge).next().is_some() {
        return Err(Error::RateLimited { engine: NAME });
    }
    let result_selector = Selector::parse(".result").expect("valid selector");
    let link_selector = Selector::parse(".result__a").expect("valid selector");
    let snippet_selector = Selector::parse(".result__snippet").expect("valid selector");
    Ok(document
        .select(&result_selector)
        .filter_map(|block| build_hit(&block, &link_selector, &snippet_selector))
        .collect())
}

fn build_hit(
    block: &ElementRef<'_>,
    link_selector: &Selector,
    snippet_selector: &Selector,
) -> Option<SearchHit> {
    let link = block.select(link_selector).next()?;
    let url = clean_url(link.value().attr("href")?)?;
    let title = normalize_space(&link.text().collect::<Vec<_>>().join(" "));
    if title.is_empty() {
        return None;
    }
    let snippet = block
        .select(snippet_selector)
        .next()
        .map(|node| normalize_space(&node.text().collect::<Vec<_>>().join(" ")))
        .filter(|value| !value.is_empty());
    Some(SearchHit {
        title,
        url,
        snippet,
    })
}

fn clean_url(raw: &str) -> Option<String> {
    let parsed = Url::parse(raw)
        .or_else(|_| Url::parse("https://duckduckgo.com").and_then(|base| base.join(raw)))
        .ok()?;
    let candidate = parsed
        .query_pairs()
        .find(|(key, _)| key == "uddg")
        .map(|(_, value)| value.into_owned())
        .unwrap_or_else(|| parsed.to_string());
    let mut url = Url::parse(&candidate).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    url.set_fragment(None);
    Some(url.to_string())
}

fn normalize_space(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn request_error(error: reqwest::Error, engine: &'static str) -> Error {
    if error.is_timeout() {
        Error::Timeout { engine }
    } else {
        Error::Request {
            engine,
            message: error.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_results_and_decodes_redirects() {
        let html = r#"
          <div class="result results_links">
            <h2><a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.rust-lang.org%2F&amp;rut=x">Rust</a></h2>
            <a class="result__snippet">A language empowering everyone.</a>
          </div>
        "#;
        let hits = parse(html).expect("result markup is not a challenge");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Rust");
        assert_eq!(hits[0].url, "https://www.rust-lang.org/");
        assert_eq!(
            hits[0].snippet.as_deref(),
            Some("A language empowering everyone.")
        );
    }

    #[test]
    fn challenge_page_is_an_error_not_an_empty_result() {
        let html = r#"<div class="anomaly-modal__mask"></div>
                      <div class="anomaly-modal__modal"><h1>Verify</h1></div>"#;
        assert!(matches!(
            parse(html),
            Err(Error::RateLimited { engine: NAME })
        ));
    }

    #[tokio::test]
    #[ignore = "queries the public DuckDuckGo service"]
    async fn live_search_returns_public_results() {
        let output = DuckDuckGo::new()
            .unwrap()
            .search(&SearchQuery::new("Rust programming language", 3))
            .await
            .unwrap();
        assert!(!output.hits.is_empty());
    }
}
