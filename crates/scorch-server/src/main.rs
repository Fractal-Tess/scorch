use std::{env, net::SocketAddr, time::Duration};

use clap::{Parser, ValueEnum};
use metasearch::{EngineCredentials, EngineKind};
use scorch_engine::{EngineConfig, ScorchEngine};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "scorchd", version, about = "Scorch HTTP API service")]
struct ServerArgs {
    #[arg(long, env = "SCORCH_BIND", default_value = "127.0.0.1:33000")]
    bind: SocketAddr,
    #[arg(
        long,
        env = "SCORCH_OBSCURA_STEALTH",
        default_value_t = true,
        action = clap::ArgAction::Set
    )]
    obscura_stealth: bool,
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
        let search_engines = unique_engines(&self.search_engines);
        EngineConfig {
            obscura_stealth: self.obscura_stealth,
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

fn unique_engines(engines: &[SearchEngineArg]) -> Vec<EngineKind> {
    let mut unique = Vec::new();
    for engine in engines.iter().copied().map(EngineKind::from) {
        if !unique.contains(&engine) {
            unique.push(engine);
        }
    }
    unique
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
        if let Err(error) = tokio::signal::ctrl_c().await {
            warn!(%error, "failed to receive Ctrl+C signal");
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => warn!(%error, "failed to install SIGTERM handler"),
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! { () = ctrl_c => {}, () = terminate => {} }
    info!("shutdown signal received");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obscura_stealth_is_enabled_by_default() {
        let args = ServerArgs::try_parse_from(["scorchd"]).unwrap();
        assert!(args.engine_config().obscura_stealth);
    }

    #[test]
    fn obscura_standard_transport_requires_an_explicit_override() {
        let args = ServerArgs::try_parse_from(["scorchd", "--obscura-stealth", "false"]).unwrap();
        assert!(!args.engine_config().obscura_stealth);
    }

    #[test]
    fn removed_browser_flags_are_rejected() {
        for flag in ["--browser", "--allowed-browsers", "--browser-path"] {
            assert!(ServerArgs::try_parse_from(["scorchd", flag, "obscura"]).is_err());
        }
    }
}
