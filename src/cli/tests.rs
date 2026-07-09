use super::cwd::resolve_cwd_arg;
use super::types::{
    Cli, Commands, CommitAction, ContextGateAction, ImportAction, MemoryAction, MemoryCleanupType,
    MemoryGovernanceCliAction, MemorySuppressionsAction, PendingAction, ReviewAction,
};
use clap::{CommandFactory, Parser};

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
fn cli_version_reports_supported_schema_version() {
    let version = super::cli_command().render_version();

    assert!(
        version.contains(env!("CARGO_PKG_VERSION")),
        "got: {version}"
    );
    assert!(
        version.contains(&format!(
            "schema v{}",
            crate::migrate::latest_schema_version()
        )),
        "got: {version}"
    );
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
        "--include-suppressed",
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
            include_suppressed,
            multi_hop,
            explain,
            json,
        } => {
            assert_eq!(query, "Melanie rollout");
            assert_eq!(project.as_deref(), Some("personal"));
            assert_eq!(memory_type.as_deref(), Some("decision"));
            assert_eq!(limit, 10);
            assert_eq!(offset, 1);
            assert_eq!(branch.as_deref(), Some("main"));
            assert!(include_stale);
            assert!(include_suppressed);
            assert!(multi_hop);
            assert!(!explain);
            assert!(!json);
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
            acknowledge_pattern,
            reason,
            actor,
            query,
            memory_type,
            status,
            limit,
            offset,
            from_file,
            read_stdin,
            confirm_destructive,
            dry_run,
            json,
            ids,
        } => {
            assert_eq!(project.as_deref(), Some("/tmp/remem"));
            assert!(matches!(action, MemoryGovernanceCliAction::Delete));
            assert!(acknowledge_pattern.is_none());
            assert_eq!(reason.as_deref(), Some("bad memory"));
            assert_eq!(actor.as_deref(), Some("codex"));
            assert!(query.is_none());
            assert!(memory_type.is_none());
            assert!(status.is_none());
            assert_eq!(limit, 50);
            assert_eq!(offset, 0);
            assert!(from_file.is_none());
            assert!(!read_stdin);
            assert!(confirm_destructive);
            assert!(!dry_run);
            assert!(!json);
            assert_eq!(ids, vec![42, 43]);
        }
        _ => panic!("expected govern command"),
    }
}

#[test]
fn cli_parses_scriptable_json_flags() {
    let status = Cli::parse_from(["remem", "status", "--json"]);
    match status.command {
        Commands::Status { json, .. } => assert!(json),
        _ => panic!("expected status command"),
    }

    let search = Cli::parse_from(["remem", "search", "context gate", "--json"]);
    match search.command {
        Commands::Search { json, .. } => assert!(json),
        _ => panic!("expected search command"),
    }

    let show = Cli::parse_from(["remem", "show", "7", "--json"]);
    match show.command {
        Commands::Show { id, json } => {
            assert_eq!(id, 7);
            assert!(json);
        }
        _ => panic!("expected show command"),
    }

    let pending = Cli::parse_from(["remem", "pending", "list-failed", "--json"]);
    match pending.command {
        Commands::Pending {
            action:
                super::types::PendingAction::ListFailed {
                    project,
                    limit,
                    json,
                },
        } => {
            assert!(project.is_none());
            assert_eq!(limit, 20);
            assert!(json);
        }
        _ => panic!("expected pending list-failed command"),
    }

    let govern = Cli::parse_from([
        "remem",
        "govern",
        "--action",
        "stale",
        "--dry-run",
        "--json",
        "42",
    ]);
    match govern.command {
        Commands::Govern {
            action,
            dry_run,
            json,
            ids,
            ..
        } => {
            assert!(matches!(action, MemoryGovernanceCliAction::Stale));
            assert!(dry_run);
            assert!(json);
            assert_eq!(ids, vec![42]);
        }
        _ => panic!("expected govern command"),
    }
}

