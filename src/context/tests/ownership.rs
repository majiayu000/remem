use rusqlite::Connection;

use crate::memory::types::tests_helper::setup_memory_schema;

use super::super::host::HostKind;
use super::super::policy::{ContextLimits, ContextPolicy};
use super::super::query::load_context_data_with_policy;
use super::super::render::{build_context_debug_trace, build_context_stats_footer};
use super::super::render::{ContextRenderStats, SectionRenderStats};
use super::super::types::ContextRequest;
use super::{insert_memory, insert_owned_memory};

#[test]
fn startup_context_excludes_unrelated_tool_and_domain_memories() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);
    let stash = "/Users/lifcc/Desktop/code/AI/tool/stash";
    let now = chrono::Utc::now().timestamp();

    insert_owned_memory(
        &conn,
        1,
        stash,
        Some("stash-dnd-decision"),
        "decision",
        "Stash DnD uses pointer sensors",
        "The Stash UI uses pointer sensors for drag and drop behavior.",
        now,
        "repo",
        stash,
        Some(stash),
        Some("stash-ui"),
    );
    insert_owned_memory(
        &conn,
        2,
        stash,
        Some("codex-sandbox"),
        "decision",
        "Codex approval mode uses workspace-write",
        "Codex CLI sandbox and approval policy are tool-level facts.",
        now + 1,
        "tool",
        "codex-cli",
        None,
        Some("codex-sandbox"),
    );
    insert_owned_memory(
        &conn,
        3,
        stash,
        Some("grok-api"),
        "discovery",
        "Grok API supports image references",
        "The xAI Grok API wrapper accepts image reference payloads.",
        now + 2,
        "domain",
        "grok-api",
        None,
        Some("grok-api"),
    );
    insert_owned_memory(
        &conn,
        4,
        stash,
        Some("warp-startup"),
        "discovery",
        "Warp launch state is macOS domain context",
        "Warp terminal launch behavior is not Stash repository context.",
        now + 3,
        "domain",
        "macos",
        None,
        Some("macos"),
    );
    insert_memory(
        &conn,
        5,
        stash,
        Some("legacy-stash-fallback"),
        "bugfix",
        "Legacy Stash memory remains compatible",
        "Rows without owner metadata still load through project fallback.",
        now - 1,
    );

    let loaded = load_context_data_with_policy(
        &conn,
        stash,
        None,
        &ContextPolicy::from_limits(ContextLimits {
            candidate_fetch_limit: 2,
            memory_index_limit: 10,
            core_item_limit: 10,
            ..ContextLimits::default()
        }),
    );
    let titles = loaded
        .memories
        .iter()
        .map(|memory| memory.title.as_str())
        .collect::<Vec<_>>();

    assert!(titles.contains(&"Stash DnD uses pointer sensors"));
    assert!(titles.contains(&"Legacy Stash memory remains compatible"));
    assert!(!titles.contains(&"Codex approval mode uses workspace-write"));
    assert!(!titles.contains(&"Grok API supports image references"));
    assert!(!titles.contains(&"Warp launch state is macOS domain context"));
    assert_eq!(loaded.owner_counts.repo, 1);
    assert_eq!(loaded.owner_counts.legacy, 1);
    assert!(loaded.owner_traces.iter().any(|trace| !trace.included
        && trace.owner_scope.as_deref() == Some("tool")
        && trace.reason == "tool_not_relevant_to_startup"));
    assert!(loaded.owner_traces.iter().any(|trace| !trace.included
        && trace.owner_scope.as_deref() == Some("domain")
        && trace.reason == "domain_not_relevant_to_startup"));
}

#[test]
fn debug_trace_reports_owner_reasons_and_counts() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);
    let stash = "/Users/lifcc/Desktop/code/AI/tool/stash";
    let now = chrono::Utc::now().timestamp();

    insert_owned_memory(
        &conn,
        1,
        stash,
        Some("stash-repo"),
        "decision",
        "Stash repo context",
        "Stash repository context should load at startup.",
        now,
        "repo",
        stash,
        Some(stash),
        Some("stash-ui"),
    );
    insert_owned_memory(
        &conn,
        2,
        stash,
        Some("codex-tool"),
        "decision",
        "Codex sandbox context",
        "Codex sandbox context should stay out of Stash startup.",
        now + 1,
        "tool",
        "codex-cli",
        None,
        Some("codex-sandbox"),
    );

    let loaded = load_context_data_with_policy(
        &conn,
        stash,
        None,
        &ContextPolicy::from_limits(ContextLimits::default()),
    );
    let stats = ContextRenderStats {
        host: "codex-cli".to_string(),
        memories_loaded: loaded.memories.len(),
        core: SectionRenderStats {
            count: loaded.memories.len(),
            chars: 128,
        },
        owner_counts: loaded.owner_counts.clone(),
        ..ContextRenderStats::default()
    };
    let request = ContextRequest {
        cwd: stash.to_string(),
        project: stash.to_string(),
        session_id: Some("sess-owner".to_string()),
        hook_source: None,
        current_branch: None,
        host: HostKind::CodexCli,
        use_colors: false,
    };

    let debug = build_context_debug_trace(&request, &ContextPolicy::from_env(), &loaded, &stats);
    assert!(debug.contains("owner counts repo=1"));
    assert!(debug.contains("owner memory id=1 scope=repo"));
    assert!(debug.contains("included reason=repo_owner_match"));
    assert!(debug.contains("owner memory id=2 scope=tool"));
    assert!(debug.contains("excluded reason=tool_not_relevant_to_startup"));

    let footer = build_context_stats_footer(&stats);
    assert!(!footer.contains("owners repo="));
    assert!(!footer.contains("tool=0 domain=0"));
}
