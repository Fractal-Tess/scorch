use std::{fmt, future::Future, pin::Pin};

use crate::{EngineOutput, Result, SearchQuery};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EngineKind {
    Bing,
    Brave,
    BraveWeb,
    CratesIo,
    Crossref,
    DockerHub,
    DuckDuckGo,
    GitHub,
    Google,
    GoogleCse,
    HackerNews,
    HuggingFace,
    Mwmbl,
    Npm,
    Nvd,
    OpenAlex,
    OpenLibrary,
    PubMed,
    Wikidata,
    Wikipedia,
    Yahoo,
}

impl EngineKind {
    pub const ALL: [Self; 19] = [
        Self::Bing,
        Self::BraveWeb,
        Self::CratesIo,
        Self::Crossref,
        Self::DockerHub,
        Self::DuckDuckGo,
        Self::GitHub,
        Self::GoogleCse,
        Self::HackerNews,
        Self::HuggingFace,
        Self::Mwmbl,
        Self::Npm,
        Self::Nvd,
        Self::OpenAlex,
        Self::OpenLibrary,
        Self::PubMed,
        Self::Wikidata,
        Self::Wikipedia,
        Self::Yahoo,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bing => "bing",
            Self::Brave => "brave",
            Self::BraveWeb => "brave-web",
            Self::CratesIo => "crates-io",
            Self::Crossref => "crossref",
            Self::DockerHub => "docker-hub",
            Self::DuckDuckGo => "duckduckgo",
            Self::GitHub => "github",
            Self::Google => "google",
            Self::GoogleCse => "google-cse",
            Self::HackerNews => "hacker-news",
            Self::HuggingFace => "hugging-face",
            Self::Mwmbl => "mwmbl",
            Self::Npm => "npm",
            Self::Nvd => "nvd",
            Self::OpenAlex => "openalex",
            Self::OpenLibrary => "open-library",
            Self::PubMed => "pubmed",
            Self::Wikidata => "wikidata",
            Self::Wikipedia => "wikipedia",
            Self::Yahoo => "yahoo",
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
