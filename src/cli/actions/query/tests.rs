use crate::memory::{
    raw_archive::RawMessage,
    service::{MultiHopMeta, SearchResultSet},
    Memory,
};

use super::{
    search::{build_search_request, preview_raw_text, preview_text, render_search_results},
    show::format_memory_timestamp,
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
    );

    assert_eq!(request.query.as_deref(), Some("needle"));
    assert_eq!(request.project.as_deref(), Some("/repo"));
    assert_eq!(request.memory_type.as_deref(), Some("decision"));
    assert_eq!(request.limit, 5);
    assert_eq!(request.offset, 10);
    assert_eq!(request.branch.as_deref(), Some("feature"));
    assert!(request.include_stale);
    assert!(request.multi_hop);
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
        raw_hits: vec![sample_raw()],
    };

    let output = render_search_results(&result, 10, 5);

    assert!(output.contains("Multi-hop: hops=2 entities=Graphiti, Mem0"));
    assert!(output.contains("Found 1 result(s):"));
    assert!(output.contains("More results available; use --offset 15."));
    assert!(output.contains("Raw archive fallback:"));
    assert!(output.contains("[raw:9] user | proj | 1970-01-01 | branch=main"));
}

#[test]
fn cli_search_render_uses_raw_fallback_when_curated_is_empty() {
    let result = SearchResultSet {
        memories: vec![],
        multi_hop: None,
        has_more: false,
        raw_hits: vec![sample_raw()],
    };

    let output = render_search_results(&result, 0, 10);

    assert!(output.contains("No curated memories found."));
    assert!(output.contains("Raw archive fallback:"));
    assert!(!output.contains("No results found."));
}

#[test]
fn cli_query_raw_preview_uses_first_line_and_truncates() {
    let raw = sample_raw();

    let preview = preview_raw_text(&raw);

    assert_eq!(preview.len(), 100);
    assert!(preview.chars().all(|ch| ch == 'r'));
}
