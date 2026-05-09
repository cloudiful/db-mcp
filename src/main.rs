use anyhow::Result;
use clap::Parser;
use db_mcp::{config::Cli, runtime};

#[tokio::main]
async fn main() -> Result<()> {
    runtime::init_tracing();
    let cli = Cli::parse();
    runtime::run(cli).await
}
