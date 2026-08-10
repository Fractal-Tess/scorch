use std::{env, net::SocketAddr, path::PathBuf, time::Duration};

use clap::{Parser, ValueEnum};
use metasearch::{EngineCredentials, EngineKind};
use scorch_engine::{EngineConfig, ScorchEngine};
use scorch_types::BrowserBackend;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(version, about = "Scorch HTTP API service")]
struct ServerArgs {
    #[arg(long, env = "SCORCH_BIND", default_value = "127.0.0.1:3000")]
    bind: SocketAddr,
    #[arg(long, value_enum, env = "SCORCH_BROWSER", default_value = "obscura")]
    browser: BrowserArg,
    #[arg(
        long,
        value_enum,
        value_delimiter = ',',
        env = "SCORCH_ALLOWED_BROWSERS",
        default_value = "obscura"
    )]
    allowed_browsers: Vec<BrowserArg>,
    #[arg(long, env = "SCORCH_BROWSER_PATH", default_value = "chromium")]
    browser_path: PathBuf,
    #[arg(long, env = "SCORCH_MAX_CONCURRENCY", default_value_t = 4)]
    max_concurrency: usize,
    #[arg(long, env = "SCORCH_MAX_RESPONSE_BYTES", default_value_t = 5 * 1024 * 1024)]
    max_response_bytes: usize,
    #[arg(long, env = "SCORCH_JOB_TTL_SECS", default_value_t = 900)]
    job_ttl_secs: u64,
    #[arg(
        long,
        value_enum,
        value_delimiter = ',',
        env = "SCORCH_SEARCH_ENGINES",
        default_value = "bing,duckduckgo,naver,wikipedia"
    )]
    search_engines: Vec<SearchEngineArg>,
    #[arg(long, env = "SCORCH_BRAVE_SEARCH_API_KEY", hide_env_values = true)]
    brave_search_api_key: Option<String>,
    #[arg(long, env = "SCORCH_GOOGLE_SEARCH_API_KEY", hide_env_values = true)]
    google_search_api_key: Option<String>,
    #[arg(long, env = "SCORCH_GOOGLE_SEARCH_ENGINE_ID", hide_env_values = true)]
    google_search_engine_id: Option<String>,
}

impl ServerArgs {
    fn engine_config(&self) -> EngineConfig {
        let mut search_engines = Vec::new();
        for engine in self.search_engines.iter().copied().map(EngineKind::from) {
            if !search_engines.contains(&engine) {
                search_engines.push(engine);
            }
        }
        let mut allowed_browsers = Vec::new();
        for browser in self.allowed_browsers.iter().copied().map(Into::into) {
            if !allowed_browsers.contains(&browser) {
                allowed_browsers.push(browser);
            }
        }
        EngineConfig {
            browser: self.browser.into(),
            allowed_browsers,
            browser_path: self.browser_path.clone(),
            max_concurrency: self.max_concurrency.max(1),
            max_response_bytes: self.max_response_bytes.max(1024),
            job_ttl: Duration::from_secs(self.job_ttl_secs.max(1)),
            search_engines,
            search_engine_credentials: EngineCredentials {
                brave_api_key: self.brave_search_api_key.clone(),
                google_api_key: self.google_search_api_key.clone(),
                google_search_engine_id: self.google_search_engine_id.clone(),
            },
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum BrowserArg {
    Obscura,
    Chromium,
}

impl From<BrowserArg> for BrowserBackend {
    fn from(value: BrowserArg) -> Self {
        match value {
            BrowserArg::Obscura => Self::Obscura,
            BrowserArg::Chromium => Self::Chromium,
        }
    }
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

impl From<SearchEngineArg> for EngineKind {
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let args = ServerArgs::parse();
    info!(
        version = env!("CARGO_PKG_VERSION"),
        bind = %args.bind,
        "starting Scorch API service"
    );
    let engine = ScorchEngine::new(args.engine_config()).await?;
    scorch_api::serve(args.bind, engine, shutdown_signal()).await?;
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
