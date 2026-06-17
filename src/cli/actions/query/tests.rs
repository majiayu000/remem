use crate::memory::{
    current_state::{
        CurrentStateAnswer, CurrentStateKeySummary, CurrentStateMemoryRef, CurrentStateResult,
        CurrentStateWhy,
    },
    edge::{MemoryEdgeReference, MemoryEdgeSummary},
    raw_archive::{insert_raw_message, RawMessage, ROLE_ASSISTANT, ROLE_USER, SOURCE_HOOK},
    service::{MultiHopMeta, SearchResultSet},
    Memory,
};
use crate::retrieval::search::{ChannelContribution, SearchExplain, SearchExplainResult};
use serde_json::Value;

use super::{
    current::render_current_state,
    raw::{
        build_raw_search_json, build_raw_search_request, render_raw_search_results,
        search_raw_archive,
    },
    search::{
        build_search_json, build_search_request, preview_raw_text, preview_text,
        render_search_results,
    },
    show::{format_memory_timestamp, ShowJson},
    why::{render_why_memory, ContextGateSummary},
};

fn sample_memory() -> Memory {
    Memory {
        id: 1,
        session_id: None,
        project: "proj".to_string(),
        topic_key: None,
        title: "Title".to_string(),
        text: format!("{}\nsecond line", "a".repeat(120)),
        memory_type: "decision".to_string(),
        files: None,
        created_at_epoch: 0,
        updated_at_epoch: 0,
        status: "active".to_string(),
        branch: None,
        scope: "project".to_string(),
    }
}

fn sample_raw() -> RawMessage {
    RawMessage {
        id: 9,
        session_id: "session-1".to_string(),
        project: "proj".to_string(),
        role: "user".to_string(),
        content: format!("{}\nsecond line", "r".repeat(120)),
        source: "hook".to_string(),
        branch: Some("main".to_string()),
        cwd: None,
        created_at_epoch: 0,
    }
}

fn sample_explain() -> SearchExplain {
    SearchExplain {
        query: "needle".to_string(),
        project: Some("proj".to_string()),
        memory_type: Some("decision".to_string()),
        branch: Some("main".to_string()),
        include_stale: false,
        limit: 10,
        offset: 0,
        fetch_limit: 33,
        expanded_terms: vec!["needle".to_string()],
        core_terms: vec!["needle".to_string()],
        claim_terms: vec!["needle".to_string()],
        fts_query: Some("\"needle\"".to_string()),
        temporal_range: None,
        temporal_field: None,
        rrf_k: 60.0,
        min_evidence_confidence: 0.62,
        filtered_result_count: 0,
        channels: vec![crate::retrieval::search::SearchExplainChannel {
            name: "fts".to_string(),
            enabled: true,
            disabled_reason: None,
            hits: vec![crate::retrieval::search::ChannelHit {
                memory_id: 1,
                rank: 1,
            }],
        }],
        results: vec![SearchExplainResult {
            memory_id: 1,
            final_rank: 1,
            final_score: 0.016393,
            evidence_confidence: 1.0,
            project: "proj".to_string(),
            scope: "project".to_string(),
            visibility: "project-local".to_string(),
            staleness: crate::memory::MemoryStalenessLabel {
                status: "active".to_string(),
                age: "fresh",
                source_anchor: "untracked".to_string(),
                label: "status=active; staleness=fresh; source_anchor=untracked".to_string(),
            },
            contributions: vec![ChannelContribution {
                channel: "fts".to_string(),
                rank: 1,
                score: 0.016393,
            }],
        }],
        has_more: false,
        raw_fallback_count: 0,
        timings: vec![],
    }
}

#[test]
fn cli_query_preview_uses_first_line_and_truncates() {
    let memory = sample_memory();

    let preview = preview_text(&memory);
    assert_eq!(preview.len(), 80);
    assert!(preview.chars().all(|ch| ch == 'a'));
}

#[test]
fn cli_query_format_memory_timestamp_handles_invalid_epoch() {
    assert_eq!(format_memory_timestamp(i64::MAX), "");
}

#[test]
fn cli_search_request_carries_service_search_options() {
    let request = build_search_request(
        "needle",
        Some("/repo"),
        Some("decision"),
        5,
        10,
        Some("feature"),
        true,
        true,
        true,
    );

    assert_eq!(request.query.as_deref(), Some("needle"));
    assert_eq!(request.project.as_deref(), Some("/repo"));
    assert_eq!(request.memory_type.as_deref(), Some("decision"));
    assert_eq!(request.limit, 5);
    assert_eq!(request.offset, 10);
    assert_eq!(request.branch.as_deref(), Some("feature"));
    assert!(request.include_stale);
    assert!(request.multi_hop);
    assert!(request.explain);
}

