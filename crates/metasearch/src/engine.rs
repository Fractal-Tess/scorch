use std::{fmt, future::Future, pin::Pin};

use crate::{EngineOutput, Result, SearchQuery};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EngineKind {
    Bing,
    Brave,
    DuckDuckGo,
    Google,
    Naver,
    Wikipedia,
}

impl EngineKind {
    pub const ALL: [Self; 4] = [Self::Bing, Self::DuckDuckGo, Self::Naver, Self::Wikipedia];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bing => "bing",
            Self::Brave => "brave",
            Self::DuckDuckGo => "duckduckgo",
            Self::Google => "google",
            Self::Naver => "naver",
            Self::Wikipedia => "wikipedia",
        }
    }
}

#[derive(Clone, Default)]
pub struct EngineCredentials {
    pub brave_api_key: Option<String>,
    pub google_api_key: Option<String>,
    pub google_search_engine_id: Option<String>,
}

impl fmt::Debug for EngineCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EngineCredentials")
            .field("brave_api_key_configured", &self.brave_api_key.is_some())
            .field("google_api_key_configured", &self.google_api_key.is_some())
            .field(
                "google_search_engine_id_configured",
                &self.google_search_engine_id.is_some(),
            )
            .finish()
    }
}

pub type BoxSearchFuture<'a> = Pin<Box<dyn Future<Output = Result<EngineOutput>> + Send + 'a>>;

pub trait SearchEngine: Send + Sync {
    fn name(&self) -> &'static str;

    fn weight(&self) -> f64 {
        1.0
    }

    fn search<'a>(&'a self, query: &'a SearchQuery) -> BoxSearchFuture<'a>;
}
