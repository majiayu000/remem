use super::types::{Cli, Commands, MemoryGovernanceCliAction};
use clap::Parser;

#[test]
fn cli_parses_governance_acknowledge_pattern_options() {
    let cli = Cli::parse_from([
        "remem",
        "govern",
        "--action",
        "acknowledge-pattern",
        "--acknowledge-pattern",
        "override_previous_instructions",
        "--reason",
        "quoted false positive",
        "--confirm-destructive",
        "42",
    ]);

    match cli.command {
        Commands::Govern {
            action,
            acknowledge_pattern,
            reason,
            confirm_destructive,
            ids,
            ..
        } => {
            assert!(matches!(
                action,
                MemoryGovernanceCliAction::AcknowledgePattern
            ));
            assert_eq!(
                acknowledge_pattern.as_deref(),
                Some("override_previous_instructions")
            );
            assert_eq!(reason.as_deref(), Some("quoted false positive"));
            assert!(confirm_destructive);
            assert_eq!(ids, vec![42]);
        }
        _ => panic!("expected govern command"),
    }
}
