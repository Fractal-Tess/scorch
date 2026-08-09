use std::{future::Future, pin::Pin};

use crate::{EngineOutput, Result, SearchQuery};

pub type BoxSearchFuture<'a> = Pin<Box<dyn Future<Output = Result<EngineOutput>> + Send + 'a>>;

pub trait SearchEngine: Send + Sync {
    fn name(&self) -> &'static str;

    fn weight(&self) -> f64 {
        1.0
    }

    fn search<'a>(&'a self, query: &'a SearchQuery) -> BoxSearchFuture<'a>;
}
