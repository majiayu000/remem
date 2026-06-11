use clap::Parser;

use super::types::{Cli, Commands, ConfigAction};

#[test]
fn cli_parses_config_migrate_claude_gate_options() {
    let cli = Cli::parse_from([
        "remem",
        "config",
        "migrate-claude-gate",
        "--dry-run",
        "--json",
    ]);

    match cli.command {
        Commands::Config {
            action: ConfigAction::MigrateClaudeGate { dry_run, json },
        } => {
            assert!(dry_run);
            assert!(json);
        }
        _ => panic!("expected config migrate-claude-gate command"),
    }
}