#[test]
fn cli_search_render_shows_multi_hop_has_more_and_raw_fallback() {
    let result = SearchResultSet {
        memories: vec![sample_memory()],
        multi_hop: Some(MultiHopMeta {
            hops: 2,
            entities_discovered: vec!["Graphiti".to_string(), "Mem0".to_string()],
        }),
        has_more: true,
        explain: None,
        raw_hits: vec![sample_raw()],
    };

    let output = render_search_results(&result, 10, 5);

    assert!(output.contains("Multi-hop: hops=2 entities=Graphiti, Mem0"));
    assert!(output.contains("Found 1 result(s):"));
    assert!(output.contains("remem show 1"));
    assert!(output.contains("remem why 1"));
    assert!(output.contains("remem search \"<query>\" --offset 15"));
    assert!(output.contains("Raw archive fallback:"));
    assert!(output.contains("[raw:9] user | proj | 1970-01-01 | branch=main"));
    assert!(output.contains(
        "use raw hits for recall only; promote durable conclusions with review/save_memory."
    ));
}

#[test]
fn cli_search_render_uses_raw_fallback_when_curated_is_empty() {
    let result = SearchResultSet {
        memories: vec![],
        multi_hop: None,
        has_more: false,
        explain: None,
        raw_hits: vec![sample_raw()],
    };

    let output = render_search_results(&result, 0, 10);

    assert!(output.contains("No curated memories found."));
    assert!(output.contains("Raw archive fallback:"));
    assert!(output.contains("use raw hits for recall only"));
}

#[test]
fn cli_search_render_includes_explain_without_memory_content_dump() {
    let result = SearchResultSet {
        memories: vec![sample_memory()],
        multi_hop: None,
        has_more: false,
        explain: Some(sample_explain()),
        raw_hits: vec![],
    };

    let output = render_search_results(&result, 0, 10);

    assert!(output.contains("Search explain:"));
    assert!(output.contains("channels:"));
    assert!(output.contains("fts: 1#1"));
    assert!(output.contains("visibility=project-local"));
    assert!(output.contains("contributions: fts#1=0.016393"));
    assert!(!output.contains("second line"));
}

#[test]
fn cli_search_render_includes_explain_for_empty_results() {
    let result = SearchResultSet {
        memories: vec![],
        multi_hop: None,
        has_more: false,
        explain: Some(sample_explain()),
        raw_hits: vec![],
    };

    let output = render_search_results(&result, 0, 10);

    assert!(output.contains("No curated memories found."));
    assert!(output.contains("remem search \"<query>\" --include-stale"));
    assert!(output.contains("remem search \"<query>\" --multi-hop"));
    assert!(output.contains("remem search \"<query>\" --project /path/to/repo"));
    assert!(output.contains("Search explain:"));
    assert!(output.contains("fts_query: Some(\"\\\"needle\\\"\")"));
}

#[test]
fn cli_current_state_render_is_compact_and_shows_conflict_evidence() {
    let result = CurrentStateResult {
        status: "unresolved_conflict".to_string(),
        state_key: "deploy-target".to_string(),
        as_of_epoch: None,
        state: Some(CurrentStateKeySummary {
            id: 10,
            owner_scope: "repo".to_string(),
            owner_key: "/repo".to_string(),
            memory_type: "decision".to_string(),
            state_key: "deploy-target".to_string(),
            state_label: Some("deploy target".to_string()),
            state_status: "active".to_string(),
            current_memory_id: Some(2),
        }),
        matches: vec![],
        current: Some(CurrentStateAnswer {
            id: 2,
            title: "Deploy target".to_string(),
            text: "Use production.\nDetailed internal rationale should stay compact.".to_string(),
            memory_type: "decision".to_string(),
            topic_key: Some("deploy-target".to_string()),
            project: "/repo".to_string(),
            scope: "project".to_string(),
            status: "active".to_string(),
            updated_at_epoch: 0,
        }),
        conflicts: vec![CurrentStateMemoryRef {
            id: 3,
            title: "Deploy target conflict".to_string(),
            memory_type: "decision".to_string(),
            topic_key: Some("deploy-target".to_string()),
            project: "/repo".to_string(),
            status: "active".to_string(),
            updated_at_epoch: 0,
            relation: None,
            reason: Some("operator conflict".to_string()),
            evidence_event_ids: vec![7],
            source_candidate_id: None,
            source_operation_id: None,
        }],
        history: vec![],
        facts: vec![],
        why: vec![CurrentStateWhy {
            edge_type: "conflicts".to_string(),
            from_memory_id: Some(3),
            to_memory_id: Some(2),
            reason: Some("operator conflict".to_string()),
            evidence_event_ids: vec![7],
            source_candidate_id: None,
            source_operation_id: None,
            created_at_epoch: 0,
        }],
    };

    let output = render_current_state(&result);

    assert!(output.contains("Current state: unresolved_conflict"));
    assert!(output.contains("[#2] Deploy target"));
    assert!(output.contains("[#3] Deploy target conflict"));
    assert!(output.contains("evidence=[7]"));
    assert!(!output.contains("Detailed internal rationale"));
}