#[test]
fn cli_parses_governance_batch_selectors_and_id_sources() {
    let cli = Cli::parse_from([
        "remem",
        "govern",
        "--project",
        "/tmp/remem",
        "--action",
        "stale",
        "--query",
        "old migration plan",
        "--type",
        "decision",
        "--status",
        "active",
        "--limit",
        "25",
        "--offset",
        "5",
        "--from-file",
        "ids.txt",
        "--stdin",
        "42",
    ]);

    match cli.command {
        Commands::Govern {
            project,
            action,
            acknowledge_pattern,
            reason,
            actor,
            query,
            memory_type,
            status,
            limit,
            offset,
            from_file,
            read_stdin,
            confirm_destructive,
            dry_run,
            json,
            ids,
        } => {
            assert_eq!(project.as_deref(), Some("/tmp/remem"));
            assert!(matches!(action, MemoryGovernanceCliAction::Stale));
            assert!(acknowledge_pattern.is_none());
            assert!(reason.is_none());
            assert!(actor.is_none());
            assert_eq!(query.as_deref(), Some("old migration plan"));
            assert_eq!(memory_type.as_deref(), Some("decision"));
            assert_eq!(status.as_deref(), Some("active"));
            assert_eq!(limit, 25);
            assert_eq!(offset, 5);
            assert_eq!(from_file.as_deref(), Some(std::path::Path::new("ids.txt")));
            assert!(read_stdin);
            assert!(!confirm_destructive);
            assert!(!dry_run);
            assert!(!json);
            assert_eq!(ids, vec![42]);
        }
        _ => panic!("expected govern command"),
    }
}

#[test]
fn cli_parses_scope_cleanup_commands() {
    let audit = Cli::parse_from([
        "remem",
        "audit-scope",
        "--project",
        "/tmp/stash",
        "--limit",
        "25",
        "--json",
    ]);
    match audit.command {
        Commands::AuditScope {
            project,
            limit,
            json,
        } => {
            assert_eq!(project.as_deref(), Some("/tmp/stash"));
            assert_eq!(limit, 25);
            assert!(json);
        }
        _ => panic!("expected audit-scope command"),
    }

    let reroute = Cli::parse_from([
        "remem",
        "reroute",
        "--refs",
        "memory:1010,memory:1011",
        "--ids",
        "42",
        "43",
        "--owner-scope",
        "tool",
        "--owner-key",
        "codex-cli",
        "--clear-target-project",
        "--topic-domain",
        "codex-sandbox",
        "--context-class",
        "search_only",
        "--confidence",
        "0.99",
        "--reason",
        "Stash cleanup",
        "--confirm",
        "--json",
    ]);
    match reroute.command {
        Commands::Reroute {
            refs,
            ids,
            owner_scope,
            owner_key,
            target_project,
            clear_target_project,
            topic_domain,
            context_class,
            confidence,
            reason,
            confirm,
            dry_run,
            json,
        } => {
            assert_eq!(refs, vec!["memory:1010", "memory:1011"]);
            assert_eq!(ids, vec![42, 43]);
            assert_eq!(owner_scope, "tool");
            assert_eq!(owner_key, "codex-cli");
            assert!(target_project.is_none());
            assert!(clear_target_project);
            assert_eq!(topic_domain.as_deref(), Some("codex-sandbox"));
            assert_eq!(context_class.as_deref(), Some("search_only"));
            assert_eq!(confidence, Some(0.99));
            assert_eq!(reason.as_deref(), Some("Stash cleanup"));
            assert!(confirm);
            assert!(!dry_run);
            assert!(json);
        }
        _ => panic!("expected reroute command"),
    }

    let archive = Cli::parse_from([
        "remem",
        "archive",
        "--refs",
        "memory:1040",
        "workstream:2010",
        "--reason",
        "expired",
    ]);
    match archive.command {
        Commands::Archive {
            refs,
            ids,
            reason,
            confirm,
            dry_run,
            json,
        } => {
            assert_eq!(refs, vec!["memory:1040", "workstream:2010"]);
            assert!(ids.is_empty());
            assert_eq!(reason.as_deref(), Some("expired"));
            assert!(!confirm);
            assert!(!dry_run);
            assert!(!json);
        }
        _ => panic!("expected archive command"),
    }

    let merge = Cli::parse_from([
        "remem",
        "merge-preferences",
        "--project",
        "/tmp/stash",
        "--dry-run",
    ]);
    match merge.command {
        Commands::MergePreferences {
            project,
            dry_run,
            confirm,
            json,
        } => {
            assert_eq!(project.as_deref(), Some("/tmp/stash"));
            assert!(dry_run);
            assert!(!confirm);
            assert!(!json);
        }
        _ => panic!("expected merge-preferences command"),
    }

    let cleanup = Cli::parse_from([
        "remem",
        "memory",
        "cleanup",
        "--cwd",
        "/tmp/stash",
        "--type",
        "preference",
        "--dry-run",
        "--plan-out",
        "/tmp/remem-cleanup.json",
    ]);
    match cleanup.command {
        Commands::Memory {
            action:
                MemoryAction::Cleanup {
                    cwd,
                    cleanup_type,
                    all_types,
                    dry_run,
                    plan_out,
                    apply,
                    plan,
                    json,
                },
        } => {
            assert_eq!(cwd.as_deref(), Some("/tmp/stash"));
            assert_eq!(cleanup_type, Some(MemoryCleanupType::Preference));
            assert!(!all_types);
            assert!(dry_run);
            assert_eq!(
                plan_out.as_deref(),
                Some(std::path::Path::new("/tmp/remem-cleanup.json"))
            );
            assert!(!apply);
            assert!(plan.is_none());
            assert!(!json);
        }
        _ => panic!("expected memory cleanup command"),
    }

    let apply_cleanup = Cli::parse_from([
        "remem",
        "memory",
        "cleanup",
        "--apply",
        "--plan",
        "/tmp/remem-cleanup.json",
        "--json",
    ]);
    match apply_cleanup.command {
        Commands::Memory {
            action: MemoryAction::Cleanup {
                apply, plan, json, ..
            },
        } => {
            assert!(apply);
            assert_eq!(
                plan.as_deref(),
                Some(std::path::Path::new("/tmp/remem-cleanup.json"))
            );
            assert!(json);
        }
        _ => panic!("expected memory cleanup apply command"),
    }
}

