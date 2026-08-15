use std::time::{Duration, Instant};

use reqwest::{
    Client, StatusCode,
    header::{
        ACCEPT, ACCEPT_ENCODING, ACCEPT_LANGUAGE, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue,
        REFERER,
    },
};
use scraper::{ElementRef, Html, Selector};
use url::{Url, form_urlencoded};

use crate::{BoxSearchFuture, EngineOutput, Error, Result, SearchEngine, SearchHit, SearchQuery};

use super::http::read_limited;

const NAME: &str = "duckduckgo";
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const ENDPOINT: &str = "https://html.duckduckgo.com/html/";

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
        //
        // The rest of the set describes a form submission on the no-JS page,
        // which is the only way this endpoint is reached by a browser. The
        // `Sec-Fetch-*` values are the ones a navigation from that page carries,
        // and the endpoint answers with `Referrer-Policy: origin`, so a browser's
        // next request quotes it as the referrer.
        let mut headers = HeaderMap::new();
        headers.insert(
            ACCEPT,
            HeaderValue::from_static(
                "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
            ),
        );
        headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.9"));
        headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
        headers.insert(REFERER, HeaderValue::from_static(ENDPOINT));
        for (name, value) in [
            ("sec-fetch-dest", "document"),
            ("sec-fetch-mode", "navigate"),
            ("sec-fetch-site", "same-origin"),
            ("sec-fetch-user", "?1"),
        ] {
            headers.insert(
                HeaderName::from_static(name),
                HeaderValue::from_static(value),
            );
        }
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(5))
            .default_headers(headers)
            // A real, current Firefox string. The previous value claimed to be
            // Chrome but omitted the `(KHTML, like Gecko)` token and the full
            // version, so it matched no browser that has ever shipped.
            .user_agent("Mozilla/5.0 (X11; Linux x86_64; rv:153.0) Gecko/20100101 Firefox/153.0")
            .build()
            .map_err(|error| Error::Request {
                engine: NAME,
                message: error.to_string(),
            })?;
        Ok(Self { client })
    }

    async fn execute(&self, query: &SearchQuery) -> Result<EngineOutput> {
        let started = Instant::now();
        let region = format!(
            "{}-{}",
            query.country.to_ascii_lowercase(),
            query.language.to_ascii_lowercase()
        );
        // This endpoint exists to serve browsers without JavaScript, which reach
        // it by submitting its form. That is a POST, and `b` is the empty
        // first-page marker the form carries. A GET with the same values in the
        // query string returns results too, but it is a shape no browser
        // produces here. Preferences such as safe search live in cookies rather
        // than in the form, so `kp` is sent as one.
        let body = form_urlencoded::Serializer::new(String::new())
            .append_pair("q", query.query.trim())
            .append_pair("b", "")
            .append_pair("kl", &region)
            .finish();
        let response = self
            .client
            .post(ENDPOINT)
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header("cookie", format!("kl={region}; kp=1"))
            .body(body)
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
    //
    // DuckDuckGo serves more than one challenge markup, so both known shapes
    // are matched.
    let challenge =
        Selector::parse(".anomaly-modal__modal, form#challenge-form").expect("valid selector");
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
    if block
        .value()
        .classes()
        .any(|class| class.eq_ignore_ascii_case("result--ad"))
    {
        return None;
    }
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
    let is_duckduckgo = parsed.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("duckduckgo.com")
            || host.eq_ignore_ascii_case("www.duckduckgo.com")
    });
    if is_duckduckgo && parsed.path() == "/y.js" {
        return None;
    }
    let mut url = if is_duckduckgo && parsed.path() == "/l/" {
        let destination = parsed
            .query_pairs()
            .find(|(key, _)| key == "uddg")
            .map(|(_, value)| value.into_owned())?;
        Url::parse(&destination).ok()?
    } else {
        parsed
    };
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
    fn sponsored_results_do_not_consume_organic_result_slots() {
        let html = r#"
          <div class="result result--ad">
            <a class="result__a" href="https://duckduckgo.com/y.js?ad_provider=bingv7aa&amp;u3=https%3A%2F%2Fexample.com">Sponsored</a>
          </div>
          <div class="result results_links">
            <a class="result__a" href="https://www.rust-lang.org/">Rust</a>
          </div>
        "#;

        let hits = parse(html).expect("result markup is not a challenge");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Rust");
    }

    #[test]
    fn cleans_only_duckduckgo_redirects() {
        assert!(
            clean_url(
                "https://duckduckgo.com/y.js?ad_provider=bingv7aa&u3=https%3A%2F%2Fexample.com"
            )
            .is_none()
        );
        assert_eq!(
            clean_url("https://duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fpage%23part")
                .as_deref(),
            Some("https://example.com/page")
        );
        assert_eq!(
            clean_url("https://example.test/?uddg=https%3A%2F%2Fexample.com%2F").as_deref(),
            Some("https://example.test/?uddg=https%3A%2F%2Fexample.com%2F")
        );
    }

    #[test]
    fn challenge_page_is_an_error_not_an_empty_result() {
        for html in [
            r#"<div class="anomaly-modal__mask"></div>
               <div class="anomaly-modal__modal"><h1>Verify</h1></div>"#,
            r#"<form id="challenge-form" action="/html/"><h1>Verify</h1></form>"#,
        ] {
            assert!(matches!(
                parse(html),
                Err(Error::RateLimited { engine: NAME })
            ));
        }
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
