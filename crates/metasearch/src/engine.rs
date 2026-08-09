use std::{future::Future, pin::Pin};

use crate::{EngineOutput, Result, SearchQuery};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EngineKind {
    Bing,
    Naver,
    Wikipedia,
}

impl EngineKind {
    pub const ALL: [Self; 3] = [Self::Bing, Self::Naver, Self::Wikipedia];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bing => "bing",
            Self::Naver => "naver",
            Self::Wikipedia => "wikipedia",
        }
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
