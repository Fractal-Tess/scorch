use std::sync::Arc;

use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{
        router::tool::ToolRouter,
        wrapper::{Json, Parameters},
    },
    model::{CallToolResult, ContentBlock},
    tool, tool_handler, tool_router,
    transport::stdio,
};
use scorch_engine::{EngineError, ScorchEngine};
use scorch_types::{
    CrawlCancelRequest, CrawlJobSummary, CrawlPage, CrawlRequest, CrawlStatusRequest,
    DeleteResponse, MapRequest, MapResponse, ScrapeDocument, ScrapeRequest, SearchRequest,
    SearchResponse,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct ScorchMcp {
    engine: Arc<ScorchEngine>,
    tool_router: ToolRouter<Self>,
}

#[tool_router(router = tool_router)]
impl ScorchMcp {
    pub fn new(engine: Arc<ScorchEngine>) -> Self {
        Self {
            engine,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "scorch_search",
        description = "Search the public web and optionally scrape each result"
    )]
    async fn search(
        &self,
        Parameters(request): Parameters<SearchRequest>,
        cancellation: CancellationToken,
    ) -> Result<Json<SearchResponse>, CallToolResult> {
        let result = tokio::select! {
            result = self.engine.search(&request) => result,
            () = cancellation.cancelled() => return Err(tool_error("search cancelled")),
        };
        result.map(Json).map_err(engine_error)
    }

    #[tool(
        name = "scorch_scrape",
        description = "Fetch or render a public web page and extract clean content"
    )]
    async fn scrape(
        &self,
        Parameters(request): Parameters<ScrapeRequest>,
        cancellation: CancellationToken,
    ) -> Result<Json<ScrapeDocument>, CallToolResult> {
        let result = tokio::select! {
            result = self.engine.scrape(&request) => result,
            () = cancellation.cancelled() => return Err(tool_error("scrape cancelled")),
        };
        result.map(Json).map_err(engine_error)
    }

    #[tool(
        name = "scorch_map",
        description = "Discover public URLs belonging to a site"
    )]
    async fn map(
        &self,
        Parameters(request): Parameters<MapRequest>,
        cancellation: CancellationToken,
    ) -> Result<Json<MapResponse>, CallToolResult> {
        let result = tokio::select! {
            result = self.engine.map(&request) => result,
            () = cancellation.cancelled() => return Err(tool_error("map cancelled")),
        };
        result.map(Json).map_err(engine_error)
    }

    #[tool(
        name = "scorch_crawl_start",
        description = "Start a bounded in-memory crawl job"
    )]
    async fn crawl_start(
        &self,
        Parameters(request): Parameters<CrawlRequest>,
    ) -> Result<Json<CrawlJobSummary>, CallToolResult> {
        self.engine
            .start_crawl(request)
            .map(|job| Json(CrawlJobSummary::from(&job)))
            .map_err(engine_error)
    }

    #[tool(
        name = "scorch_crawl_status",
        description = "Read a page of crawl status and results"
    )]
    async fn crawl_status(
        &self,
        Parameters(request): Parameters<CrawlStatusRequest>,
    ) -> Result<Json<CrawlPage>, CallToolResult> {
        self.engine
            .crawl_page(&request)
            .map(Json)
            .map_err(engine_error)
    }

    #[tool(
        name = "scorch_crawl_cancel",
        description = "Cancel and remove an in-memory crawl job"
    )]
    async fn crawl_cancel(
        &self,
        Parameters(request): Parameters<CrawlCancelRequest>,
    ) -> Result<Json<DeleteResponse>, CallToolResult> {
        let deleted = self.engine.delete_crawl(request.id);
        if !deleted {
            return Err(engine_error(EngineError::JobNotFound));
        }
        Ok(Json(DeleteResponse {
            id: request.id,
            deleted,
        }))
    }
}

#[tool_handler(router = self.tool_router, name = "scorch", instructions = "Search, scrape, map, and crawl the public web with bounded local browser execution")]
impl ServerHandler for ScorchMcp {}

pub async fn run(engine: Arc<ScorchEngine>) -> anyhow::Result<()> {
    let service = ScorchMcp::new(engine).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

fn engine_error(error: EngineError) -> CallToolResult {
    tool_error(format!("{}: {error}", error.code()))
}

fn tool_error(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.into())])
}

#[cfg(test)]
mod tests {
    use super::*;
    use scorch_engine::EngineConfig;

    #[tokio::test]
    async fn exposes_expected_tools() {
        let engine = ScorchEngine::new(EngineConfig::default()).await.unwrap();
        let server = ScorchMcp::new(engine);
        let names: Vec<_> = server
            .tool_router
            .list_all()
            .into_iter()
            .map(|tool| tool.name.into_owned())
            .collect();
        assert_eq!(
            names,
            [
                "scorch_crawl_cancel",
                "scorch_crawl_start",
                "scorch_crawl_status",
                "scorch_map",
                "scorch_scrape",
                "scorch_search",
            ]
        );
    }
}
