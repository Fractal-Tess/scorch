use std::{net::SocketAddr, path::PathBuf};

use clap::Parser;

#[derive(Debug, Parser)]
#[command(version, about)]
pub struct Config {
    #[arg(long, env = "SCORCH_BIND", default_value = "127.0.0.1:3000")]
    pub bind: SocketAddr,

    #[arg(long, env = "SCORCH_BROWSER_PATH", default_value = "chromium")]
    pub browser_path: PathBuf,

    #[arg(long, env = "SCORCH_MAX_CONCURRENCY", default_value_t = 4)]
    pub max_concurrency: usize,

    #[arg(long, env = "SCORCH_REQUEST_TIMEOUT_SECS", default_value_t = 30)]
    pub request_timeout_secs: u64,
}
