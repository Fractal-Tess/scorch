use std::{
    collections::{HashSet, VecDeque},
    time::{Duration, Instant},
};

use quick_xml::{Reader, events::Event};
use scorch_types::{MapRequest, MapResponse};
use url::Url;

use crate::{
    error::{EngineError, Result},
    fetch::SafeFetcher,
};

const MAX_SITEMAPS: usize = 20;

pub async fn map(fetcher: &SafeFetcher, request: &MapRequest) -> Result<MapResponse> {
    let started = Instant::now();
    if !(1..=1_000).contains(&request.limit) {
        return Err(EngineError::InvalidRequest(
            "map limit must be between 1 and 1000".into(),
        ));
    }
    let root = Url::parse(&request.url)
        .map_err(|error| EngineError::InvalidRequest(format!("invalid map URL: {error}")))?;
    let mut links = Vec::new();
    let mut seen = HashSet::new();
    let mut sources = Vec::new();

    let robots_url = root
        .join("/robots.txt")
        .map_err(|error| EngineError::InvalidRequest(error.to_string()))?;
    let mut sitemap_queue = VecDeque::new();
    if let Ok(response) = fetcher
        .get(robots_url.as_str(), Duration::from_secs(10))
        .await
        && response.status.is_success()
    {
        let robots = String::from_utf8_lossy(&response.body);
        for line in robots.lines() {
            if let Some((key, value)) = line.split_once(':')
                && key.trim().eq_ignore_ascii_case("sitemap")
            {
                sitemap_queue.push_back(value.trim().to_owned());
            }
        }
        sources.push("robots.txt".into());
    }
    if sitemap_queue.is_empty() {
        sitemap_queue.push_back(
            root.join("/sitemap.xml")
                .map_err(|error| EngineError::InvalidRequest(error.to_string()))?
                .to_string(),
        );
    }

    let mut visited_sitemaps = HashSet::new();
    while let Some(sitemap_url) = sitemap_queue.pop_front() {
        if visited_sitemaps.len() >= MAX_SITEMAPS || links.len() >= request.limit {
            break;
        }
        if !visited_sitemaps.insert(sitemap_url.clone()) {
            continue;
        }
        let Ok(response) = fetcher.get(&sitemap_url, Duration::from_secs(15)).await else {
            continue;
        };
        if !response.status.is_success() {
            continue;
        }
        let xml = String::from_utf8_lossy(&response.body);
        let (urls, nested) = parse_sitemap(&xml);
        sources.push(sitemap_url);
        for nested in nested {
            sitemap_queue.push_back(nested);
        }
        for url in urls {
            if in_scope(&root, &url, request) && seen.insert(url.clone()) {
                links.push(url);
                if links.len() >= request.limit {
                    break;
                }
            }
        }
    }

    if links.len() < request.limit {
        let options = scorch_types::ScrapeOptions {
            formats: vec![scorch_types::ScrapeFormat::Links],
            render: scorch_types::RenderMode::Never,
            only_main_content: false,
            ..Default::default()
        };
        let response = fetcher
            .get(&request.url, Duration::from_millis(options.timeout_ms))
            .await?;
        if response.status.is_success() {
            let html = String::from_utf8_lossy(&response.body);
            for url in html_links(&html, &response.final_url) {
                if in_scope(&root, &url, request) && seen.insert(url.clone()) {
                    links.push(url);
                    if links.len() >= request.limit {
                        break;
                    }
                }
            }
            sources.push("root page".into());
        }
    }

    Ok(MapResponse {
        url: root.to_string(),
        links,
        elapsed_ms: started.elapsed().as_millis() as u64,
        sources,
    })
}

fn parse_sitemap(xml: &str) -> (Vec<String>, Vec<String>) {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let is_index = xml.contains("<sitemapindex") || xml.contains(":sitemapindex");
    let mut inside_loc = false;
    let mut locations = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(tag)) if tag.local_name().as_ref() == b"loc" => inside_loc = true,
            Ok(Event::Text(text)) if inside_loc => {
                if let Ok(value) = text.decode() {
                    locations.push(value.trim().to_owned());
                }
            }
            Ok(Event::End(tag)) if tag.local_name().as_ref() == b"loc" => inside_loc = false,
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    if is_index {
        (Vec::new(), locations)
    } else {
        (locations, Vec::new())
    }
}

fn html_links(html: &str, base: &str) -> Vec<String> {
    use scraper::{Html, Selector};
    let document = Html::parse_document(html);
    let Ok(selector) = Selector::parse("a[href]") else {
        return Vec::new();
    };
    let Ok(base) = Url::parse(base) else {
        return Vec::new();
    };
    document
        .select(&selector)
        .filter_map(|node| {
            let href = node.value().attr("href")?;
            let mut url = base.join(href).ok()?;
            if !matches!(url.scheme(), "http" | "https") {
                return None;
            }
            url.set_fragment(None);
            Some(url.to_string())
        })
        .collect()
}

pub fn in_scope(root: &Url, candidate: &str, request: &MapRequest) -> bool {
    let Ok(url) = Url::parse(candidate) else {
        return false;
    };
    let root_host = root.host_str().unwrap_or_default();
    let host = url.host_str().unwrap_or_default();
    let host_matches = host == root_host
        || (request.include_subdomains && host.ends_with(&format!(".{root_host}")));
    host_matches && path_allowed(url.path(), &request.include_paths, &request.exclude_paths)
}

pub fn path_allowed(path: &str, includes: &[String], excludes: &[String]) -> bool {
    (includes.is_empty() || includes.iter().any(|prefix| path.starts_with(prefix)))
        && !excludes.iter().any(|prefix| path.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_urlset_and_index() {
        let (urls, nested) =
            parse_sitemap("<urlset><url><loc>https://example.com/a</loc></url></urlset>");
        assert_eq!(urls, ["https://example.com/a"]);
        assert!(nested.is_empty());
        let (urls, nested) = parse_sitemap(
            "<sitemapindex><sitemap><loc>https://example.com/s.xml</loc></sitemap></sitemapindex>",
        );
        assert!(urls.is_empty());
        assert_eq!(nested, ["https://example.com/s.xml"]);
    }
}
