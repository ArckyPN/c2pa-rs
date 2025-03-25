mod cli;
mod live_signing;
mod signer;

use std::time::Instant;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands};
use live_signing::LiveBenchmark;

fn main() -> Result<()> {
    let now = Instant::now();
    let cli = Cli::parse();

    pretty_env_logger::init();

    match &cli.command {
        Commands::LiveSigning(live) => LiveBenchmark::new(live)?.run()?,
    }

    log::info!("finished running {} in {:?}", cli.command, now.elapsed());
    Ok(())
}
