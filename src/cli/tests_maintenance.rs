use super::types::{Cli, Commands};
use clap::Parser;

#[test]
fn cli_parses_top_level_cleanup_preview() {
    let cleanup = Cli::parse_from(["remem", "cleanup", "--dry-run", "--json"]);
    match cleanup.command {
        Commands::Cleanup { dry_run, json } => {
            assert!(dry_run);
            assert!(json);
        }
        _ => panic!("expected cleanup command"),
    }
}
