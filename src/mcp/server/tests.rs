use rmcp::handler::server::wrapper::Parameters;
use serde_json::Value;

use super::super::types::{
    CommitLookupParams, GetObservationsParams, GovernMemoryParams, SaveMemoryParams, SearchParams,
    SessionCommitsParams,
};
use super::errors::{self, McpErrorCode, McpToolError};
use super::MemoryServer;
use crate::db::test_support::ScopedTestDataDir;
use crate::memory;
use crate::memory::raw_archive::{insert_raw_message, ROLE_USER, SOURCE_HOOK};
use crate::memory::service::{resolve_local_note_path, sanitize_segment};

fn assert_mcp_error(
    err: McpToolError,
    expected_code: McpErrorCode,
    expected_tool: &str,
    expected_retryable: bool,
) -> Value {
    assert_eq!(err.code(), expected_code);
    let json: Value = match serde_json::from_str(&err.to_string()) {
        Ok(json) => json,
        Err(parse_err) => panic!("error should be JSON: {parse_err}"),
    };
    assert_eq!(json["error"]["code"], expected_code.wire_code());
    assert_eq!(json["error"]["tool"], expected_tool);
    assert_eq!(json["error"]["retryable"], expected_retryable);
    assert!(json["error"]["message"].as_str().is_some());
    json
}

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

#[test]
fn lookup_commit_rejects_empty_sha_as_invalid_request() {
    let _dir = ScopedTestDataDir::new("mcp-commit-empty-sha");
    let server = match MemoryServer::new() {
        Ok(server) => server,
        Err(err) => panic!("memory server should initialize: {err}"),
    };

    let result = server.lookup_commit(Parameters(CommitLookupParams {
        sha: "   ".to_string(),
        project: None,
    }));

    let err = match result {
        Ok(value) => panic!("empty commit SHA should be rejected, got {value}"),
        Err(err) => err,
    };
    let json = assert_mcp_error(err, McpErrorCode::InvalidRequest, "lookup_commit", false);
    assert_eq!(json["error"]["message"], "commit SHA is required");
}

#[test]
fn commits_for_session_rejects_empty_session_id_as_invalid_request() {
    let _dir = ScopedTestDataDir::new("mcp-commits-empty-session");
    let server = match MemoryServer::new() {
        Ok(server) => server,
        Err(err) => panic!("memory server should initialize: {err}"),
    };

    let result = server.commits_for_session(Parameters(SessionCommitsParams {
        session_id: "\t ".to_string(),
        project: None,
        limit: None,
    }));

    let err = match result {
        Ok(value) => panic!("empty session_id should be rejected, got {value}"),
        Err(err) => err,
    };
    let json = assert_mcp_error(
        err,
        McpErrorCode::InvalidRequest,
        "commits_for_session",
        false,
    );
    assert_eq!(json["error"]["message"], "session_id is required");
}

