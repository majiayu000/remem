use crate::memory::{
    raw_archive::RawMessage,
    service::{MultiHopMeta, SearchResultSet},
    Memory,
};
use crate::retrieval::search::{ChannelContribution, SearchExplain, SearchExplainResult};

use super::{
    search::{build_search_request, preview_raw_text, preview_text, render_search_results},
    show::format_memory_timestamp,
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
        fts_query: Some("\"needle\"".to_string()),
        temporal_range: None,
        rrf_k: 60.0,
        channels: vec![crate::retrieval::search::SearchExplainChannel {
            name: "fts".to_string(),
            hits: vec![crate::retrieval::search::ChannelHit {
                memory_id: 1,
                rank: 1,
            }],
        }],
        results: vec![SearchExplainResult {
            memory_id: 1,
            final_rank: 1,
            final_score: 0.016393,
            project: "proj".to_string(),
            scope: "project".to_string(),
            visibility: "project-local".to_string(),
            contributions: vec![ChannelContribution {
                channel: "fts".to_string(),
                rank: 1,
                score: 0.016393,
            }],
        }],
        has_more: false,
        raw_fallback_count: 0,
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
fn cli_query_raw_preview_uses_first_line_and_truncates() {
    let raw = sample_raw();

    let preview = preview_raw_text(&raw);

    assert_eq!(preview.len(), 100);
    assert!(preview.chars().all(|ch| ch == 'r'));
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

    let output = render_why_memory(&sample_memory(), Some("proj"), Some("main"), Some(&gate));

    assert!(output.contains("Memory #1"));
    assert!(output.contains("project match: exact proj"));
    assert!(output.contains("branch match: branchless; visible in branch-scoped search for main"));
    assert!(output.contains("type: core memory type"));
    assert!(output.contains("status: active; default search can include it"));
    assert!(output.contains("query scoring: query-specific"));
    assert!(output.contains("context visibility: memory index candidate and core candidate"));
    assert!(output.contains("context gate: latest codex-cli output for proj: mode=suppressed"));
    assert!(output.contains("gate rows are context-output level, not per-memory proof"));
    assert!(output.contains("remem show 1"));
}
