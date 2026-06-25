use super::types::{Cli, Commands, TimelineAction, WorkstreamAction, WorkstreamStatusArg};
use clap::Parser;

#[test]
fn cli_parses_timeline_around_json_filters() {
    let cli = Cli::parse_from([
        "remem",
        "timeline",
        "around",
        "--query",
        "release manifest",
        "--project",
        "/repo",
        "--depth-before",
        "3",
        "--depth-after",
        "4",
        "--json",
    ]);

    match cli.command {
        Commands::Timeline {
            action:
                TimelineAction::Around {
                    anchor,
                    query,
                    project,
                    depth_before,
                    depth_after,
                    json,
                },
        } => {
            assert_eq!(anchor, None);
            assert_eq!(query.as_deref(), Some("release manifest"));
            assert_eq!(project.as_deref(), Some("/repo"));
            assert_eq!(depth_before, 3);
            assert_eq!(depth_after, 4);
            assert!(json);
        }
        _ => panic!("expected timeline around command"),
    }
}

#[test]
fn cli_parses_workstream_update_json_filters() {
    let cli = Cli::parse_from([
        "remem",
        "workstreams",
        "update",
        "42",
        "--project",
        "/repo",
        "--status",
        "paused",
        "--next-action",
        "wait for connector id",
        "--blockers",
        "external registration",
        "--confirm",
        "--json",
    ]);

    match cli.command {
        Commands::Workstreams {
            action:
                WorkstreamAction::Update {
                    id,
                    project,
                    status,
                    next_action,
                    blockers,
                    confirm,
                    json,
                },
        } => {
            assert_eq!(id, 42);
            assert_eq!(project, "/repo");
            assert_eq!(status, Some(WorkstreamStatusArg::Paused));
            assert_eq!(next_action.as_deref(), Some("wait for connector id"));
            assert_eq!(blockers.as_deref(), Some("external registration"));
            assert!(confirm);
            assert!(json);
        }
        _ => panic!("expected workstream update command"),
    }
}

#[test]
fn cli_parses_workstream_merge_json_filters() {
    let cli = Cli::parse_from([
        "remem",
        "workstreams",
        "merge",
        "--project",
        "/repo",
        "--into",
        "42",
        "43",
        "44",
        "--confirm",
        "--json",
    ]);

    match cli.command {
        Commands::Workstreams {
            action:
                WorkstreamAction::Merge {
                    project,
                    into,
                    duplicates,
                    confirm,
                    json,
                },
        } => {
            assert_eq!(project, "/repo");
            assert_eq!(into, 42);
            assert_eq!(duplicates, vec![43, 44]);
            assert!(confirm);
            assert!(json);
        }
        _ => panic!("expected workstream merge command"),
    }
}

#[test]
fn cli_rejects_invalid_workstream_status() {
    let parsed = Cli::try_parse_from([
        "remem",
        "workstreams",
        "update",
        "42",
        "--project",
        "/repo",
        "--status",
        "waiting",
        "--confirm",
        "--json",
    ]);

    assert!(parsed.is_err());
}
