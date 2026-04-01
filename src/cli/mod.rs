mod actions;
mod cwd;
mod dispatch;
mod types;
#[cfg(test)]
mod tests;

use anyhow::Result;
use clap::Parser;

use types::Cli;

pub async fn run() -> Result<()> {
    dispatch::run_cli(Cli::parse()).await
}
