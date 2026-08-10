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
        let mut tool_router = Self::tool_router();
        for route in tool_router.map.values_mut() {
            strip_nonstandard_integer_formats(std::sync::Arc::make_mut(
                &mut route.attr.input_schema,
            ));
            if let Some(schema) = &mut route.attr.output_schema {
                strip_nonstandard_integer_formats(std::sync::Arc::make_mut(schema));
            }
        }

        Self {
            client,
            tool_router,
        }
    }

    #[tool(
        name = "scorch_search",
        description = "Search the public web; omit scrapeOptions for compact result metadata, or set it only when page content is required"
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

fn strip_nonstandard_integer_formats(schema: &mut serde_json::Map<String, serde_json::Value>) {
    if schema
        .get("format")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|format| {
            matches!(
                format,
                "int8"
                    | "int16"
                    | "int32"
                    | "int64"
                    | "int128"
                    | "isize"
                    | "uint"
                    | "uint8"
                    | "uint16"
                    | "uint32"
                    | "uint64"
                    | "uint128"
                    | "usize"
            )
        })
    {
        schema.remove("format");
    }

    for value in schema.values_mut() {
        match value {
            serde_json::Value::Object(object) => strip_nonstandard_integer_formats(object),
            serde_json::Value::Array(values) => {
                for value in values {
                    if let serde_json::Value::Object(object) = value {
                        strip_nonstandard_integer_formats(object);
                    }
                }
            }
            _ => {}
        }
    }
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
        let client = ApiClient::new("http://127.0.0.1:33000").unwrap();
        let server = ScorchMcp::new(client);
        let tools = server.tool_router.list_all();
        let names: Vec<_> = tools.iter().map(|tool| tool.name.as_ref()).collect();
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

        for tool in tools {
            let schema = serde_json::to_string(&tool).unwrap();
            for format in [
                "int8", "int16", "int32", "int64", "int128", "isize", "uint", "uint8", "uint16",
                "uint32", "uint64", "uint128", "usize",
            ] {
                assert!(
                    !schema.contains(&format!("\"format\":\"{format}\"")),
                    "{} contains unsupported integer format {format}",
                    tool.name
                );
            }
        }
    }
}
