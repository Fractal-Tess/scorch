#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid metasearch configuration: {0}")]
    InvalidConfiguration(String),
    #[error("invalid search query: {0}")]
    InvalidQuery(String),
    #[error("{engine} request timed out")]
    Timeout { engine: &'static str },
    #[error("{engine} request failed: {message}")]
    Request {
        engine: &'static str,
        message: String,
    },
    #[error("{engine} returned HTTP {status}")]
    HttpStatus { engine: &'static str, status: u16 },
    #[error("{engine} rate limit exceeded")]
    RateLimited { engine: &'static str },
    #[error("{engine} response exceeded {limit} bytes")]
    ResponseTooLarge { engine: &'static str, limit: usize },
    #[error("{engine} response could not be parsed: {message}")]
    Parse {
        engine: &'static str,
        message: String,
    },
    #[error("all metasearch engines failed: {0}")]
    AllEnginesFailed(String),
}

pub type Result<T> = std::result::Result<T, Error>;
