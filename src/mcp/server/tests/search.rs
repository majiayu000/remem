use rmcp::handler::server::wrapper::Parameters;
use serde_json::Value;

use super::super::super::types::{GetObservationsParams, SearchParams};
use super::super::MemoryServer;
use crate::db::test_support::ScopedTestDataDir;
use crate::memory;
use crate::memory::raw_archive::{insert_raw_message, ROLE_USER, SOURCE_HOOK};

#[test]
fn search_reopens_database_after_file_removal() {
    let test_dir = ScopedTestDataDir::new("mcp-search");
    let server = MemoryServer::new().expect("memory server should initialize");

    let first = server.search(Parameters(SearchParams {
        query: None,
        limit: Some(5),
        project: None,
        r#type: None,
        offset: Some(0),
        include_stale: Some(true),
        include_suppressed: None,
        branch: None,
        multi_hop: Some(false),
        explain: None,
    }));
    assert!(first.is_ok());
    assert!(test_dir.db_path().exists());

    test_dir.remove_db_files();
    assert!(!test_dir.db_path().exists());

    let second = server.search(Parameters(SearchParams {
        query: None,
        limit: Some(5),
        project: None,
        r#type: None,
        offset: Some(0),
        include_stale: Some(true),
        include_suppressed: None,
        branch: None,
        multi_hop: Some(false),
        explain: None,
    }));
    assert!(second.is_ok());
    assert!(test_dir.db_path().exists());
}

#[test]
fn search_returns_stable_compact_envelope_with_expansion_hint() {
    let _dir = ScopedTestDataDir::new("mcp-search-envelope");
    let conn = crate::db::open_db().expect("db opens");
    let memory_id = memory::insert_memory(
        &conn,
        Some("session-1"),
        "/repo",
        Some("aurora-contract"),
        "Aurora contract decision",
        "The aurora recall contract keeps search compact before expansion.",
        "decision",
        None,
    )
    .expect("memory insert succeeds");
    drop(conn);

    let server = MemoryServer::new().expect("memory server should initialize");
    let response = server
        .search(Parameters(SearchParams {
            query: Some("aurora".to_string()),
            limit: Some(5),
            project: Some("/repo".to_string()),
            r#type: None,
            offset: Some(0),
            include_stale: Some(true),
            include_suppressed: None,
            branch: None,
            multi_hop: Some(false),
            explain: None,
        }))
        .expect("search succeeds");
    let json: Value = serde_json::from_str(&response).expect("search returns json");

    assert_eq!(json["mode"], "compact");
    assert!(json["results"].is_array());
    assert_eq!(json["results"][0]["id"], memory_id);
    assert_eq!(json["results"][0]["source"], "memory");
    assert_eq!(json["results"][0]["source_type"], "memory");
    assert_eq!(json["next_step"]["tool"], "get_observations");
    assert_eq!(json["next_step"]["source"], "memory");
    assert_eq!(json["next_step"]["ids"][0], memory_id);
    assert_eq!(json["pagination"]["has_more"], false);

    let expanded = server
        .get_observations(Parameters(GetObservationsParams {
            ids: vec![memory_id],
            project: Some("/repo".to_string()),
            source: json["next_step"]["source"].as_str().map(str::to_string),
            include_suppressed: None,
        }))
        .expect("expansion succeeds");
    let expanded_json: Value = serde_json::from_str(&expanded).expect("expanded json");
    assert_eq!(expanded_json[0]["id"], memory_id);
}

#[test]
fn search_next_step_preserves_include_suppressed_for_audit_expansion() {
    let _dir = ScopedTestDataDir::new("mcp-search-include-suppressed-next-step");
    let conn = crate::db::open_db().expect("db opens");
    let memory_id = memory::insert_memory(
        &conn,
        Some("session-include-suppressed"),
        "/repo",
        None,
        "Suppressed audit target",
        "suppressed-audit-needle should expand only with audit flag.",
        "decision",
        None,
    )
    .expect("memory insert succeeds");
    crate::memory::suppression::create_suppression(
        &conn,
        &crate::memory::suppression::SuppressRequest {
            target: crate::memory::suppression::parse_target(&format!("memory:{memory_id}"))
                .expect("target parses"),
            reason: Some("not useful"),
            actor: Some("test"),
        },
    )
    .expect("suppression insert succeeds");
    drop(conn);

    let server = MemoryServer::new().expect("memory server should initialize");
    let response = server
        .search(Parameters(SearchParams {
            query: Some("suppressed-audit-needle".to_string()),
            limit: Some(5),
            project: Some("/repo".to_string()),
            r#type: None,
            offset: Some(0),
            include_stale: None,
            include_suppressed: Some(true),
            branch: None,
            multi_hop: Some(false),
            explain: None,
        }))
        .expect("search succeeds");
    let json: Value = serde_json::from_str(&response).expect("search returns json");

    assert_eq!(json["results"][0]["id"], memory_id);
    assert_eq!(json["next_step"]["include_suppressed"], true);
}

#[test]
fn search_labels_sparse_result_raw_fallback_as_raw_archive() {
    let _dir = ScopedTestDataDir::new("mcp-search-raw-fallback");
    let conn = crate::db::open_db().expect("db opens");
    insert_raw_message(
        &conn,
        "session-raw",
        "/repo",
        ROLE_USER,
        "literal fallback needle only exists in raw archive",
        SOURCE_HOOK,
        Some("main"),
        None,
    )
    .expect("raw insert succeeds");
    drop(conn);

    let server = MemoryServer::new().expect("memory server should initialize");
    let response = server
        .search(Parameters(SearchParams {
            query: Some("needle".to_string()),
            limit: Some(5),
            project: Some("/repo".to_string()),
            r#type: None,
            offset: Some(0),
            include_stale: Some(true),
            include_suppressed: None,
            branch: Some("main".to_string()),
            multi_hop: Some(false),
            explain: None,
        }))
        .expect("search succeeds");
    let json: Value = serde_json::from_str(&response).expect("search returns json");

    assert_eq!(json["mode"], "compact");
    assert_eq!(json["raw_hits"][0]["source_type"], "raw_archive");
    assert_eq!(json["raw_hits"][0]["source"], SOURCE_HOOK);
    assert!(json["raw_hits_note"]
        .as_str()
        .expect("raw note should be present")
        .contains("not curated memories"));
}

#[test]
fn search_preserves_multi_hop_metadata_in_compact_envelope() {
    let _dir = ScopedTestDataDir::new("mcp-search-multi-hop");
    let server = MemoryServer::new().expect("memory server should initialize");

    let response = server
        .search(Parameters(SearchParams {
            query: None,
            limit: Some(5),
            project: Some("/repo".to_string()),
            r#type: None,
            offset: Some(0),
            include_stale: Some(true),
            include_suppressed: None,
            branch: None,
            multi_hop: Some(true),
            explain: None,
        }))
        .expect("search succeeds");
    let json: Value = serde_json::from_str(&response).expect("search returns json");

    assert_eq!(json["mode"], "compact");
    assert_eq!(json["multi_hop"]["hops"], 1);
    assert!(json["results"].is_array());
    assert!(json["next_step"]["ids"].is_array());
}
