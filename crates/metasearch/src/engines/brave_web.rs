use std::time::{Duration, Instant};

use reqwest::{Client, header};
use scraper::{ElementRef, Html, Selector};
use url::Url;

use crate::{BoxSearchFuture, EngineOutput, Error, Result, SearchEngine, SearchHit, SearchQuery};

use super::http::read_limited;

const NAME: &str = "brave-web";
const SEARCH_ENDPOINT: &str = "https://search.brave.com/search";
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

pub struct BraveWeb {
    client: Client,
}

impl BraveWeb {
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(5))
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
        let language = query.language.to_ascii_lowercase();
        let country = query.country.to_ascii_lowercase();
        let mut url = Url::parse(SEARCH_ENDPOINT).expect("constant URL is valid");
        url.query_pairs_mut()
            .append_pair("q", query.query.trim())
            .append_pair("source", "web");
        let response = self
            .client
            .get(url)
            .header(header::ACCEPT, "text/html,application/xhtml+xml")
            .header(
                header::ACCEPT_LANGUAGE,
                format!("{language}-{country},{language};q=0.9,en;q=0.5"),
            )
            .header(header::REFERER, "https://search.brave.com/")
            .header(
                header::COOKIE,
                format!(
                    "safesearch=moderate; useLocation=0; summarizer=0; country={country}; ui_lang={language}-{country}"
                ),
            )
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
        let html = std::str::from_utf8(&bytes).map_err(parse_error)?;
        if is_challenge(html) {
            return Err(Error::RateLimited { engine: NAME });
        }
        let hits = parse(html).into_iter().take(query.limit).collect();
        Ok(EngineOutput {
            engine: NAME,
            hits,
            elapsed: started.elapsed(),
        })
    }
}

impl SearchEngine for BraveWeb {
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

fn parse(html: &str) -> Vec<SearchHit> {
    let document = Html::parse_document(html);
    let result_selector = Selector::parse("div.snippet[data-type='web']").expect("valid selector");
    let link_selector = Selector::parse("a[href]").expect("valid selector");
    let title_selector = Selector::parse("div.title").expect("valid selector");
    let content_selector = Selector::parse("div.content").expect("valid selector");
    document
        .select(&result_selector)
        .filter_map(|result| build_hit(&result, &link_selector, &title_selector, &content_selector))
        .collect()
}

fn build_hit(
    result: &ElementRef<'_>,
    link_selector: &Selector,
    title_selector: &Selector,
    content_selector: &Selector,
) -> Option<SearchHit> {
    let link = result.select(link_selector).next()?;
    let mut url = Url::parse(link.value().attr("href")?).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    url.set_fragment(None);
    let title = result
        .select(title_selector)
        .next()
        .map(element_text)
        .filter(|title| !title.is_empty())?;
    let snippet = result
        .select(content_selector)
        .next()
        .map(element_text)
        .filter(|snippet| !snippet.is_empty());
    Some(SearchHit {
        title,
        url: url.to_string(),
        snippet,
    })
}

fn element_text(element: ElementRef<'_>) -> String {
    normalize_space(&element.text().collect::<String>())
}

fn normalize_space(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_challenge(html: &str) -> bool {
    let lowercase = html.to_ascii_lowercase();
    lowercase.contains("flagged as being suspicious")
        || lowercase.contains("scheduled a captcha")
        || lowercase.contains("challenge-form")
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
    fn parses_web_results_and_discards_non_web_urls() {
        let html = r#"
          <div class="snippet generated" data-type="web">
            <div class="result-content">
              <a href="https://www.rust-lang.org/#start">
                <div class="site-name-content">Rust</div>
                <div class="title search-snippet-title">The <strong>Rust</strong> Programming Language</div>
              </a>
              <div class="content result-description">A language empowering <strong>everyone</strong>.</div>
            </div>
          </div>
          <div class="snippet generated" data-type="web">
            <a href="javascript:alert(1)"><div class="title">Unsafe</div></a>
          </div>
        "#;
        let hits = parse(html);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "The Rust Programming Language");
        assert_eq!(hits[0].url, "https://www.rust-lang.org/");
        assert_eq!(
            hits[0].snippet.as_deref(),
            Some("A language empowering everyone.")
        );
    }

    #[test]
    fn detects_captcha_responses() {
        assert!(is_challenge(
            "Your request has been flagged as being suspicious and scheduled a captcha"
        ));
        assert!(!is_challenge(
            "<div class='snippet' data-type='web'>result</div>"
        ));
    }

    #[tokio::test]
    #[ignore = "queries the public Brave Search website"]
    async fn live_search_returns_public_results() {
        let output = BraveWeb::new()
            .unwrap()
            .search(&SearchQuery::new("Rust programming language", 3))
            .await
            .unwrap();
        assert!(!output.hits.is_empty());
    }
}
