use rmcp::handler::server::wrapper::Parameters;

use super::super::types::{CommitLookupParams, SearchParams, SessionCommitsParams};
use super::MemoryServer;
use crate::db::test_support::ScopedTestDataDir;
use crate::memory::service::{resolve_local_note_path, sanitize_segment};

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

#[test]
fn commit_tools_return_git_metadata_separate_from_session_summary() {
    let _test_dir = ScopedTestDataDir::new("mcp-commit");
    let server = MemoryServer::new().expect("memory server should initialize");
    let conn = crate::db::open_db().expect("test database should open");
    let changed_files = vec!["src/git_trace.rs".to_string()];
    crate::git_trace::link_commit_to_session(
        &conn,
        &crate::git_trace::CommitLinkInput {
            metadata: crate::git_trace::CommitMetadataInput {
                project: "proj",
                repo_path: Some("/repo"),
                sha: "abcdef1234567890abcdef1234567890abcdef12",
                short_sha: Some("abcdef1"),
                branch: Some("main"),
                message: Some("Add traceability"),
                authored_at_epoch: Some(1_700_000_000),
                changed_files: &changed_files,
            },
            session_id: "content-session-1",
            memory_session_id: Some("mem-session-1"),
            source: "git_metadata",
        },
    )
    .expect("commit should link");

    let lookup = server
        .lookup_commit(Parameters(CommitLookupParams {
            sha: "abcdef1".to_string(),
            project: Some("proj".to_string()),
        }))
        .expect("lookup_commit should succeed");
    let lookup_json: serde_json::Value =
        serde_json::from_str(&lookup).expect("lookup response should be JSON");
    assert_eq!(lookup_json[0]["git"]["short_sha"], "abcdef1");
    assert_eq!(
        lookup_json[0]["sessions"][0]["session_id"],
        "content-session-1"
    );

    let session = server
        .commits_for_session(Parameters(SessionCommitsParams {
            session_id: "mem-session-1".to_string(),
            project: Some("proj".to_string()),
            limit: Some(5),
        }))
        .expect("commits_for_session should succeed");
    let session_json: serde_json::Value =
        serde_json::from_str(&session).expect("session response should be JSON");
    assert_eq!(session_json[0]["git"]["short_sha"], "abcdef1");
    assert_eq!(session_json[0]["link"]["source"], "git_metadata");
}
