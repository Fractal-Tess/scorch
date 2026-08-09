use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("target URL is not allowed: {0}")]
    UnsafeUrl(String),
    #[error("DNS resolution failed: {0}")]
    Dns(String),
    #[error("request timed out")]
    Timeout,
    #[error("remote response exceeded the {0} byte limit")]
    ResponseTooLarge(usize),
    #[error("remote request failed: {0}")]
    Fetch(String),
    #[error("browser is unavailable: {0}")]
    Browser(String),
    #[error("unsupported content type: {0}")]
    UnsupportedContent(String),
    #[error("content extraction failed: {0}")]
    Extraction(String),
    #[error("search failed: {0}")]
    Search(String),
    #[error("runtime capacity is exhausted: {0}")]
    Capacity(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("crawl job was not found")]
    JobNotFound,
}

impl EngineError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest(_) => "invalid_request",
            Self::UnsafeUrl(_) => "unsafe_url",
            Self::Dns(_) => "dns_error",
            Self::Timeout => "timeout",
            Self::ResponseTooLarge(_) => "response_too_large",
            Self::Fetch(_) => "fetch_error",
            Self::Browser(_) => "browser_error",
            Self::UnsupportedContent(_) => "unsupported_content",
            Self::Extraction(_) => "extraction_error",
            Self::Search(_) => "search_error",
            Self::Capacity(_) => "capacity_exhausted",
            Self::NotFound(_) => "not_found",
            Self::JobNotFound => "job_not_found",
        }
    }
}

pub type Result<T> = std::result::Result<T, EngineError>;
