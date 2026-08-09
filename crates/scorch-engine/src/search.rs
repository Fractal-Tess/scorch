use std::time::{Duration, Instant};

use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
use scorch_types::{SearchRequest, SearchResponse, SearchResult};
use scraper::{ElementRef, Html, Selector};
use url::Url;

use crate::{
    error::{EngineError, Result},
    fetch::SafeFetcher,
};

pub async fn search(fetcher: &SafeFetcher, request: &SearchRequest) -> Result<SearchResponse> {
    let started = Instant::now();
    let query = request.query.trim();
    if query.is_empty() {
        return Err(EngineError::InvalidRequest(
            "search query cannot be empty".into(),
        ));
    }
    if !(1..=20).contains(&request.limit) {
        return Err(EngineError::InvalidRequest(
            "search limit must be between 1 and 20".into(),
        ));
    }

    let mut provider_errors = Vec::new();
    for provider in [Provider::DuckDuckGo, Provider::Bing] {
        match provider.search(fetcher, request).await {
            Ok(mut results) if !results.is_empty() => {
                results.truncate(request.limit);
                return Ok(SearchResponse {
                    query: query.to_owned(),
                    provider: provider.name().into(),
                    results,
                    elapsed_ms: started.elapsed().as_millis() as u64,
                });
            }
            Ok(_) => provider_errors.push(format!("{} returned no results", provider.name())),
            Err(error) => provider_errors.push(format!("{}: {error}", provider.name())),
        }
    }

    Err(EngineError::Search(provider_errors.join("; ")))
}

#[derive(Clone, Copy)]
enum Provider {
    DuckDuckGo,
    Bing,
}

impl Provider {
    fn name(self) -> &'static str {
        match self {
            Self::DuckDuckGo => "duckduckgo",
            Self::Bing => "bing",
        }
    }

    async fn search(
        self,
        fetcher: &SafeFetcher,
        request: &SearchRequest,
    ) -> Result<Vec<SearchResult>> {
        let url = match self {
            Self::DuckDuckGo => duckduckgo_url(request),
            Self::Bing => bing_url(request),
        };
        let user_agent = match self {
            Self::DuckDuckGo => {
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"
            }
            Self::Bing => "Mozilla/5.0",
        };
        let response = fetcher
            .get_with_user_agent(url.as_str(), Duration::from_secs(15), user_agent)
            .await?;
        if !response.status.is_success() {
            return Err(EngineError::Search(format!("HTTP {}", response.status)));
        }
        let html = String::from_utf8_lossy(&response.body);
        Ok(match self {
            Self::DuckDuckGo => parse_duckduckgo(&html),
            Self::Bing => parse_bing(&html),
        })
    }
}

fn duckduckgo_url(request: &SearchRequest) -> Url {
    let mut url = Url::parse("https://html.duckduckgo.com/html/").expect("constant URL is valid");
    url.query_pairs_mut()
        .append_pair("q", request.query.trim())
        .append_pair(
            "kl",
            &format!(
                "{}-{}",
                request.country.to_ascii_lowercase(),
                request.language.to_ascii_lowercase()
            ),
        )
        .append_pair("kp", "1");
    url
}

fn bing_url(request: &SearchRequest) -> Url {
    let mut url = Url::parse("https://www.bing.com/search").expect("constant URL is valid");
    url.query_pairs_mut()
        .append_pair("q", request.query.trim())
        .append_pair("count", &request.limit.to_string());
    url
}

fn parse_duckduckgo(html: &str) -> Vec<SearchResult> {
    let document = Html::parse_document(html);
    if Selector::parse(".anomaly-modal__modal")
        .ok()
        .is_some_and(|selector| document.select(&selector).next().is_some())
    {
        return Vec::new();
    }
    let result_selector = Selector::parse(".result.web-result").expect("valid selector");
    let link_selector = Selector::parse(".result__a").expect("valid selector");
    let snippet_selector = Selector::parse(".result__snippet").expect("valid selector");

    document
        .select(&result_selector)
        .filter_map(|block| {
            build_result(
                &block,
                &link_selector,
                &snippet_selector,
                clean_duckduckgo_url,
            )
        })
        .enumerate()
        .map(numbered)
        .collect()
}

fn parse_bing(html: &str) -> Vec<SearchResult> {
    let document = Html::parse_document(html);
    let result_selector = Selector::parse("li.b_algo").expect("valid selector");
    let link_selector = Selector::parse("h2 a").expect("valid selector");
    let snippet_selector = Selector::parse(".b_caption p").expect("valid selector");

    document
        .select(&result_selector)
        .filter_map(|block| build_result(&block, &link_selector, &snippet_selector, clean_bing_url))
        .enumerate()
        .map(numbered)
        .collect()
}

fn build_result(
    block: &ElementRef<'_>,
    link_selector: &Selector,
    snippet_selector: &Selector,
    clean_url: impl Fn(&str) -> Option<String>,
) -> Option<(String, String, Option<String>)> {
    let link = block.select(link_selector).next()?;
    let url = clean_url(link.value().attr("href")?)?;
    let title = normalize_space(&link.text().collect::<Vec<_>>().join(" "));
    if title.is_empty() {
        return None;
    }
    let description = block
        .select(snippet_selector)
        .next()
        .map(|node| normalize_space(&node.text().collect::<Vec<_>>().join(" ")))
        .filter(|value| !value.is_empty());
    Some((title, url, description))
}

fn numbered(
    (index, (title, url, description)): (usize, (String, String, Option<String>)),
) -> SearchResult {
    SearchResult {
        position: index + 1,
        title,
        url,
        description,
        document: None,
        error: None,
    }
}

fn clean_duckduckgo_url(raw: &str) -> Option<String> {
    let parsed = Url::parse(raw)
        .or_else(|_| Url::parse("https://duckduckgo.com").and_then(|base| base.join(raw)))
        .ok()?;
    let candidate = parsed
        .query_pairs()
        .find(|(key, _)| key == "uddg")
        .map(|(_, value)| value.into_owned())
        .unwrap_or_else(|| parsed.to_string());
    public_url(&candidate)
}

fn clean_bing_url(raw: &str) -> Option<String> {
    let parsed = Url::parse(raw).ok()?;
    if parsed
        .host_str()
        .is_some_and(|host| host.ends_with("bing.com"))
        && let Some(encoded) = parsed
            .query_pairs()
            .find(|(key, _)| key == "u")
            .map(|(_, value)| value.into_owned())
            .and_then(|value| value.strip_prefix("a1").map(ToOwned::to_owned))
        && let Ok(decoded) = STANDARD_NO_PAD.decode(encoded)
        && let Ok(decoded) = String::from_utf8(decoded)
    {
        return public_url(&decoded);
    }
    public_url(raw)
}

fn public_url(value: &str) -> Option<String> {
    let url = Url::parse(value).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| url.to_string())
}

fn normalize_space(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_duckduckgo_result() {
        let html = r#"<div class="result web-result"><a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2F">Example</a><a class="result__snippet">A result</a></div>"#;
        let results = parse_duckduckgo(html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://example.com/");
    }

    #[test]
    fn parses_bing_redirect() {
        let html = r#"<li class="b_algo"><h2><a href="https://www.bing.com/ck/a?u=a1aHR0cHM6Ly9leGFtcGxlLmNvbS8">Example</a></h2><div class="b_caption"><p>A result</p></div></li>"#;
        let results = parse_bing(html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://example.com/");
    }
}