#[test]
fn cli_parses_memory_suppression_and_feedback_commands() {
    let suppress = Cli::parse_from([
        "remem",
        "memory",
        "suppress",
        "memory:42",
        "--reason",
        "too noisy",
        "--json",
    ]);
    match suppress.command {
        Commands::Memory {
            action:
                MemoryAction::Suppress {
                    target,
                    reason,
                    json,
                    ..
                },
        } => {
            assert_eq!(target, "memory:42");
            assert_eq!(reason.as_deref(), Some("too noisy"));
            assert!(json);
        }
        _ => panic!("expected memory suppress command"),
    }

    let feedback = Cli::parse_from([
        "remem",
        "memory",
        "feedback",
        "memory:42",
        "--value",
        "not-relevant",
        "--session-id",
        "s1",
    ]);
    match feedback.command {
        Commands::Memory {
            action:
                MemoryAction::Feedback {
                    target,
                    value,
                    session_id,
                    ..
                },
        } => {
            assert_eq!(target, "memory:42");
            assert_eq!(value, "not-relevant");
            assert_eq!(session_id.as_deref(), Some("s1"));
        }
        _ => panic!("expected memory feedback command"),
    }

    let list = Cli::parse_from(["remem", "memory", "suppressions", "list", "--json"]);
    match list.command {
        Commands::Memory {
            action:
                MemoryAction::Suppressions {
                    action:
                        MemorySuppressionsAction::List {
                            include_inactive,
                            json,
                        },
                },
        } => {
            assert!(!include_inactive);
            assert!(json);
        }
        _ => panic!("expected memory suppressions list command"),
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

#[test]
fn cli_help_mentions_context_gate_modes_and_command_descriptions() {
    let mut command = Cli::command();
    let help = command.render_long_help().to_string();
    assert!(help.contains("Show memory store health"));
    assert!(help.contains("Inspect or repair failed pending observation rows"));
    assert!(help.contains("Inspect or switch the memory AI model profile"));

    let context = match Cli::command().find_subcommand("context") {
        Some(command) => command.clone(),
        None => panic!("context subcommand exists"),
    };
    let mut context = context;
    let context_help = context.render_long_help().to_string();
    assert!(context_help.contains("off|auto|strict|delta"));
    assert!(context_help.contains("Host profile"));
    assert!(context_help.contains("REMEM_CONTEXT_HOST"));
    assert!(context_help.contains("REMEM_CONTEXT_GATE"));
    assert!(context_help.contains("REMEM_CONTEXT_GATE_HOSTS"));
    assert!(context_help.contains("REMEM_CONTEXT_SUPPRESS_SOURCES"));
    assert!(context_help.contains("REMEM_CONTEXT_DEBUG"));
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

#[test]
fn cli_parses_context_gate_status_filters() {
    let cli = Cli::parse_from([
        "remem",
        "context-gate",
        "status",
        "--project",
        "/tmp/remem",
        "--session",
        "sess-1",
        "--limit",
        "7",
        "--json",
    ]);

    match cli.command {
        Commands::ContextGate {
            action:
                ContextGateAction::Status {
                    project,
                    session,
                    limit,
                    json,
                },
        } => {
            assert_eq!(project.as_deref(), Some("/tmp/remem"));
            assert_eq!(session.as_deref(), Some("sess-1"));
            assert_eq!(limit, 7);
            assert!(json);
        }
        _ => panic!("expected context-gate status command"),
    }
}
