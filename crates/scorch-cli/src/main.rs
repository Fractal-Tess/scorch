mod client;
mod mcp;

use std::time::Duration;

use clap::{Args, Parser, Subcommand, ValueEnum};
use client::ApiClient;
use scorch_types::{
    CrawlRequest, CrawlStatus, MapRequest, RenderMode, ScrapeFormat, ScrapeOptions, ScrapeRequest,
    SearchEngine, SearchRequest,
};
use uuid::Uuid;

#[derive(Parser)]
#[command(
    name = "scorch",
    version,
    about = "Command-line client for the Scorch HTTP API"
)]
struct Cli {
    #[arg(
        long,
        global = true,
        env = "SCORCH_API_URL",
        default_value = "http://127.0.0.1:33000"
    )]
    api_url: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Scrape(ScrapeArgs),
    Search(SearchArgs),
    Map(MapArgs),
    Crawl(CrawlArgs),
    CrawlStatus(CrawlStatusArgs),
    CrawlCancel { id: Uuid },
    Mcp,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum FormatArg {
    Markdown,
    Html,
    Text,
    Links,
    Metadata,
    Screenshot,
}

impl From<FormatArg> for ScrapeFormat {
    fn from(value: FormatArg) -> Self {
        match value {
            FormatArg::Markdown => Self::Markdown,
            FormatArg::Html => Self::Html,
            FormatArg::Text => Self::Text,
            FormatArg::Links => Self::Links,
            FormatArg::Metadata => Self::Metadata,
            FormatArg::Screenshot => Self::Screenshot,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum RenderArg {
    Auto,
    Always,
    Never,
}

impl From<RenderArg> for RenderMode {
    fn from(value: RenderArg) -> Self {
        match value {
            RenderArg::Auto => Self::Auto,
            RenderArg::Always => Self::Always,
            RenderArg::Never => Self::Never,
        }
    }
}

#[derive(Args)]
struct ScrapeArgs {
    url: String,
    #[arg(long, value_delimiter = ',', default_value = "markdown")]
    format: Vec<FormatArg>,
    #[arg(long, value_enum, default_value = "auto")]
    render: RenderArg,
    #[arg(long, default_value_t = 30_000)]
    timeout_ms: u64,
    #[arg(long, default_value_t = 0)]
    wait_for_ms: u64,
    #[arg(long)]
    full_content: bool,
    #[arg(long)]
    full_page_screenshot: bool,
}

impl ScrapeArgs {
    fn request(&self) -> ScrapeRequest {
        ScrapeRequest {
            url: self.url.clone(),
            options: ScrapeOptions {
                formats: self.format.iter().copied().map(Into::into).collect(),
                render: self.render.into(),
                timeout_ms: self.timeout_ms,
                wait_for_ms: self.wait_for_ms,
                only_main_content: !self.full_content,
                full_page_screenshot: self.full_page_screenshot,
                ..Default::default()
            },
        }
    }
}

#[derive(Args)]
struct SearchArgs {
    query: String,
    #[arg(long, default_value_t = 5)]
    limit: usize,
    #[arg(long)]
    scrape: bool,
    #[arg(long, default_value = "us")]
    country: String,
    #[arg(long, default_value = "en")]
    language: String,
    #[arg(long = "engine", value_enum, value_delimiter = ',')]
    engines: Vec<SearchEngineArg>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum SearchEngineArg {
    Bing,
    Brave,
    Duckduckgo,
    Google,
    Naver,
    Wikipedia,
}

impl From<SearchEngineArg> for SearchEngine {
    fn from(value: SearchEngineArg) -> Self {
        match value {
            SearchEngineArg::Bing => Self::Bing,
            SearchEngineArg::Brave => Self::Brave,
            SearchEngineArg::Duckduckgo => Self::DuckDuckGo,
            SearchEngineArg::Google => Self::Google,
            SearchEngineArg::Naver => Self::Naver,
            SearchEngineArg::Wikipedia => Self::Wikipedia,
        }
    }
}

#[derive(Args)]
struct MapArgs {
    url: String,
    #[arg(long, default_value_t = 100)]
    limit: usize,
    #[arg(long)]
    include_subdomains: bool,
}

#[derive(Args)]
struct CrawlArgs {
    url: String,
    #[arg(long, default_value_t = 20)]
    limit: usize,
    #[arg(long, default_value_t = 2)]
    max_depth: usize,
    #[arg(long, default_value_t = 4)]
    concurrency: usize,
    #[arg(long)]
    wait: bool,
}

#[derive(Args)]
struct CrawlStatusArgs {
    id: Uuid,
    #[arg(long, default_value_t = 0)]
    cursor: usize,
    #[arg(long, default_value_t = 10)]
    page_size: usize,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let client = ApiClient::new(&cli.api_url)?;
    match cli.command {
        Command::Mcp => mcp::run(client).await?,
        command => run_client(&client, command).await?,
    }
    Ok(())
}

async fn run_client(client: &ApiClient, command: Command) -> anyhow::Result<()> {
    let value = match command {
        Command::Scrape(args) => serde_json::to_value(client.scrape(&args.request()).await?)?,
        Command::Search(args) => serde_json::to_value(
            client
                .search(&SearchRequest {
                    query: args.query,
                    limit: args.limit,
                    scrape_options: args.scrape.then(ScrapeOptions::default),
                    country: args.country,
                    language: args.language,
                    engines: args.engines.into_iter().map(Into::into).collect(),
                })
                .await?,
        )?,
        Command::Map(args) => serde_json::to_value(
            client
                .map(&MapRequest {
                    url: args.url,
                    limit: args.limit,
                    include_subdomains: args.include_subdomains,
                    include_paths: Vec::new(),
                    exclude_paths: Vec::new(),
                })
                .await?,
        )?,
        Command::Crawl(args) => {
            let started = client
                .start_crawl(&CrawlRequest {
                    url: args.url,
                    limit: args.limit,
                    max_depth: args.max_depth,
                    concurrency: args.concurrency,
                    include_paths: Vec::new(),
                    exclude_paths: Vec::new(),
                    scrape_options: ScrapeOptions::default(),
                })
                .await?;
            if args.wait {
                loop {
                    let status = client.crawl_status(started.id, 0, 50).await?;
                    if matches!(
                        status.summary.status,
                        CrawlStatus::Completed | CrawlStatus::Cancelled | CrawlStatus::Failed
                    ) {
                        break serde_json::to_value(status)?;
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            } else {
                serde_json::to_value(started)?
            }
        }
        Command::CrawlStatus(args) => serde_json::to_value(
            client
                .crawl_status(args.id, args.cursor, args.page_size)
                .await?,
        )?,
        Command::CrawlCancel { id } => serde_json::to_value(client.cancel_crawl(id).await?)?,
        Command::Mcp => unreachable!(),
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removed_browser_flag_is_rejected() {
        assert!(
            Cli::try_parse_from([
                "scorch",
                "scrape",
                "https://example.com",
                "--browser",
                "obscura",
            ])
            .is_err()
        );
    }

    #[test]
    fn search_accepts_an_explicit_engine_subset() {
        let cli = Cli::try_parse_from([
            "scorch",
            "search",
            "Rust",
            "--engine",
            "bing,duckduckgo",
            "--engine",
            "wikipedia",
        ])
        .unwrap();
        let Command::Search(args) = cli.command else {
            panic!("expected search command");
        };
        assert_eq!(args.engines.len(), 3);
    }
}
