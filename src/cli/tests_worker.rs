use clap::Parser;

use super::types::{Cli, Commands, PendingAction};

#[test]
fn pending_archived_range_requires_dual_exact_dry_run() {
    let valid = Cli::parse_from([
        "remem",
        "pending",
        "retry-extraction-ranges",
        "--id",
        "308",
        "--acknowledge-quarantine",
        "--include-archived",
        "--dry-run",
    ]);
    match valid.command {
        Commands::Pending {
            action:
                PendingAction::RetryExtractionRanges {
                    id,
                    acknowledge_quarantine,
                    include_archived,
                    dry_run,
                    ..
                },
        } => {
            assert_eq!(id, Some(308));
            assert!(acknowledge_quarantine);
            assert!(include_archived);
            assert!(dry_run);
        }
        _ => panic!("expected archived exact dry-run"),
    }

    for invalid in [
        vec!["--include-archived"],
        vec!["--id", "308", "--include-archived", "--dry-run"],
        vec![
            "--id",
            "308",
            "--acknowledge-quarantine",
            "--include-archived",
        ],
    ] {
        let mut args = vec!["remem", "pending", "retry-extraction-ranges"];
        args.extend(invalid);
        assert!(Cli::try_parse_from(args).is_err());
    }
}

#[test]
fn worker_exact_replay_requires_complete_argument_set() {
    let cli = Cli::parse_from([
        "remem",
        "worker",
        "--once",
        "--replay-range-id",
        "308",
        "--acknowledge-quarantine",
        "--include-archived",
        "--profile",
        "claude",
    ]);
    match cli.command {
        Commands::Worker(args) => {
            assert!(args.once);
            assert_eq!(args.replay_range_id, Some(308));
            assert!(args.acknowledge_quarantine);
            assert!(args.include_archived);
            assert_eq!(args.profile.as_deref(), Some("claude"));
        }
        _ => panic!("expected exact worker command"),
    }

    for incomplete in [
        vec!["--replay-range-id", "308"],
        vec!["--once", "--replay-range-id", "308", "--profile", "claude"],
        vec!["--once", "--profile", "claude"],
    ] {
        let mut args = vec!["remem", "worker"];
        args.extend(incomplete);
        assert!(Cli::try_parse_from(args).is_err());
    }
}