#[test]
fn save_memory_local_copy_failures_are_invalid_request() {
    let test_dir = ScopedTestDataDir::new("mcp-save-local-copy-error");
    let server = match MemoryServer::new() {
        Ok(server) => server,
        Err(err) => panic!("memory server should initialize: {err}"),
    };

    let outside = server.save_memory(Parameters(SaveMemoryParams {
        text: "body".to_string(),
        title: Some("Memory".to_string()),
        project: Some("proj".to_string()),
        topic_key: None,
        memory_type: None,
        files: None,
        local_path: Some("/etc/passwd".to_string()),
        scope: None,
        branch: None,
        created_at_epoch: None,
        local_copy_enabled: Some(true),
    }));

    let err = match outside {
        Ok(value) => panic!("out-of-bounds local_path should be rejected, got {value}"),
        Err(err) => err,
    };
    let json = assert_mcp_error(err, McpErrorCode::InvalidRequest, "save_memory", false);
    assert!(json["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("outside the allowed directory")));

    let blocking_file = test_dir.path.join("manual-notes").join("proj");
    let parent = match blocking_file.parent() {
        Some(parent) => parent,
        None => panic!("blocking file should have a parent"),
    };
    if let Err(err) = std::fs::create_dir_all(parent) {
        panic!("create blocking file parent: {err}");
    }
    if let Err(err) = std::fs::write(&blocking_file, "not a directory") {
        panic!("create blocking file: {err}");
    }
    let local_path = blocking_file.join("forced-failure.md");

    let write_failure = server.save_memory(Parameters(SaveMemoryParams {
        text: "body".to_string(),
        title: Some("Memory".to_string()),
        project: Some("proj".to_string()),
        topic_key: None,
        memory_type: None,
        files: None,
        local_path: Some(local_path.display().to_string()),
        scope: None,
        branch: None,
        created_at_epoch: None,
        local_copy_enabled: Some(true),
    }));

    let err = match write_failure {
        Ok(value) => panic!("local write failure should be rejected, got {value}"),
        Err(err) => err,
    };
    let json = assert_mcp_error(err, McpErrorCode::InvalidRequest, "save_memory", false);
    assert!(json["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("local copy")));
}

#[test]
fn save_memory_response_reports_durable_feedback_shape() {
    let _dir = ScopedTestDataDir::new("mcp-save-feedback-shape");
    let server = MemoryServer::new().expect("memory server should initialize");

    let response = server
        .save_memory(Parameters(SaveMemoryParams {
            text: "MCP durable feedback body".to_string(),
            title: Some("MCP feedback".to_string()),
            project: Some("proj".to_string()),
            topic_key: Some("mcp-feedback".to_string()),
            memory_type: Some("decision".to_string()),
            files: None,
            local_path: None,
            scope: None,
            branch: Some("main".to_string()),
            created_at_epoch: None,
            local_copy_enabled: Some(false),
        }))
        .expect("save_memory should succeed");
    let json: Value = serde_json::from_str(&response).expect("response should be json");

    assert_eq!(json["status"], "saved");
    assert_eq!(json["operation"], "inserted");
    assert_eq!(json["upserted"], true);
    assert_eq!(json["project"], "proj");
    assert_eq!(json["scope"], "project");
    assert_eq!(json["topic_key"], "mcp-feedback");
    assert_eq!(json["branch"], "main");
    assert_eq!(json["local_copy"]["status"], "disabled");
    assert_eq!(json["local_status"], "disabled");
    assert!(json["local_path"].is_null());
    assert_eq!(json["next_step"]["tool"], "get_observations");
    assert_eq!(json["next_step"]["source"], "memory");
    assert_eq!(json["next_step"]["ids"][0], json["id"]);
    assert!(json["created_at_epoch"].as_i64().is_some_and(|ts| ts > 0));
    assert!(json["updated_at_epoch"].as_i64().is_some_and(|ts| ts > 0));
}

#[test]
fn govern_memory_validation_failures_are_invalid_request() {
    let _dir = ScopedTestDataDir::new("mcp-govern-validation");
    let server = match MemoryServer::new() {
        Ok(server) => server,
        Err(err) => panic!("memory server should initialize: {err}"),
    };

    let cases = [
        (
            GovernMemoryParams {
                ids: vec![],
                project: Some("proj".to_string()),
                action: "delete".to_string(),
                reason: Some("cleanup".to_string()),
                actor: None,
                dry_run: Some(false),
                confirm_destructive: Some(true),
            },
            "at least one memory id",
        ),
        (
            GovernMemoryParams {
                ids: vec![1],
                project: Some("proj".to_string()),
                action: "delete".to_string(),
                reason: Some("cleanup".to_string()),
                actor: None,
                dry_run: Some(false),
                confirm_destructive: Some(false),
            },
            "confirm_destructive=true",
        ),
        (
            GovernMemoryParams {
                ids: vec![1],
                project: Some("proj".to_string()),
                action: "delete".to_string(),
                reason: Some("   ".to_string()),
                actor: None,
                dry_run: Some(false),
                confirm_destructive: Some(true),
            },
            "explicit reason",
        ),
    ];

    for (params, expected_message) in cases {
        let result = server.govern_memory(Parameters(params));
        let err = match result {
            Ok(value) => panic!("governance validation should be rejected, got {value}"),
            Err(err) => err,
        };
        let json = assert_mcp_error(err, McpErrorCode::InvalidRequest, "govern_memory", false);
        assert!(json["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains(expected_message)));
    }
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
            branch: None,
            multi_hop: Some(false),
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
        }))
        .expect("expansion succeeds");
    let expanded_json: Value = serde_json::from_str(&expanded).expect("expanded json");
    assert_eq!(expanded_json[0]["id"], memory_id);
}

#[test]
fn get_observations_attaches_topic_trace_for_memory_topic_key() {
    let _dir = ScopedTestDataDir::new("mcp-topic-trace");
    let conn = crate::db::open_db().expect("db opens");
    conn.execute_batch("PRAGMA foreign_keys=OFF;")
        .expect("disable foreign keys for isolated topic segment fixture");
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
    crate::db::insert_topic_segment(
        &conn,
        &crate::db::TopicSegmentInput {
            host_id: 1,
            project_id: 1,
            session_row_id: 1,
            project: "/repo",
            topic_key: "aurora-contract",
            title: "Aurora setup",
            summary: "Initial contract work.",
            status: "resolved",
            segment_index: 0,
            covered_from_event_id: 10,
            covered_to_event_id: 12,
            evidence_event_ids: "[10,12]",
            files: Some(r#"["src/mcp/server/context_tools.rs"]"#),
            confidence: 0.8,
        },
    )
    .expect("topic segment insert succeeds");
    crate::db::insert_topic_segment(
        &conn,
        &crate::db::TopicSegmentInput {
            host_id: 1,
            project_id: 1,
            session_row_id: 2,
            project: "/repo",
            topic_key: "aurora-contract",
            title: "Aurora follow-up",
            summary: "Expansion returns trace.",
            status: "open",
            segment_index: 0,
            covered_from_event_id: 20,
            covered_to_event_id: 21,
            evidence_event_ids: "[20,21]",
            files: None,
            confidence: 0.75,
        },
    )
    .expect("topic segment insert succeeds");
    drop(conn);

    let server = MemoryServer::new().expect("memory server should initialize");
    let expanded = server
        .get_observations(Parameters(GetObservationsParams {
            ids: vec![memory_id],
            project: Some("/repo".to_string()),
            source: Some("memory".to_string()),
        }))
        .expect("expansion succeeds");
    let json: Value = serde_json::from_str(&expanded).expect("expanded json");

    assert_eq!(json[0]["id"], memory_id);
    assert_eq!(
        json[0]["topic_trace"]
            .as_array()
            .expect("trace array")
            .len(),
        2
    );
    assert_eq!(json[0]["topic_trace"][0]["title"], "Aurora setup");
    assert_eq!(json[0]["topic_trace"][1]["title"], "Aurora follow-up");
    assert_eq!(json[0]["topic_trace"][0]["evidence_event_ids"][0], 10);
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
            branch: Some("main".to_string()),
            multi_hop: Some(false),
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
            branch: None,
            multi_hop: Some(true),
        }))
        .expect("search succeeds");
    let json: Value = serde_json::from_str(&response).expect("search returns json");

    assert_eq!(json["mode"], "compact");
    assert_eq!(json["multi_hop"]["hops"], 1);
    assert!(json["results"].is_array());
    assert!(json["next_step"]["ids"].is_array());
}

#[test]
fn get_observations_rejects_unknown_source() {
    let _dir = ScopedTestDataDir::new("mcp-get-observations-source");
    let server = MemoryServer::new().expect("memory server should initialize");

    let result = server.get_observations(Parameters(GetObservationsParams {
        ids: vec![1],
        project: None,
        source: Some("raw_archive".to_string()),
    }));

    let err = match result {
        Ok(value) => panic!("unknown source should be rejected, got {value}"),
        Err(err) => err,
    };
    let json = assert_mcp_error(
        err,
        McpErrorCode::UnsupportedSource,
        "get_observations",
        false,
    );
    assert!(json["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("expected 'memory' or 'observation'")));
}

#[test]
fn get_observations_reports_missing_memory_ids_as_not_found() {
    let _dir = ScopedTestDataDir::new("mcp-get-observations-missing");
    let server = match MemoryServer::new() {
        Ok(server) => server,
        Err(err) => panic!("memory server should initialize: {err}"),
    };

    let result = server.get_observations(Parameters(GetObservationsParams {
        ids: vec![999_999],
        project: None,
        source: Some("memory".to_string()),
    }));

    let err = match result {
        Ok(value) => panic!("missing memory should be rejected, got {value}"),
        Err(err) => err,
    };
    let json = assert_mcp_error(err, McpErrorCode::NotFound, "get_observations", false);
    assert!(json["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("999999")));
}

#[test]
fn timeline_rejects_missing_anchor_and_query_as_invalid_request() {
    let _dir = ScopedTestDataDir::new("mcp-timeline-invalid-request");
    let server = match MemoryServer::new() {
        Ok(server) => server,
        Err(err) => panic!("memory server should initialize: {err}"),
    };

    let result = server.timeline(Parameters(super::super::types::TimelineParams {
        anchor: None,
        query: None,
        depth_before: None,
        depth_after: None,
        project: None,
    }));

    let err = match result {
        Ok(value) => panic!("missing anchor and query should be rejected, got {value}"),
        Err(err) => err,
    };
    let json = assert_mcp_error(err, McpErrorCode::InvalidRequest, "timeline", false);
    assert_eq!(json["error"]["message"], "anchor or query required");
}

#[test]
fn timeline_reports_query_miss_as_not_found() {
    let _dir = ScopedTestDataDir::new("mcp-timeline-not-found");
    let server = match MemoryServer::new() {
        Ok(server) => server,
        Err(err) => panic!("memory server should initialize: {err}"),
    };

    let result = server.timeline(Parameters(super::super::types::TimelineParams {
        anchor: None,
        query: Some("definitely-not-in-empty-db".to_string()),
        depth_before: None,
        depth_after: None,
        project: None,
    }));

    let err = match result {
        Ok(value) => panic!("query miss should be not_found, got {value}"),
        Err(err) => err,
    };
    let json = assert_mcp_error(err, McpErrorCode::NotFound, "timeline", false);
    assert!(json["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("No results for query")));
}

#[test]
fn mcp_tool_errors_report_db_open_failure_as_retryable() {
    let test_dir = ScopedTestDataDir::new("mcp-db-open-error");
    if let Err(err) = std::fs::remove_dir_all(&test_dir.path) {
        if err.kind() != std::io::ErrorKind::NotFound {
            panic!("remove temp dir: {err}");
        }
    }
    if let Err(err) = std::fs::write(&test_dir.path, "not a directory") {
        panic!("create blocking file: {err}");
    }

    let server = match MemoryServer::new() {
        Ok(server) => server,
        Err(err) => panic!("memory server should initialize: {err}"),
    };
    let result = server.search(Parameters(SearchParams {
        query: None,
        limit: Some(5),
        project: None,
        r#type: None,
        offset: Some(0),
        include_stale: Some(true),
        branch: None,
        multi_hop: Some(false),
    }));

    if let Err(err) = std::fs::remove_file(&test_dir.path) {
        if err.kind() != std::io::ErrorKind::NotFound {
            panic!("remove blocking file: {err}");
        }
    }
    let err = match result {
        Ok(value) => panic!("blocking data dir file should fail DB open, got {value}"),
        Err(err) => err,
    };
    let json = assert_mcp_error(err, McpErrorCode::DbOpenFailed, "search", true);
    assert!(json["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("DB open failed")));
}

#[test]
fn mcp_serialization_failures_use_structured_code() {
    struct FailingSerialize;

    impl serde::Serialize for FailingSerialize {
        fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            Err(serde::ser::Error::custom("forced serialization failure"))
        }
    }

    let err = match errors::to_json_pretty("search", &FailingSerialize) {
        Ok(value) => panic!("forced serializer should fail, got {value}"),
        Err(err) => err,
    };
    let json = assert_mcp_error(err, McpErrorCode::SerializationFailed, "search", false);
    assert!(json["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("forced serialization failure")));
}

#[test]
fn mcp_error_codes_are_stable() {
    let cases = [
        (McpErrorCode::InvalidRequest, "invalid_request", false),
        (McpErrorCode::NotFound, "not_found", false),
        (McpErrorCode::DbOpenFailed, "db_open_failed", true),
        (McpErrorCode::DbQueryFailed, "db_query_failed", true),
        (
            McpErrorCode::SerializationFailed,
            "serialization_failed",
            false,
        ),
        (McpErrorCode::UnsupportedSource, "unsupported_source", false),
    ];

    for (code, expected_wire_code, expected_retryable) in cases {
        let err = McpToolError::new("unit_test", code, "test message");
        let json = assert_mcp_error(err, code, "unit_test", expected_retryable);
        assert_eq!(json["error"]["code"], expected_wire_code);
    }
}
