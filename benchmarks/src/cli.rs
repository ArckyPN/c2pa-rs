use std::{fmt::Display, path::PathBuf};

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Run the comparison between live and original signing.
    #[command(name = "live")]
    LiveSigning(LiveSigning),
}

impl Display for Commands {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Commands::LiveSigning(_) => f.write_str("live"),
        }
    }
}

#[derive(Debug, Parser)]
pub struct LiveSigning {
    /// Path to the directory containing the test fragments
    #[arg(long, default_value = "benchmarks/fragments")]
    pub dir: PathBuf,

    /// Path to the data output file
    #[arg(short, long = "out", default_value = "benchmarks/data-live.json")]
    pub output: PathBuf,

    #[arg(short = 'n', long, default_value = "5")]
    pub samples: usize,
}
