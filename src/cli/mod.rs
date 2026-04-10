mod actions;
mod cwd;
mod dispatch;
#[cfg(test)]
mod tests;
mod types;

use anyhow::Result;
use clap::Parser;

use types::Cli;

pub async fn run() -> Result<()> {
    dispatch::run_cli(Cli::parse()).await
}
