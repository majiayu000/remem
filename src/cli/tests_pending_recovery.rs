use super::super::types::{Cli, Commands, ImportAction, PendingAction};
use clap::Parser;

#[cfg(feature = "eval")]
#[test]
fn cli_parses_eval_e2e_options() {
    let cli = Cli::parse_from(["remem", "eval-e2e", "--json", "--keep-data-dir", "-k", "3"]);

    match cli.command {
        Commands::EvalE2e {
            k,
            json,
            keep_data_dir,
        } => {
            assert_eq!(k, 3);
            assert!(json);
            assert!(keep_data_dir);
        }
        _ => panic!("expected eval-e2e command"),
    }
}

#[cfg(feature = "eval")]
#[test]
fn cli_parses_eval_governance_options() {
    let cli = Cli::parse_from(["remem", "eval-governance", "--json", "-k", "4"]);

    match cli.command {
        Commands::EvalGovernance { k, json } => {
            assert_eq!(k, 4);
            assert!(json);
        }
        _ => panic!("expected eval-governance command"),
    }
}

#[test]
fn cli_parses_pending_short_aliases() {
    let list = Cli::parse_from(["remem", "pending", "list", "--limit", "3"]);
    match list.command {
        Commands::Pending {
            action: PendingAction::ListFailed { limit, .. },
        } => assert_eq!(limit, 3),
        _ => panic!("expected pending list alias"),
    }

    let retry = Cli::parse_from(["remem", "pending", "retry", "--dry-run"]);
    match retry.command {
        Commands::Pending {
            action: PendingAction::RetryFailed { dry_run, limit, .. },
        } => {
            assert!(dry_run);
            assert_eq!(limit, 100);
        }
        _ => panic!("expected pending retry alias"),
    }

    let purge = Cli::parse_from(["remem", "pending", "purge", "--older-than-days", "14"]);
    match purge.command {
        Commands::Pending {
            action: PendingAction::PurgeFailed {
                older_than_days, ..
            },
        } => assert_eq!(older_than_days, 14),
        _ => panic!("expected pending purge alias"),
    }
}

#[test]
fn cli_parses_exact_archived_pending_recovery() {
    let cli = Cli::parse_from([
        "remem",
        "pending",
        "recover-archived",
        "--id",
        "42",
        "--host",
        "codex-cli",
        "--dry-run",
        "--json",
    ]);

    match cli.command {
        Commands::Pending {
            action:
                PendingAction::RecoverArchived {
                    id,
                    host,
                    dry_run,
                    json,
                },
        } => {
            assert_eq!(id, 42);
            assert_eq!(host.as_deref(), Some("codex-cli"));
            assert!(dry_run);
            assert!(json);
        }
        _ => panic!("expected exact archived pending recovery"),
    }
}

#[test]
fn cli_rejects_invalid_archived_pending_recovery_target_or_host() {
    assert!(Cli::try_parse_from(["remem", "pending", "recover-archived", "--id", "0",]).is_err());
    assert!(Cli::try_parse_from([
        "remem",
        "pending",
        "recover-archived",
        "--id",
        "42",
        "--host",
        "unknown",
    ])
    .is_err());
}

#[test]
fn cli_parses_markdown_export_and_import_commands() {
    let export = Cli::parse_from([
        "remem",
        "export",
        "--markdown",
        "--output",
        "/tmp/remem-md",
        "--project",
        "/repo",
        "--include-inactive",
        "--limit",
        "25",
    ]);
    match export.command {
        Commands::Export(args) => {
            assert!(args.markdown);
            assert_eq!(
                args.output.as_deref(),
                Some(std::path::Path::new("/tmp/remem-md"))
            );
            assert!(args.pack.is_none());
            assert_eq!(args.project.as_deref(), Some("/repo"));
            assert!(args.include_inactive);
            assert_eq!(args.limit, 25);
        }
        _ => panic!("expected export command"),
    }

    let import = Cli::parse_from([
        "remem",
        "import",
        "markdown",
        "--source",
        "/tmp/remem-md",
        "--best-effort",
    ]);
    match import.command {
        Commands::Import {
            action:
                Some(ImportAction::Markdown {
                    source,
                    best_effort,
                }),
            pack,
            dry_run,
        } => {
            assert_eq!(source, std::path::PathBuf::from("/tmp/remem-md"));
            assert!(best_effort);
            assert!(pack.is_none());
            assert!(!dry_run);
        }
        _ => panic!("expected import markdown command"),
    }
}

#[test]
fn cli_parses_pack_export_command() {
    let export = Cli::parse_from([
        "remem",
        "export",
        "--pack",
        "/repo/.remem-pack",
        "--project",
        "/repo",
        "--limit",
        "50",
    ]);
    match export.command {
        Commands::Export(args) => {
            assert!(!args.markdown);
            assert!(args.output.is_none());
            assert_eq!(
                args.pack.as_deref(),
                Some(std::path::Path::new("/repo/.remem-pack"))
            );
            assert_eq!(args.project.as_deref(), Some("/repo"));
            assert_eq!(args.limit, 50);
        }
        _ => panic!("expected export command"),
    }
}
