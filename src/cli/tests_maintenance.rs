use super::query_types::{
    ProfileSnapshotFormatArg, UserClaimScopeArg, UserClaimSensitivityArg, UserClaimTypeArg,
    UserClaimsAction, UserProfileAction, UserReviewAction, UserSummaryAction,
};
use super::types::{Cli, Commands, UserAction};
use clap::Parser;

#[test]
fn cli_parses_top_level_cleanup_preview() {
    let cleanup = Cli::parse_from(["remem", "cleanup", "--dry-run", "--json"]);
    match cleanup.command {
        Commands::Cleanup {
            dry_run,
            json,
            archived_failures,
        } => {
            assert!(dry_run);
            assert!(json);
            assert_eq!(archived_failures, None);
        }
        _ => panic!("expected cleanup command"),
    }
}

#[test]
fn cli_parses_cleanup_archived_failures_default_horizon() {
    let cleanup = Cli::parse_from(["remem", "cleanup", "--archived-failures"]);
    match cleanup.command {
        Commands::Cleanup {
            archived_failures, ..
        } => assert_eq!(archived_failures, Some(90)),
        _ => panic!("expected cleanup command"),
    }
}

#[test]
fn cli_parses_cleanup_archived_failures_custom_horizon() {
    let cleanup = Cli::parse_from(["remem", "cleanup", "--archived-failures=30"]);
    match cleanup.command {
        Commands::Cleanup {
            archived_failures, ..
        } => assert_eq!(archived_failures, Some(30)),
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

#[test]
fn cli_parses_backfill_embeddings_limit() {
    let cli = Cli::parse_from(["remem", "backfill-embeddings", "--limit", "250"]);
    match cli.command {
        Commands::BackfillEmbeddings { limit, batch_size } => {
            assert_eq!(limit, 250);
            assert_eq!(batch_size, 1000);
        }
        _ => panic!("expected backfill-embeddings command"),
    }
}

#[test]
fn cli_parses_reindex_embeddings_alias() {
    let cli = Cli::parse_from(["remem", "reindex-embeddings", "--limit", "50"]);
    match cli.command {
        Commands::BackfillEmbeddings { limit, batch_size } => {
            assert_eq!(limit, 50);
            assert_eq!(batch_size, 1000);
        }
        _ => panic!("expected reindex-embeddings alias"),
    }
}

#[test]
fn cli_parses_reindex_embeddings_batch_size() {
    let cli = Cli::parse_from([
        "remem",
        "reindex-embeddings",
        "--limit",
        "100000",
        "--batch-size",
        "5000",
    ]);
    match cli.command {
        Commands::BackfillEmbeddings { limit, batch_size } => {
            assert_eq!(limit, 100000);
            assert_eq!(batch_size, 5000);
        }
        _ => panic!("expected reindex-embeddings alias"),
    }
}

#[test]
fn cli_parses_user_remember_with_claim_metadata() {
    let cli = Cli::parse_from([
        "remem",
        "user",
        "remember",
        "--scope",
        "repo",
        "--owner-key",
        "/repo",
        "--type",
        "goal",
        "--key",
        "goal:remem",
        "--sensitivity",
        "personal",
        "--confidence",
        "0.8",
        "--json",
        "Make remem the best coding-agent memory system",
    ]);

    match cli.command {
        Commands::User {
            action:
                UserAction::Remember {
                    scope,
                    owner_key,
                    claim_type,
                    claim_key,
                    sensitivity,
                    confidence,
                    json,
                    text,
                    ..
                },
        } => {
            assert_eq!(scope, UserClaimScopeArg::Repo);
            assert_eq!(owner_key.as_deref(), Some("/repo"));
            assert_eq!(claim_type, UserClaimTypeArg::Goal);
            assert_eq!(claim_key.as_deref(), Some("goal:remem"));
            assert_eq!(sensitivity, UserClaimSensitivityArg::Personal);
            assert_eq!(confidence, 0.8);
            assert!(json);
            assert_eq!(text, "Make remem the best coding-agent memory system");
        }
        _ => panic!("expected user remember command"),
    }
}

#[test]
fn cli_parses_user_claim_governance_commands() {
    let list = Cli::parse_from([
        "remem",
        "user",
        "claims",
        "list",
        "--scope",
        "session",
        "--owner-key",
        "session-1",
        "--include-inactive",
        "--limit",
        "25",
        "--json",
    ]);
    match list.command {
        Commands::User {
            action:
                UserAction::Claims {
                    action:
                        UserClaimsAction::List {
                            scope,
                            owner_key,
                            include_inactive,
                            limit,
                            json,
                        },
                },
        } => {
            assert_eq!(scope, Some(UserClaimScopeArg::Session));
            assert_eq!(owner_key.as_deref(), Some("session-1"));
            assert!(include_inactive);
            assert_eq!(limit, 25);
            assert!(json);
        }
        _ => panic!("expected user claims list command"),
    }

    let show = Cli::parse_from(["remem", "user", "claims", "show", "42", "--json"]);
    match show.command {
        Commands::User {
            action:
                UserAction::Claims {
                    action: UserClaimsAction::Show { id, json },
                },
        } => {
            assert_eq!(id, 42);
            assert!(json);
        }
        _ => panic!("expected user claims show command"),
    }

    let why = Cli::parse_from(["remem", "user", "claims", "why", "42", "--json"]);
    match why.command {
        Commands::User {
            action:
                UserAction::Claims {
                    action: UserClaimsAction::Why { id, json },
                },
        } => {
            assert_eq!(id, 42);
            assert!(json);
        }
        _ => panic!("expected user claims why command"),
    }

    let edit = Cli::parse_from([
        "remem",
        "user",
        "claims",
        "edit",
        "42",
        "--text",
        "Prefer concise updates",
        "--type",
        "preference",
        "--sensitivity",
        "normal",
    ]);
    match edit.command {
        Commands::User {
            action:
                UserAction::Claims {
                    action:
                        UserClaimsAction::Edit {
                            id,
                            text,
                            claim_type,
                            sensitivity,
                            ..
                        },
                },
        } => {
            assert_eq!(id, 42);
            assert_eq!(text, "Prefer concise updates");
            assert_eq!(claim_type, Some(UserClaimTypeArg::Preference));
            assert_eq!(sensitivity, Some(UserClaimSensitivityArg::Normal));
        }
        _ => panic!("expected user claims edit command"),
    }

    let suppress = Cli::parse_from(["remem", "user", "claims", "suppress", "42", "--json"]);
    match suppress.command {
        Commands::User {
            action:
                UserAction::Claims {
                    action: UserClaimsAction::Suppress { id, json },
                },
        } => {
            assert_eq!(id, 42);
            assert!(json);
        }
        _ => panic!("expected user claims suppress command"),
    }

    let unsuppress = Cli::parse_from(["remem", "user", "claims", "unsuppress", "42", "--json"]);
    match unsuppress.command {
        Commands::User {
            action:
                UserAction::Claims {
                    action: UserClaimsAction::Unsuppress { id, json },
                },
        } => {
            assert_eq!(id, 42);
            assert!(json);
        }
        _ => panic!("expected user claims unsuppress command"),
    }

    let delete = Cli::parse_from(["remem", "user", "claims", "delete", "42", "--json"]);
    match delete.command {
        Commands::User {
            action:
                UserAction::Claims {
                    action: UserClaimsAction::Delete { id, json },
                },
        } => {
            assert_eq!(id, 42);
            assert!(json);
        }
        _ => panic!("expected user claims delete command"),
    }
}

#[test]
fn cli_parses_user_summary_commands() {
    let show = Cli::parse_from([
        "remem",
        "user",
        "summary",
        "show",
        "--project",
        "/repo",
        "--scope",
        "repo",
        "--owner-key",
        "/repo",
        "--json",
    ]);
    match show.command {
        Commands::User {
            action:
                UserAction::Summary {
                    action:
                        UserSummaryAction::Show {
                            project,
                            scope,
                            owner_key,
                            json,
                        },
                },
        } => {
            assert_eq!(project.as_deref(), Some("/repo"));
            assert_eq!(scope, UserClaimScopeArg::Repo);
            assert_eq!(owner_key.as_deref(), Some("/repo"));
            assert!(json);
        }
        _ => panic!("expected user summary show command"),
    }

    let refresh = Cli::parse_from([
        "remem",
        "user",
        "summary",
        "refresh",
        "-p",
        "/repo",
        "--scope",
        "workspace",
        "--owner-key",
        "/workspace",
        "--json",
    ]);
    match refresh.command {
        Commands::User {
            action:
                UserAction::Summary {
                    action:
                        UserSummaryAction::Refresh {
                            project,
                            scope,
                            owner_key,
                            json,
                        },
                },
        } => {
            assert_eq!(project.as_deref(), Some("/repo"));
            assert_eq!(scope, UserClaimScopeArg::Workspace);
            assert_eq!(owner_key.as_deref(), Some("/workspace"));
            assert!(json);
        }
        _ => panic!("expected user summary refresh command"),
    }

    let edit = Cli::parse_from([
        "remem",
        "user",
        "summary",
        "edit",
        "--project",
        "/repo",
        "--text",
        "Manual summary",
        "--json",
    ]);
    match edit.command {
        Commands::User {
            action:
                UserAction::Summary {
                    action:
                        UserSummaryAction::Edit {
                            project,
                            scope,
                            owner_key,
                            text,
                            json,
                        },
                },
        } => {
            assert_eq!(project.as_deref(), Some("/repo"));
            assert_eq!(scope, UserClaimScopeArg::User);
            assert!(owner_key.is_none());
            assert_eq!(text, "Manual summary");
            assert!(json);
        }
        _ => panic!("expected user summary edit command"),
    }

    let sources = Cli::parse_from([
        "remem",
        "user",
        "summary",
        "sources",
        "--project",
        "/repo",
        "--include-excluded",
        "--json",
    ]);
    match sources.command {
        Commands::User {
            action:
                UserAction::Summary {
                    action:
                        UserSummaryAction::Sources {
                            project,
                            scope,
                            owner_key,
                            include_excluded,
                            json,
                        },
                },
        } => {
            assert_eq!(project.as_deref(), Some("/repo"));
            assert_eq!(scope, UserClaimScopeArg::User);
            assert!(owner_key.is_none());
            assert!(include_excluded);
            assert!(json);
        }
        _ => panic!("expected user summary sources command"),
    }
}

#[test]
fn cli_parses_user_profile_export_command() {
    let cli = Cli::parse_from([
        "remem",
        "user",
        "profile",
        "export",
        "--format",
        "markdown",
        "--output",
        "profile.md",
        "--project",
        "/repo",
        "--owner-scope",
        "repo",
        "--owner-key",
        "/repo",
        "--include-suppressed",
        "--include-sensitive",
        "--include-inactive",
        "--include-deleted",
        "--include-manual-summaries",
    ]);
    match cli.command {
        Commands::User {
            action:
                UserAction::Profile {
                    action:
                        UserProfileAction::Export {
                            format,
                            output,
                            project,
                            owner_scope,
                            owner_key,
                            include_suppressed,
                            include_sensitive,
                            include_inactive,
                            include_deleted,
                            include_manual_summaries,
                        },
                },
        } => {
            assert_eq!(format, ProfileSnapshotFormatArg::Markdown);
            assert_eq!(output.as_deref(), Some(std::path::Path::new("profile.md")));
            assert_eq!(project.as_deref(), Some("/repo"));
            assert_eq!(owner_scope, UserClaimScopeArg::Repo);
            assert_eq!(owner_key.as_deref(), Some("/repo"));
            assert!(include_suppressed);
            assert!(include_sensitive);
            assert!(include_inactive);
            assert!(include_deleted);
            assert!(include_manual_summaries);
        }
        _ => panic!("expected user profile export command"),
    }
}

#[test]
fn cli_parses_user_recall_command() {
    let cli = Cli::parse_from([
        "remem",
        "user",
        "recall",
        "review recall design",
        "--project",
        "/repo",
        "--task-intent",
        "review",
        "--current-file",
        "src/user_context/recall.rs",
        "--state-key",
        "recall-design",
        "--include-sensitive",
        "--include-suppressed",
        "--limit",
        "7",
        "--budget-chars",
        "1200",
        "--json",
    ]);
    match cli.command {
        Commands::User {
            action:
                UserAction::Recall {
                    query,
                    project,
                    task_intent,
                    current_files,
                    state_keys,
                    include_sensitive,
                    include_suppressed,
                    limit,
                    budget_chars,
                    json,
                    ..
                },
        } => {
            assert_eq!(query, "review recall design");
            assert_eq!(project.as_deref(), Some("/repo"));
            assert_eq!(task_intent.as_deref(), Some("review"));
            assert_eq!(current_files, vec!["src/user_context/recall.rs"]);
            assert_eq!(state_keys, vec!["recall-design"]);
            assert!(include_sensitive);
            assert!(include_suppressed);
            assert_eq!(limit, 7);
            assert_eq!(budget_chars, 1200);
            assert!(json);
        }
        _ => panic!("expected user recall command"),
    }
}

#[test]
fn cli_parses_user_review_commands() {
    let inbox = Cli::parse_from([
        "remem",
        "user",
        "review",
        "inbox",
        "--include-resolved",
        "--status",
        "suppressed",
        "--limit",
        "7",
        "--json",
    ]);
    match inbox.command {
        Commands::User {
            action:
                UserAction::Review {
                    action:
                        UserReviewAction::Inbox {
                            include_resolved,
                            status,
                            limit,
                            json,
                        },
                },
        } => {
            assert!(include_resolved);
            assert_eq!(status.as_deref(), Some("suppressed"));
            assert_eq!(limit, 7);
            assert!(json);
        }
        _ => panic!("expected user review inbox command"),
    }

    let edit = Cli::parse_from([
        "remem",
        "user",
        "review",
        "edit",
        "42",
        "--text",
        "Prefer edited review gates",
        "--type",
        "preference",
        "--key",
        "preference:review-gates",
        "--sensitivity",
        "normal",
        "--note",
        "approved after edit",
        "--json",
    ]);
    match edit.command {
        Commands::User {
            action:
                UserAction::Review {
                    action:
                        UserReviewAction::Edit {
                            id,
                            text,
                            claim_type,
                            claim_key,
                            sensitivity,
                            note,
                            json,
                        },
                },
        } => {
            assert_eq!(id, 42);
            assert_eq!(text, "Prefer edited review gates");
            assert_eq!(claim_type, Some(UserClaimTypeArg::Preference));
            assert_eq!(claim_key.as_deref(), Some("preference:review-gates"));
            assert_eq!(sensitivity, Some(UserClaimSensitivityArg::Normal));
            assert_eq!(note.as_deref(), Some("approved after edit"));
            assert!(json);
        }
        _ => panic!("expected user review edit command"),
    }
}
