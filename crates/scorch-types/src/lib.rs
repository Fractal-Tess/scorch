use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ErrorResponse {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ScrapeFormat {
    Markdown,
    Html,
    Text,
    Links,
    Metadata,
    Screenshot,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum RenderMode {
    #[default]
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct ScrapeOptions {
    pub formats: Vec<ScrapeFormat>,
    pub render: RenderMode,
    pub timeout_ms: u64,
    pub wait_for_ms: u64,
    pub only_main_content: bool,
    pub block_media: bool,
    pub full_page_screenshot: bool,
}

impl Default for ScrapeOptions {
    fn default() -> Self {
        Self {
            formats: vec![ScrapeFormat::Markdown],
            render: RenderMode::Auto,
            timeout_ms: 30_000,
            wait_for_ms: 0,
            only_main_content: true,
            block_media: true,
            full_page_screenshot: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScrapeRequest {
    pub url: String,
    #[serde(default)]
    pub options: ScrapeOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ScrapeEngine {
    Fetch,
    Browser,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PageMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_url: Option<String>,
    pub status_code: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScrapeDocument {
    pub url: String,
    pub final_url: String,
    pub engine: ScrapeEngine,
    pub elapsed_ms: u64,
    pub metadata: PageMetadata,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub markdown: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub links: Option<Vec<Link>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screenshot: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SearchProvider {
    #[default]
    Bing,
    Metasearch,
    Naver,
    Wikipedia,
    Duckduckgo,
}

impl SearchProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bing => "bing",
            Self::Metasearch => "metasearch",
            Self::Naver => "naver",
            Self::Wikipedia => "wikipedia",
            Self::Duckduckgo => "duckduckgo",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchRequest {
    pub query: String,
    #[serde(default = "default_search_limit")]
    pub limit: usize,
    #[serde(default)]
    pub provider: Option<SearchProvider>,
    #[serde(default)]
    pub scrape_options: Option<ScrapeOptions>,
    #[serde(default = "default_country")]
    pub country: String,
    #[serde(default = "default_language")]
    pub language: String,
}

fn default_search_limit() -> usize {
    5
}

fn default_country() -> String {
    "us".into()
}

fn default_language() -> String {
    "en".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub position: usize,
    pub title: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document: Option<ScrapeDocument>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    pub query: String,
    pub provider: String,
    pub results: Vec<SearchResult>,
    pub elapsed_ms: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MapRequest {
    pub url: String,
    #[serde(default = "default_map_limit")]
    pub limit: usize,
    #[serde(default)]
    pub include_subdomains: bool,
    #[serde(default)]
    pub include_paths: Vec<String>,
    #[serde(default)]
    pub exclude_paths: Vec<String>,
}

fn default_map_limit() -> usize {
    100
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MapResponse {
    pub url: String,
    pub links: Vec<String>,
    pub elapsed_ms: u64,
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CrawlRequest {
    pub url: String,
    #[serde(default = "default_crawl_limit")]
    pub limit: usize,
    #[serde(default = "default_crawl_depth")]
    pub max_depth: usize,
    #[serde(default = "default_crawl_concurrency")]
    pub concurrency: usize,
    #[serde(default)]
    pub include_paths: Vec<String>,
    #[serde(default)]
    pub exclude_paths: Vec<String>,
    #[serde(default)]
    pub scrape_options: ScrapeOptions,
}

fn default_crawl_limit() -> usize {
    20
}

fn default_crawl_depth() -> usize {
    2
}

fn default_crawl_concurrency() -> usize {
    4
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum CrawlStatus {
    Queued,
    Running,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CrawlError {
    pub url: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CrawlJob {
    pub id: Uuid,
    pub status: CrawlStatus,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    pub total: usize,
    pub completed: usize,
    pub documents: Vec<ScrapeDocument>,
    pub errors: Vec<CrawlError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CrawlJobSummary {
    pub id: Uuid,
    pub status: CrawlStatus,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    pub total: usize,
    pub completed: usize,
    pub error_count: usize,
}

impl From<&CrawlJob> for CrawlJobSummary {
    fn from(job: &CrawlJob) -> Self {
        Self {
            id: job.id,
            status: job.status,
            created_at_ms: job.created_at_ms,
            expires_at_ms: job.expires_at_ms,
            total: job.total,
            completed: job.completed,
            error_count: job.errors.len(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CrawlStatusRequest {
    pub id: Uuid,
    #[serde(default)]
    pub cursor: usize,
    #[serde(default = "default_page_size")]
    pub page_size: usize,
}

fn default_page_size() -> usize {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CrawlPage {
    #[serde(flatten)]
    pub summary: CrawlJobSummary,
    pub cursor: usize,
    pub documents: Vec<ScrapeDocument>,
    pub errors: Vec<CrawlError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CrawlCancelRequest {
    pub id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeleteResponse {
    pub id: Uuid,
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReadinessResponse {
    pub status: String,
    pub browser_available: bool,
    pub browser_path: String,
    pub max_concurrency: usize,
    pub default_search_provider: String,
}
