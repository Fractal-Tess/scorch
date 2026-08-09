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
use scorch_types::{
    CrawlCancelRequest, CrawlJobSummary, CrawlPage, CrawlRequest, CrawlStatusRequest,
    DeleteResponse, MapRequest, MapResponse, ScrapeDocument, ScrapeRequest, SearchRequest,
    SearchResponse,
};
use tokio_util::sync::CancellationToken;

use crate::client::ApiClient;

#[derive(Clone)]
pub struct ScorchMcp {
    client: ApiClient,
    tool_router: ToolRouter<Self>,
}

#[tool_router(router = tool_router)]
impl ScorchMcp {
    pub fn new(client: ApiClient) -> Self {
        Self {
            client,
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
            result = self.client.search(&request) => result,
            () = cancellation.cancelled() => return Err(tool_error("search cancelled")),
        };
        result.map(Json).map_err(client_error)
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
            result = self.client.scrape(&request) => result,
            () = cancellation.cancelled() => return Err(tool_error("scrape cancelled")),
        };
        result.map(Json).map_err(client_error)
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
            result = self.client.map(&request) => result,
            () = cancellation.cancelled() => return Err(tool_error("map cancelled")),
        };
        result.map(Json).map_err(client_error)
    }

    #[tool(
        name = "scorch_crawl_start",
        description = "Start a bounded in-memory crawl job"
    )]
    async fn crawl_start(
        &self,
        Parameters(request): Parameters<CrawlRequest>,
        cancellation: CancellationToken,
    ) -> Result<Json<CrawlJobSummary>, CallToolResult> {
        let result = tokio::select! {
            result = self.client.start_crawl(&request) => result,
            () = cancellation.cancelled() => return Err(tool_error("crawl start cancelled")),
        };
        result.map(Json).map_err(client_error)
    }

    #[tool(
        name = "scorch_crawl_status",
        description = "Read a page of crawl status and results"
    )]
    async fn crawl_status(
        &self,
        Parameters(request): Parameters<CrawlStatusRequest>,
        cancellation: CancellationToken,
    ) -> Result<Json<CrawlPage>, CallToolResult> {
        let result = tokio::select! {
            result = self.client.crawl_status(request.id, request.cursor, request.page_size) => result,
            () = cancellation.cancelled() => return Err(tool_error("crawl status cancelled")),
        };
        result.map(Json).map_err(client_error)
    }

    #[tool(
        name = "scorch_crawl_cancel",
        description = "Cancel and remove an in-memory crawl job"
    )]
    async fn crawl_cancel(
        &self,
        Parameters(request): Parameters<CrawlCancelRequest>,
        cancellation: CancellationToken,
    ) -> Result<Json<DeleteResponse>, CallToolResult> {
        let result = tokio::select! {
            result = self.client.cancel_crawl(request.id) => result,
            () = cancellation.cancelled() => return Err(tool_error("crawl cancellation cancelled")),
        };
        result.map(Json).map_err(client_error)
    }
}

#[tool_handler(router = self.tool_router, name = "scorch", instructions = "Use the configured Scorch HTTP API to search, scrape, map, and crawl the public web")]
impl ServerHandler for ScorchMcp {}

pub async fn run(client: ApiClient) -> anyhow::Result<()> {
    let service = ScorchMcp::new(client).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

fn client_error(error: anyhow::Error) -> CallToolResult {
    tool_error(error.to_string())
}

fn tool_error(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.into())])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_expected_tools() {
        let client = ApiClient::new("http://127.0.0.1:3000").unwrap();
        let server = ScorchMcp::new(client);
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
