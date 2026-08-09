mod aggregate;
mod engine;
pub mod engines;
mod error;
mod model;

pub use aggregate::{MetaSearch, MetaSearchConfig};
pub use engine::{BoxSearchFuture, SearchEngine};
pub use error::{Error, Result};
pub use model::{AggregatedHit, EngineOutput, MetaSearchOutput, SearchHit, SearchQuery};