#[test]
fn cli_query_raw_preview_uses_first_line_and_truncates() {
    let raw = sample_raw();

    let preview = preview_raw_text(&raw);

    assert_eq!(preview.len(), 100);
    assert!(preview.chars().all(|ch| ch == 'r'));
}

#[test]
fn cli_search_json_report_is_machine_parseable() -> std::result::Result<(), serde_json::Error> {
    let result = SearchResultSet {
        memories: vec![sample_memory()],
        multi_hop: Some(MultiHopMeta {
            hops: 1,
            entities_discovered: vec!["Mem0".to_string()],
        }),
        has_more: true,
        explain: Some(sample_explain()),
        raw_hits: vec![sample_raw()],
    };
    let output = build_search_json(
        "needle",
        Some("proj"),
        Some("decision"),
        3,
        6,
        Some("main"),
        true,
        true,
        true,
        &result,
    );

    let text = serde_json::to_string(&output)?;
    let parsed: Value = serde_json::from_str(&text)?;

    assert_eq!(parsed["query"], "needle");
    assert_eq!(parsed["limit"], 3);
    assert_eq!(parsed["next_offset"], 9);
    assert_eq!(parsed["results"][0]["id"], 1);
    assert_eq!(parsed["raw_hits"][0]["id"], 9);
    assert_eq!(parsed["multi_hop"]["entities_discovered"][0], "Mem0");
    assert_eq!(parsed["explain_details"]["query"], "needle");
    Ok(())
}

#[test]
fn cli_raw_search_request_carries_raw_filters() {
    let request = build_raw_search_request(
        "literal phrase",
        Some("/repo"),
        Some("main"),
        Some("user"),
        21,
        40,
    );

    assert_eq!(request.query, "literal phrase");
    assert_eq!(request.project.as_deref(), Some("/repo"));
    assert_eq!(request.branch.as_deref(), Some("main"));
    assert_eq!(request.role.as_deref(), Some("user"));
    assert_eq!(request.limit, 21);
    assert_eq!(request.offset, 40);
}

#[test]
fn cli_raw_search_uses_raw_archive_filters() -> anyhow::Result<()> {
    let conn = rusqlite::Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    insert_raw_message(
        &conn,
        "s-user-main",
        "/repo",
        ROLE_USER,
        "literal phrase from user on main",
        SOURCE_HOOK,
        Some("main"),
        Some("/repo"),
    )?;
    insert_raw_message(
        &conn,
        "s-assistant-main",
        "/repo",
        ROLE_ASSISTANT,
        "literal phrase from assistant on main",
        SOURCE_HOOK,
        Some("main"),
        Some("/repo"),
    )?;
    insert_raw_message(
        &conn,
        "s-user-feature",
        "/repo",
        ROLE_USER,
        "literal phrase from user on feature",
        SOURCE_HOOK,
        Some("feature"),
        Some("/repo"),
    )?;

    let request = build_raw_search_request(
        "literal phrase",
        Some("/repo"),
        Some("main"),
        Some("user"),
        20,
        0,
    );
    let rows = search_raw_archive(&conn, &request)?;

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].role, ROLE_USER);
    assert_eq!(rows[0].branch.as_deref(), Some("main"));
    assert!(rows[0].content.contains("literal phrase"));
    Ok(())
}

#[test]
fn cli_raw_search_render_labels_rows_as_raw_not_curated() {
    let output = render_raw_search_results(&[sample_raw()], 20, 20, true);

    assert!(output.contains("Raw archive rows (not curated memories):"));
    assert!(
        output.contains("[raw:9] user | proj | 1970-01-01 00:00 UTC | source=hook | branch=main")
    );
    assert!(output.contains("raw rows are captured chat turns, not curated memories."));
    assert!(output.contains("remem raw search \"<query>\" --offset 40"));
}

