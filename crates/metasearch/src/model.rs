use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub query: String,
    pub limit: usize,
    pub country: String,
    pub language: String,
}

impl SearchQuery {
    pub fn new(query: impl Into<String>, limit: usize) -> Self {
        Self {
            query: query.into(),
            limit,
            country: "us".into(),
            language: "en".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EngineOutput {
    pub engine: &'static str,
    pub hits: Vec<SearchHit>,
    pub elapsed: Duration,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AggregatedHit {
    pub title: String,
    pub url: String,
    pub snippet: Option<String>,
    pub sources: Vec<String>,
    pub score: f64,
}

#[derive(Debug, Clone)]
pub struct MetaSearchOutput {
    pub hits: Vec<AggregatedHit>,
    pub engines_used: Vec<String>,
    pub engine_failures: Vec<String>,
    pub elapsed: Duration,
    pub cached: bool,
}
