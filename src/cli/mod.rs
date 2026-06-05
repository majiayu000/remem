mod actions;
mod cwd;
mod dispatch;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_eval;
mod types;

use anyhow::Result;
use clap::{Command, CommandFactory, FromArgMatches};

use types::Cli;

pub async fn run() -> Result<()> {
    let matches = cli_command().get_matches();
    let cli = Cli::from_arg_matches(&matches)?;
    dispatch::run_cli(cli).await
}

fn cli_command() -> Command {
    let version = crate::build_info::version_label();
    Cli::command()
        .version(version.clone())
        .long_version(version)
}