#[test]
fn cli_raw_search_render_empty_mentions_curated_search() {
    let output = render_raw_search_results(&[], 0, 20, false);

    assert!(output.contains("No raw archive rows found."));
    assert!(output.contains("Curated search may still have promoted memories"));
    assert!(output.contains("remem search \"<query>\""));
}

#[test]
fn cli_raw_search_json_report_is_machine_parseable() -> std::result::Result<(), serde_json::Error> {
    let output = build_raw_search_json(
        "literal phrase",
        Some("proj"),
        Some("main"),
        Some("user"),
        20,
        40,
        true,
        &[sample_raw()],
    );

    let text = serde_json::to_string(&output)?;
    let parsed: Value = serde_json::from_str(&text)?;

    assert_eq!(parsed["query"], "literal phrase");
    assert_eq!(parsed["project"], "proj");
    assert_eq!(parsed["branch"], "main");
    assert_eq!(parsed["role"], "user");
    assert_eq!(parsed["source_type"], "raw_archive");
    assert_eq!(
        parsed["note"],
        "raw archive rows are captured chat turns, not curated memories"
    );
    assert_eq!(parsed["count"], 1);
    assert_eq!(parsed["has_more"], true);
    assert_eq!(parsed["next_offset"], 60);
    assert_eq!(parsed["results"][0]["source_type"], "raw_archive");
    assert_eq!(parsed["results"][0]["content"], sample_raw().content);
    Ok(())
}

#[test]
fn cli_show_json_report_is_machine_parseable() -> std::result::Result<(), serde_json::Error> {
    let output = ShowJson {
        found: true,
        id: 1,
        memory: Some(sample_memory()),
        relations: None,
    };

    let text = serde_json::to_string(&output)?;
    let parsed: Value = serde_json::from_str(&text)?;

    assert_eq!(parsed["found"], true);
    assert_eq!(parsed["id"], 1);
    assert_eq!(parsed["memory"]["title"], "Title");
    Ok(())
}

#[test]
fn cli_show_json_can_expose_memory_relations() -> std::result::Result<(), serde_json::Error> {
    let output = ShowJson {
        found: true,
        id: 2,
        memory: Some(sample_memory()),
        relations: Some(MemoryEdgeSummary {
            incoming_count: 1,
            outgoing_count: 0,
            incoming: vec![MemoryEdgeReference {
                id: 7,
                edge_type: "supersedes".to_string(),
                from_memory_id: Some(1),
                to_memory_id: Some(2),
                state_key_id: None,
                source_candidate_id: Some(5),
                evidence_event_ids: vec![10, 11],
                source_operation_id: Some(9),
                confidence: Some(0.92),
                reason: Some("candidate replaces active state/topic memories".to_string()),
                created_at_epoch: 100,
            }],
            outgoing: Vec::new(),
        }),
    };

    let text = serde_json::to_string(&output)?;
    let parsed: Value = serde_json::from_str(&text)?;

    assert_eq!(parsed["relations"]["incoming_count"], 1);
    assert_eq!(
        parsed["relations"]["incoming"][0]["edge_type"],
        "supersedes"
    );
    assert_eq!(parsed["relations"]["incoming"][0]["from_memory_id"], 1);
    assert_eq!(parsed["relations"]["incoming"][0]["source_candidate_id"], 5);
    assert_eq!(
        parsed["relations"]["incoming"][0]["evidence_event_ids"][1],
        11
    );
    Ok(())
}

#[test]
fn cli_why_render_distinguishes_visibility_from_query_scoring() {
    let gate = ContextGateSummary {
        host: "codex-cli".to_string(),
        project: "proj".to_string(),
        output_mode: "suppressed".to_string(),
        emit_count: 2,
        suppress_count: 1,
        updated_at_epoch: 0,
        last_emitted_epoch: 0,
    };

    let output = render_why_memory(
        &sample_memory(),
        Some("proj"),
        Some("main"),
        Some(&gate),
        None,
    );

    assert!(output.contains("Memory #1"));
    assert!(output.contains("project match: exact proj"));
    assert!(output.contains("branch match: branchless; visible in branch-scoped search for main"));
    assert!(output.contains("type: core memory type"));
    assert!(output.contains("status: active; default search can include it"));
    assert!(output.contains("currentness: no TTL/currentness metadata available"));
    assert!(output.contains("query scoring: query-specific"));
    assert!(output.contains("context visibility: memory index candidate and core candidate"));
    assert!(output.contains("context gate: latest codex-cli output for proj: mode=suppressed"));
    assert!(output.contains("gate rows are context-output level, not per-memory proof"));
    assert!(output.contains("remem show 1"));
}
