use super::types::{Cli, Commands, RawAction, RawRole};
use clap::Parser;

#[test]
fn cli_parses_raw_search_filters() {
    let cli = Cli::parse_from([
        "remem",
        "raw",
        "search",
        "literal phrase",
        "--project",
        "/repo",
        "--branch",
        "main",
        "--role",
        "user",
        "--limit",
        "20",
        "--offset",
        "40",
        "--since",
        "2026-01-01",
        "--until",
        "1767312000",
        "--json",
    ]);

    match cli.command {
        Commands::Raw {
            action:
                RawAction::Search {
                    query,
                    project,
                    branch,
                    role,
                    limit,
                    offset,
                    since,
                    until,
                    json,
                },
        } => {
            assert_eq!(query, "literal phrase");
            assert_eq!(project.as_deref(), Some("/repo"));
            assert_eq!(branch.as_deref(), Some("main"));
            assert_eq!(role, Some(RawRole::User));
            assert_eq!(limit, 20);
            assert_eq!(offset, 40);
            assert_eq!(since.as_deref(), Some("2026-01-01"));
            assert_eq!(until.as_deref(), Some("1767312000"));
            assert!(json);
        }
        _ => panic!("expected raw search command"),
    }
}

#[test]
fn cli_parses_raw_sessions_window() {
    let cli = Cli::parse_from([
        "remem",
        "raw",
        "sessions",
        "--since",
        "2026-01-01",
        "--until",
        "2026-02-01",
        "--project",
        "/repo",
        "--sample",
        "3",
        "--json",
    ]);

    match cli.command {
        Commands::Raw {
            action:
                RawAction::Sessions {
                    since,
                    until,
                    project,
                    sample,
                    json,
                },
        } => {
            assert_eq!(since.as_deref(), Some("2026-01-01"));
            assert_eq!(until.as_deref(), Some("2026-02-01"));
            assert_eq!(project.as_deref(), Some("/repo"));
            assert_eq!(sample, 3);
            assert!(json);
        }
        _ => panic!("expected raw sessions command"),
    }
}
