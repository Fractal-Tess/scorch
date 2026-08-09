use std::collections::{BTreeMap, HashSet};

use dom_smoothie::Readability;
use scorch_types::{Link, PageMetadata, ScrapeDocument, ScrapeEngine, ScrapeFormat, ScrapeOptions};
use scraper::{Html, Selector};
use url::Url;

use crate::{error::Result, fetch::FetchResponse};

pub struct ExtractInput<'a> {
    pub requested_url: &'a str,
    pub html: String,
    pub response: &'a FetchResponse,
    pub engine: ScrapeEngine,
    pub elapsed_ms: u64,
    pub screenshot: Option<String>,
}

pub fn extract(input: ExtractInput<'_>, options: &ScrapeOptions) -> Result<ScrapeDocument> {
    let document = Html::parse_document(&input.html);
    let final_url = Url::parse(&input.response.final_url).ok();
    let title = select_text(&document, "title");
    let description = select_attr(&document, "meta[name='description']", "content")
        .or_else(|| select_attr(&document, "meta[property='og:description']", "content"));
    let language = select_attr(&document, "html", "lang");
    let canonical_url = select_attr(&document, "link[rel='canonical']", "href")
        .and_then(|value| absolutize(final_url.as_ref(), &value));
    let links = extract_links(&document, final_url.as_ref());

    let (content_html, content_text, mut warnings) = if options.only_main_content {
        match readable_content(&input.html, &input.response.final_url) {
            Some(content) => content,
            None => {
                let text = root_text(&document);
                (
                    input.html.clone(),
                    text,
                    vec!["main-content extraction fell back to the full document".into()],
                )
            }
        }
    } else {
        (input.html.clone(), root_text(&document), Vec::new())
    };

    let wants = |format| options.formats.contains(&format);
    let needs_markdown = wants(ScrapeFormat::Markdown);
    let mut markdown = needs_markdown.then(|| html2md::parse_html(&content_html));
    if let (Some(markdown), Some(title)) = (&mut markdown, &title) {
        let first_line = markdown.lines().next().unwrap_or_default();
        if !first_line
            .to_ascii_lowercase()
            .contains(&title.to_ascii_lowercase())
        {
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
        url: input.requested_url.to_owned(),
        final_url: input.response.final_url.clone(),
        engine: input.engine,
        elapsed_ms: input.elapsed_ms,
        metadata: PageMetadata {
            title,
            description,
            language,
            canonical_url,
            status_code: input.response.status.as_u16(),
            content_type: input.response.content_type.clone(),
            headers: if wants(ScrapeFormat::Metadata) {
                input.response.headers.clone()
            } else {
                BTreeMap::new()
            },
        },
        markdown,
        html: wants(ScrapeFormat::Html).then_some(content_html),
        text: wants(ScrapeFormat::Text).then_some(content_text),
        links: wants(ScrapeFormat::Links).then_some(links),
        screenshot: input.screenshot,
        warnings,
    })
}

fn readable_content(html: &str, url: &str) -> Option<(String, String, Vec<String>)> {
    let mut readability = Readability::new(html, Some(url), None).ok()?;
    let article = readability.parse().ok()?;
    if article.text_content.trim().is_empty() {
        return None;
    }
    Some((
        article.content.to_string(),
        article.text_content.to_string(),
        Vec::new(),
    ))
}

fn root_text(document: &Html) -> String {
    Selector::parse("body")
        .ok()
        .and_then(|selector| document.select(&selector).next())
        .map(|element| normalize_space(&element.text().collect::<Vec<_>>().join(" ")))
        .unwrap_or_default()
}

fn select_text(document: &Html, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    let value = document
        .select(&selector)
        .next()?
        .text()
        .collect::<Vec<_>>()
        .join(" ");
    let value = normalize_space(&value);
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
            let text = normalize_space(&element.text().collect::<Vec<_>>().join(" "));
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

fn normalize_space(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

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
