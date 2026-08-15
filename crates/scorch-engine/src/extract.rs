use std::{
    borrow::Cow,
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use dom_smoothie::Readability;
use scorch_types::{Link, PageMetadata, ScrapeDocument, ScrapeEngine, ScrapeFormat, ScrapeOptions};
use scraper::{Html, Selector};
use url::Url;

use crate::{
    error::{EngineError, Result},
    fetch::FetchResponse,
};

#[derive(Clone)]
pub(super) struct ExtractInput {
    pub requested_url: String,
    pub html: Arc<str>,
    pub response: FetchResponse,
    pub engine: ScrapeEngine,
    pub cache_ttl: Option<std::time::Duration>,
    pub cache_observed_at: Option<std::time::Instant>,
}

pub(super) fn extract(input: ExtractInput, options: &ScrapeOptions) -> Result<ScrapeDocument> {
    let ExtractInput {
        requested_url,
        html,
        response,
        engine,
        cache_ttl: _,
        cache_observed_at: _,
    } = input;
    let document = Html::parse_document(&html);
    let final_url = Url::parse(&response.final_url).ok();
    let title = select_text(&document, "title");
    let description = select_attr(&document, "meta[name='description']", "content")
        .or_else(|| select_attr(&document, "meta[property='og:description']", "content"));
    let language = select_attr(&document, "html", "lang");
    let canonical_url = select_attr(&document, "link[rel='canonical']", "href")
        .and_then(|value| absolutize(final_url.as_ref(), &value));

    let wants = |format| options.formats.contains(&format);
    let wants_markdown = wants(ScrapeFormat::Markdown);
    let wants_html = wants(ScrapeFormat::Html);
    let wants_text = wants(ScrapeFormat::Text);
    let needs_content = wants_markdown || wants_html || wants_text;
    let (content_html, content_text, mut warnings) = if !needs_content {
        (Cow::Borrowed(""), None, Vec::new())
    } else if options.only_main_content {
        match readable_content(&html, &response.final_url) {
            Some((content, text)) => (Cow::Owned(content), wants_text.then_some(text), Vec::new()),
            None => (
                Cow::Borrowed(html.as_ref()),
                wants_text.then(|| root_text(&document)),
                vec!["main-content extraction fell back to the full document".into()],
            ),
        }
    } else {
        (
            Cow::Borrowed(html.as_ref()),
            wants_text.then(|| root_text(&document)),
            Vec::new(),
        )
    };

    let mut markdown = if wants_markdown {
        Some(
            htmd::convert(content_html.as_ref())
                .map_err(|error| EngineError::Extraction(error.to_string()))?,
        )
    } else {
        None
    };
    if let (Some(markdown), Some(title)) = (&mut markdown, &title) {
        let first_line = markdown.lines().next().unwrap_or_default();
        if !contains_ascii_case_insensitive(first_line, title) {
            *markdown = format!("# {title}\n\n{}", markdown.trim_start());
        }
    }
    if markdown
        .as_ref()
        .is_some_and(|value| value.trim().is_empty())
    {
        warnings.push("extracted Markdown is empty".into());
    }

    Ok(ScrapeDocument {
        url: requested_url,
        final_url: response.final_url,
        engine,
        elapsed_ms: 0,
        metadata: PageMetadata {
            title,
            description,
            language,
            canonical_url,
            status_code: response.status.as_u16(),
            content_type: response.content_type,
            headers: if wants(ScrapeFormat::Metadata) {
                response.headers
            } else {
                BTreeMap::new()
            },
        },
        markdown,
        html: wants_html.then(|| content_html.into_owned()),
        text: content_text,
        links: wants(ScrapeFormat::Links).then(|| extract_links(&document, final_url.as_ref())),
        warnings,
    })
}

fn readable_content(html: &str, url: &str) -> Option<(String, String)> {
    let mut readability = Readability::new(html, Some(url), None).ok()?;
    let article = readability.parse().ok()?;
    if article.text_content.trim().is_empty() {
        return None;
    }
    Some((
        article.content.to_string(),
        article.text_content.to_string(),
    ))
}

fn root_text(document: &Html) -> String {
    Selector::parse("body")
        .ok()
        .and_then(|selector| document.select(&selector).next())
        .map(|element| normalize_text(element.text()))
        .unwrap_or_default()
}

fn select_text(document: &Html, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    let value = normalize_text(document.select(&selector).next()?.text());
    (!value.is_empty()).then_some(value)
}

fn select_attr(document: &Html, selector: &str, attribute: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    document
        .select(&selector)
        .next()?
        .value()
        .attr(attribute)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn extract_links(document: &Html, base: Option<&Url>) -> Vec<Link> {
    let Ok(selector) = Selector::parse("a[href]") else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    document
        .select(&selector)
        .filter_map(|element| {
            let href = element.value().attr("href")?;
            let url = absolutize(base, href)?;
            let parsed = Url::parse(&url).ok()?;
            if !matches!(parsed.scheme(), "http" | "https") || !seen.insert(url.clone()) {
                return None;
            }
            let text = normalize_text(element.text());
            Some(Link {
                url,
                text: (!text.is_empty()).then_some(text),
            })
        })
        .collect()
}

fn absolutize(base: Option<&Url>, value: &str) -> Option<String> {
    Url::parse(value)
        .or_else(|_| {
            base.ok_or(url::ParseError::RelativeUrlWithoutBase)?
                .join(value)
        })
        .ok()
        .map(|mut url| {
            url.set_fragment(None);
            url.to_string()
        })
}

fn normalize_text<'a>(fragments: impl Iterator<Item = &'a str>) -> String {
    let mut normalized = String::new();
    for word in fragments.flat_map(str::split_whitespace) {
        if !normalized.is_empty() {
            normalized.push(' ');
        }
        normalized.push_str(word);
    }
    normalized
}

fn contains_ascii_case_insensitive(haystack: &str, needle: &str) -> bool {
    needle.is_empty()
        || haystack
            .as_bytes()
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response() -> FetchResponse {
        FetchResponse {
            final_url: "https://example.com/root".into(),
            status: reqwest::StatusCode::OK,
            content_type: Some("text/html".into()),
            headers: BTreeMap::new(),
            body: Vec::new(),
        }
    }

    #[test]
    fn metadata_only_skips_content_extraction() {
        let document = extract(
            ExtractInput {
                requested_url: "https://example.com".into(),
                html: "<html><title>Example</title><body></body></html>".into(),
                response: response(),
                engine: ScrapeEngine::Obscura,
                cache_ttl: Some(std::time::Duration::from_secs(60)),
                cache_observed_at: Some(std::time::Instant::now()),
            },
            &ScrapeOptions {
                formats: vec![ScrapeFormat::Metadata],
                ..Default::default()
            },
        )
        .unwrap();

        assert!(document.warnings.is_empty(), "{:?}", document.warnings);
    }

    #[test]
    fn resolves_and_deduplicates_links() {
        let document = Html::parse_document(
            r#"<a href="/a">A</a><a href="/a#fragment">Again</a><a href="mailto:x@y.z">Mail</a>"#,
        );
        let base = Url::parse("https://example.com/root").unwrap();
        let links = extract_links(&document, Some(&base));
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].url, "https://example.com/a");
    }
}
