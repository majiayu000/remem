use rmcp::handler::server::wrapper::Parameters;

use super::super::types::SearchParams;
use super::MemoryServer;
use crate::db::test_support::ScopedTestDataDir;
use crate::memory_service::{resolve_local_note_path, sanitize_segment};

#[test]
fn sanitize_segment_collapses_invalid_chars() {
    let got = sanitize_segment("Harness / PR#33 -- Review Loop", "fallback", 64);
    assert_eq!(got, "harness_pr_33_review_loop");
}

#[test]
fn resolve_relative_path_outside_base_is_rejected() {
    let _dir = ScopedTestDataDir::new("mcp-path-traversal");
    // Relative path that resolves outside the allowed base must be rejected.
    let got = resolve_local_note_path("manual", Some("x"), Some("docs/test.md"));
    assert!(
        got.is_err(),
        "relative path resolving outside base should be rejected"
    );
}

#[test]
fn memory_server_new_does_not_open_database_eagerly() {
    let test_dir = ScopedTestDataDir::new("mcp-new");
    let db_path = test_dir.db_path();
    assert!(!db_path.exists());

    let _server = MemoryServer::new().expect("memory server should initialize");
    assert!(!db_path.exists());
}

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
        branch: None,
        multi_hop: Some(false),
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
        branch: None,
        multi_hop: Some(false),
    }));
    assert!(second.is_ok());
    assert!(test_dir.db_path().exists());
}
