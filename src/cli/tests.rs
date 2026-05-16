use super::cwd::resolve_cwd_arg;
use super::types::{Cli, Commands, ReviewAction};
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
        } => {
            assert_eq!(query, "Melanie rollout");
            assert_eq!(project.as_deref(), Some("personal"));
            assert_eq!(memory_type.as_deref(), Some("decision"));
            assert_eq!(limit, 10);
            assert_eq!(offset, 1);
            assert_eq!(branch.as_deref(), Some("main"));
            assert!(include_stale);
            assert!(multi_hop);
        }
        _ => panic!("expected search command"),
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
