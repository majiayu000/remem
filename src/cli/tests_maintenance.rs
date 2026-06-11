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

#[test]
fn cli_parses_encrypt_rekey_raw() {
    let cli = Cli::parse_from(["remem", "encrypt", "--rekey-raw"]);
    match cli.command {
        Commands::Encrypt { rekey_raw } => assert!(rekey_raw),
        _ => panic!("expected encrypt command"),
    }
}
