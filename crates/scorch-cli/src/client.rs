use anyhow::{Context, bail};
use reqwest::Method;
use scorch_types::{
    CrawlJobSummary, CrawlPage, CrawlRequest, DeleteResponse, ErrorResponse, MapRequest,
    MapResponse, ScrapeDocument, ScrapeRequest, SearchRequest, SearchResponse,
};
use serde::{Serialize, de::DeserializeOwned};
use uuid::Uuid;

#[derive(Clone)]
pub struct ApiClient {
    base_url: String,
    client: reqwest::Client,
}

impl ApiClient {
    pub fn new(base_url: &str) -> anyhow::Result<Self> {
        let base_url = base_url.trim_end_matches('/');
        reqwest::Url::parse(base_url).context("invalid API URL")?;
        Ok(Self {
            base_url: base_url.into(),
            client: reqwest::Client::builder().build()?,
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
        let bytes = response
            .bytes()
            .await
            .context("failed to read API response")?;
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
