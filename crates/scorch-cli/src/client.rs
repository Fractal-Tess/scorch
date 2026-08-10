use std::time::Duration;

use anyhow::{Context, bail};
use futures_util::StreamExt;
use reqwest::Method;
use scorch_types::{
    CrawlJobSummary, CrawlPage, CrawlRequest, DeleteResponse, ErrorResponse, MapRequest,
    MapResponse, ScrapeDocument, ScrapeRequest, SearchRequest, SearchResponse,
};
use serde::{Serialize, de::DeserializeOwned};
use uuid::Uuid;

const MAX_API_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone)]
pub struct ApiClient {
    base_url: String,
    client: reqwest::Client,
}

impl ApiClient {
    pub fn new(base_url: &str) -> anyhow::Result<Self> {
        let base_url = base_url.trim_end_matches('/');
        let parsed = reqwest::Url::parse(base_url).context("invalid API URL")?;
        if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
            bail!("API URL must use HTTP or HTTPS and include a host");
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            bail!("credentials in the API URL are not supported");
        }
        if parsed.query().is_some() || parsed.fragment().is_some() {
            bail!("API URL cannot contain a query or fragment");
        }
        Ok(Self {
            base_url: base_url.into(),
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(135))
                .build()?,
        })
    }

    pub async fn scrape(&self, request: &ScrapeRequest) -> anyhow::Result<ScrapeDocument> {
        self.json(Method::POST, "/v1/scrape", Some(request)).await
    }

    pub async fn search(&self, request: &SearchRequest) -> anyhow::Result<SearchResponse> {
        self.json(Method::POST, "/v1/search", Some(request)).await
    }

    pub async fn map(&self, request: &MapRequest) -> anyhow::Result<MapResponse> {
        self.json(Method::POST, "/v1/map", Some(request)).await
    }

    pub async fn start_crawl(&self, request: &CrawlRequest) -> anyhow::Result<CrawlJobSummary> {
        self.json(Method::POST, "/v1/crawls", Some(request)).await
    }

    pub async fn crawl_status(
        &self,
        id: Uuid,
        cursor: usize,
        page_size: usize,
    ) -> anyhow::Result<CrawlPage> {
        self.json::<(), _>(
            Method::GET,
            &format!("/v1/crawls/{id}?cursor={cursor}&pageSize={page_size}"),
            None,
        )
        .await
    }

    pub async fn cancel_crawl(&self, id: Uuid) -> anyhow::Result<DeleteResponse> {
        self.json::<(), _>(Method::DELETE, &format!("/v1/crawls/{id}"), None)
            .await
    }

    async fn json<B: Serialize + ?Sized, R: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
    ) -> anyhow::Result<R> {
        let mut request = self
            .client
            .request(method, format!("{}{path}", self.base_url));
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await.context("API request failed")?;
        let status = response.status();
        let bytes = read_limited(response).await?;
        if !status.is_success() {
            if let Ok(error) = serde_json::from_slice::<ErrorResponse>(&bytes) {
                bail!("{}: {}", error.code, error.message);
            }
            bail!(
                "API returned HTTP {status}: {}",
                String::from_utf8_lossy(&bytes)
            );
        }
        serde_json::from_slice(&bytes).context("API returned invalid JSON")
    }
}

async fn read_limited(response: reqwest::Response) -> anyhow::Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_API_RESPONSE_BYTES as u64)
    {
        bail!("API response exceeds the {MAX_API_RESPONSE_BYTES} byte limit");
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("failed to read API response")?;
        if bytes.len().saturating_add(chunk.len()) > MAX_API_RESPONSE_BYTES {
            bail!("API response exceeds the {MAX_API_RESPONSE_BYTES} byte limit");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_api_origin() {
        assert!(ApiClient::new("http://127.0.0.1:33000").is_ok());
        assert!(ApiClient::new("file:///tmp/scorch.sock").is_err());
        assert!(ApiClient::new("https://user:secret@example.com").is_err());
        assert!(ApiClient::new("https://example.com?token=secret").is_err());
    }
}
