use anyhow::Result;
use clap::Parser;
use db_mcp::{config::Cli, runtime};
use std::process;

#[tokio::main]
async fn main() -> Result<()> {
    runtime::init_tracing();
    let cli = Cli::parse();
    if let Err(err) = runtime::run(cli).await {
        eprintln!("db-mcp failed: {err}");
        for cause in err.chain().skip(1) {
            eprintln!("caused by: {cause}");
        }
        process::exit(1);
    }
    Ok(())
}
