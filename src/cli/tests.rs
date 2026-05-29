use super::cwd::resolve_cwd_arg;
use super::types::{Cli, Commands, CommitAction, MemoryGovernanceCliAction, ReviewAction};
use clap::Parser;

#[test]
fn cli_resolve_cwd_arg_prefers_explicit_value() {
    assert_eq!(
        resolve_cwd_arg(Some("/tmp/remem".to_string())),
        "/tmp/remem"
    );
}

#[test]
fn cli_resolve_cwd_arg_falls_back_to_current_dir() {
    let expected = std::env::current_dir()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    assert_eq!(resolve_cwd_arg(None), expected);
}

#[test]
fn cli_parses_review_edit_options() {
    let cli = Cli::parse_from([
        "remem",
        "review",
        "edit",
        "42",
        "--text",
        "edited memory",
        "--topic-key",
        "edited-topic",
        "--type",
        "architecture",
    ]);

    match cli.command {
        Commands::Review {
            action:
                ReviewAction::Edit {
                    id,
                    text,
                    topic_key,
                    memory_type,
                    scope,
                },
        } => {
            assert_eq!(id, 42);
            assert_eq!(text.as_deref(), Some("edited memory"));
            assert_eq!(topic_key.as_deref(), Some("edited-topic"));
            assert_eq!(memory_type.as_deref(), Some("architecture"));
            assert!(scope.is_none());
        }
        _ => panic!("expected review edit command"),
    }
}

#[test]
fn cli_parses_search_type_alias_and_multi_hop_filters() {
    let cli = Cli::parse_from([
        "remem",
        "search",
        "Melanie rollout",
        "--project",
        "personal",
        "--type",
        "decision",
        "--branch",
        "main",
        "--offset",
        "1",
        "--include-stale",
        "--multi-hop",
    ]);

    match cli.command {
        Commands::Search {
            query,
            project,
            memory_type,
            limit,
            offset,
            branch,
            include_stale,
            multi_hop,
            explain,
        } => {
            assert_eq!(query, "Melanie rollout");
            assert_eq!(project.as_deref(), Some("personal"));
            assert_eq!(memory_type.as_deref(), Some("decision"));
            assert_eq!(limit, 10);
            assert_eq!(offset, 1);
            assert_eq!(branch.as_deref(), Some("main"));
            assert!(include_stale);
            assert!(multi_hop);
            assert!(!explain);
        }
        _ => panic!("expected search command"),
    }
}

#[test]
fn cli_parses_why_project_and_branch_filters() {
    let cli = Cli::parse_from([
        "remem",
        "why",
        "78360",
        "--project",
        "/tmp/remem",
        "--branch",
        "main",
    ]);

    match cli.command {
        Commands::Why {
            id,
            project,
            branch,
        } => {
            assert_eq!(id, 78360);
            assert_eq!(project.as_deref(), Some("/tmp/remem"));
            assert_eq!(branch.as_deref(), Some("main"));
        }
        _ => panic!("expected why command"),
    }
}

#[test]
fn cli_parses_governance_delete_options() {
    let cli = Cli::parse_from([
        "remem",
        "govern",
        "--project",
        "/tmp/remem",
        "--action",
        "delete",
        "--reason",
        "bad memory",
        "--actor",
        "codex",
        "--confirm-destructive",
        "42",
        "43",
    ]);

    match cli.command {
        Commands::Govern {
            project,
            action,
            reason,
            actor,
            confirm_destructive,
            dry_run,
            ids,
        } => {
            assert_eq!(project.as_deref(), Some("/tmp/remem"));
            assert!(matches!(action, MemoryGovernanceCliAction::Delete));
            assert_eq!(reason.as_deref(), Some("bad memory"));
            assert_eq!(actor.as_deref(), Some("codex"));
            assert!(confirm_destructive);
            assert!(!dry_run);
            assert_eq!(ids, vec![42, 43]);
        }
        _ => panic!("expected govern command"),
    }
}

#[test]
fn cli_parses_usage_options() {
    let cli = Cli::parse_from([
        "remem",
        "usage",
        "--project",
        "/tmp/remem",
        "--days",
        "30",
        "--weeks",
        "12",
    ]);

    match cli.command {
        Commands::Usage {
            project,
            days,
            weeks,
        } => {
            assert_eq!(project.as_deref(), Some("/tmp/remem"));
            assert_eq!(days, 30);
            assert_eq!(weeks, 12);
        }
        _ => panic!("expected usage command"),
    }
}

#[test]
fn cli_parses_commit_show_and_session_lookup() {
    let show = Cli::parse_from([
        "remem",
        "commit",
        "show",
        "abcdef1",
        "--project",
        "proj",
        "--json",
    ]);
    match show.command {
        Commands::Commit {
            action: CommitAction::Show { sha, project, json },
        } => {
            assert_eq!(sha, "abcdef1");
            assert_eq!(project.as_deref(), Some("proj"));
            assert!(json);
        }
        _ => panic!("expected commit show command"),
    }

    let session = Cli::parse_from([
        "remem",
        "commit",
        "session",
        "content-session-1",
        "--limit",
        "7",
    ]);
    match session.command {
        Commands::Commit {
            action:
                CommitAction::Session {
                    session_id,
                    project,
                    limit,
                    json,
                },
        } => {
            assert_eq!(session_id, "content-session-1");
            assert!(project.is_none());
            assert_eq!(limit, 7);
            assert!(!json);
        }
        _ => panic!("expected commit session command"),
    }
}

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

#[test]
fn cli_parses_context_debug_option() {
    let cli = Cli::parse_from([
        "remem",
        "context",
        "--cwd",
        "/tmp/remem",
        "--host",
        "codex-cli",
        "--debug",
    ]);

    match cli.command {
        Commands::Context {
            cwd, host, debug, ..
        } => {
            assert_eq!(cwd.as_deref(), Some("/tmp/remem"));
            assert_eq!(host.as_deref(), Some("codex-cli"));
            assert!(debug);
        }
        _ => panic!("expected context command"),
    }
}
