mod agent;
mod boot;
mod cli;
mod concept;
mod config;
mod cron;
mod error;
mod http;
mod infra;
mod locale;
mod mcp;
mod operation;
mod rest;
mod theme;
mod ui;
mod usecase;

use error::Result;
use std::process::ExitCode;

rust_i18n::i18n!("locales", fallback = "en");

#[tokio::main]
async fn main() -> Result<ExitCode> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        return cli::run(&args).await;
    }
    logging::init()?;
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    boot::run().await.map(|()| ExitCode::SUCCESS)
}
