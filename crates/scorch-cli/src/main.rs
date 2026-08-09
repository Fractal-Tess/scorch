mod client;
mod mcp;

use std::{
    env,
    net::SocketAddr,
    path::PathBuf,
    time::{Duration, Instant},
};

use clap::{Args, Parser, Subcommand, ValueEnum};
use client::ApiClient;
use scorch_engine::{EngineConfig, ScorchEngine};
use scorch_types::{
    CrawlRequest, CrawlStatus, MapRequest, RenderMode, ScrapeFormat, ScrapeOptions, ScrapeRequest,
    SearchRequest,
};
use tracing::info;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

#[derive(Parser)]
#[command(version, about = "Self-contained web search, scraping, and crawling")]
struct Cli {
    #[arg(
        long,
        global = true,
        env = "SCORCH_API_URL",
        default_value = "http://127.0.0.1:3000"
    )]
    api_url: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Serve(ServeArgs),
    Scrape(ScrapeArgs),
    Search(SearchArgs),
    Map(MapArgs),
    Crawl(CrawlArgs),
    CrawlStatus(CrawlStatusArgs),
    CrawlCancel { id: Uuid },
    Mcp(EngineArgs),
    Benchmark(BenchmarkArgs),
}

#[derive(Args, Clone)]
struct EngineArgs {
    #[arg(long, env = "SCORCH_BROWSER_PATH", default_value = "chromium")]
    browser_path: PathBuf,
    #[arg(long, env = "SCORCH_MAX_CONCURRENCY", default_value_t = 4)]
    max_concurrency: usize,
    #[arg(long, env = "SCORCH_MAX_RESPONSE_BYTES", default_value_t = 5 * 1024 * 1024)]
    max_response_bytes: usize,
    #[arg(long, env = "SCORCH_JOB_TTL_SECS", default_value_t = 900)]
    job_ttl_secs: u64,
}

impl EngineArgs {
    fn config(&self) -> EngineConfig {
        EngineConfig {
            browser_path: self.browser_path.clone(),
            max_concurrency: self.max_concurrency.max(1),
            max_response_bytes: self.max_response_bytes.max(1024),
            job_ttl: Duration::from_secs(self.job_ttl_secs.max(1)),
            ..Default::default()
        }
    }
}

#[derive(Args)]
struct ServeArgs {
    #[arg(long, env = "SCORCH_BIND", default_value = "127.0.0.1:3000")]
    bind: SocketAddr,
    #[command(flatten)]
    engine: EngineArgs,
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

#[derive(Args)]
struct BenchmarkArgs {
    #[arg(required = true)]
    urls: Vec<String>,
    #[arg(long, default_value_t = 3)]
    runs: usize,
    #[command(flatten)]
    engine: EngineArgs,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let cli = Cli::parse();

    match cli.command {
        Command::Serve(args) => {
            info!(
                version = env!("CARGO_PKG_VERSION"),
                bind = %args.bind,
                "starting Scorch service"
            );
            let engine = ScorchEngine::new(args.engine.config()).await?;
            scorch_api::serve(args.bind, engine, shutdown_signal()).await?;
        }
        Command::Mcp(args) => {
            info!(
                version = env!("CARGO_PKG_VERSION"),
                "starting Scorch MCP server"
            );
            let engine = ScorchEngine::new(args.config()).await?;
            mcp::run(engine).await?;
        }
        Command::Benchmark(args) => {
            info!(
                version = env!("CARGO_PKG_VERSION"),
                "starting Scorch benchmark"
            );
            benchmark(args).await?
        }
        command => run_client(&cli.api_url, command).await?,
    }
    Ok(())
}

async fn run_client(api_url: &str, command: Command) -> anyhow::Result<()> {
    let client = ApiClient::new(api_url)?;
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
        Command::Serve(_) | Command::Mcp(_) | Command::Benchmark(_) => unreachable!(),
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

async fn benchmark(args: BenchmarkArgs) -> anyhow::Result<()> {
    if args.runs == 0 || args.runs > 20 {
        anyhow::bail!("runs must be between 1 and 20");
    }
    let engine = ScorchEngine::new(args.engine.config()).await?;
    let mut reports = Vec::new();
    for url in args.urls {
        for render in [RenderMode::Never, RenderMode::Always] {
            let mut timings = Vec::new();
            let mut bytes = 0;
            let mut failures = Vec::new();
            for _ in 0..args.runs {
                let request = ScrapeRequest {
                    url: url.clone(),
                    options: ScrapeOptions {
                        formats: vec![ScrapeFormat::Html],
                        render,
                        only_main_content: false,
                        ..Default::default()
                    },
                };
                let started = Instant::now();
                match engine.scrape(&request).await {
                    Ok(document) => {
                        timings.push(started.elapsed().as_millis() as u64);
                        bytes = document.html.map_or(0, |html| html.len());
                    }
                    Err(error) => failures.push(error.to_string()),
                }
            }
            timings.sort_unstable();
            reports.push(serde_json::json!({
                "url": url,
                "engine": if render == RenderMode::Never { "fetch" } else { "browser" },
                "runs": args.runs,
                "successfulRuns": timings.len(),
                "medianMs": timings.get(timings.len() / 2),
                "minMs": timings.first(),
                "maxMs": timings.last(),
                "htmlBytes": bytes,
                "failures": failures,
            }));
        }
    }
    println!("{}", serde_json::to_string_pretty(&reports)?);
    Ok(())
}

fn init_tracing() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("scorch=info"));
    let json = env::var("SCORCH_LOG_FORMAT").is_ok_and(|value| value.eq_ignore_ascii_case("json"));
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr);
    if json {
        subscriber.json().flatten_event(true).init();
    } else {
        subscriber.compact().init();
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler")
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! { () = ctrl_c => {}, () = terminate => {} }
    info!("shutdown signal received");
}
