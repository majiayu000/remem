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

#[test]
fn cli_parses_exact_raw_messages_selector_and_cursor() {
    let cli = Cli::parse_from([
        "remem",
        "raw",
        "messages",
        "--source-root",
        "remote",
        "--project",
        "/repo",
        "--session-id",
        "session-1",
        "--limit",
        "750",
        "--cursor",
        "rm1_deadbeef",
        "--json",
    ]);

    match cli.command {
        Commands::Raw {
            action:
                RawAction::Messages {
                    source_root,
                    project,
                    session_id,
                    limit,
                    cursor,
                    json,
                },
        } => {
            assert_eq!(source_root, "remote");
            assert_eq!(project, "/repo");
            assert_eq!(session_id, "session-1");
            assert_eq!(limit, 750);
            assert_eq!(cursor.as_deref(), Some("rm1_deadbeef"));
            assert!(json);
        }
        _ => panic!("expected raw messages command"),
    }
}

#[test]
fn cli_raw_messages_defaults_to_500_and_requires_exact_tuple() {
    let cli = Cli::parse_from([
        "remem",
        "raw",
        "messages",
        "--source-root",
        "local",
        "--project",
        "/repo",
        "--session-id",
        "session-1",
    ]);
    match cli.command {
        Commands::Raw {
            action: RawAction::Messages { limit, .. },
        } => assert_eq!(limit, 500),
        _ => panic!("expected raw messages command"),
    }

    assert!(Cli::try_parse_from([
        "remem",
        "raw",
        "messages",
        "--project",
        "/repo",
        "--session-id",
        "session-1",
    ])
    .is_err());
    assert!(Cli::try_parse_from([
        "remem",
        "raw",
        "messages",
        "--source-root",
        "local",
        "--session-id",
        "session-1",
    ])
    .is_err());
    assert!(Cli::try_parse_from([
        "remem",
        "raw",
        "messages",
        "--source-root",
        "local",
        "--project",
        "/repo",
    ])
    .is_err());
}

#[test]
fn cli_parses_raw_reconcile_bounds_and_repeatable_roots() {
    let cli = Cli::parse_from([
        "remem",
        "raw",
        "reconcile",
        "--since",
        "1783653658",
        "--until",
        "1784258459",
        "--root",
        "fixture=/tmp/one",
        "--root",
        "fixture=/tmp/two",
        "--json",
    ]);

    match cli.command {
        Commands::Raw {
            action:
                RawAction::Reconcile {
                    since,
                    until,
                    roots,
                    json,
                },
        } => {
            assert_eq!(since, "1783653658");
            assert_eq!(until, "1784258459");
            assert_eq!(roots, vec!["fixture=/tmp/one", "fixture=/tmp/two"]);
            assert!(json);
        }
        _ => panic!("expected raw reconcile command"),
    }
}
