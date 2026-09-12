use clap::Parser;
use db_mcp::{config::Cli, runtime};
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    runtime::init_tracing();
    let cli = Cli::parse();
    runtime::dispatch(cli).await
}
