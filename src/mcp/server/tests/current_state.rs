use rmcp::handler::server::wrapper::Parameters;
use serde_json::Value;

use super::super::super::types::CurrentStateParams;
use super::super::{errors::McpErrorCode, MemoryServer};
use super::assert_mcp_error;
use crate::db::test_support::ScopedTestDataDir;

#[test]
fn current_state_tool_returns_explicit_conflict_status() {
    let _test_dir = ScopedTestDataDir::new("mcp-current-state");
    let server = MemoryServer::new().expect("memory server should initialize");
    let conn = crate::db::open_db().expect("test database should open");
    seed_mcp_current_state_conflict(&conn);

    let response = server
        .current_state(Parameters(CurrentStateParams {
            state_key: "deploy-target".to_string(),
            project: Some("/repo".to_string()),
            r#type: Some("decision".to_string()),
            owner_scope: None,
            owner_key: None,
            as_of_epoch: None,
        }))
        .expect("current_state should return JSON");
    let json: Value = serde_json::from_str(&response).expect("current_state response is JSON");

    assert_eq!(json["status"], "unresolved_conflict");
    assert_eq!(json["current"]["id"], 2);
    assert_eq!(json["current"]["staleness"]["source_anchor"], "untracked");
    assert_eq!(json["conflicts"][0]["id"], 3);
    assert_eq!(
        json["conflicts"][0]["staleness"]["source_anchor"],
        "untracked"
    );
}

#[test]
fn current_state_tool_reports_source_anchor_error_per_ref() {
    let _test_dir = ScopedTestDataDir::new("mcp-current-state-source-error");
    let server = MemoryServer::new().expect("memory server should initialize");
    let conn = crate::db::open_db().expect("test database should open");
    seed_mcp_current_state_source_error(&conn);

    let response = server
        .current_state(Parameters(CurrentStateParams {
            state_key: "deploy-target".to_string(),
            project: Some("/repo".to_string()),
            r#type: Some("decision".to_string()),
            owner_scope: None,
            owner_key: None,
            as_of_epoch: None,
        }))
        .expect("current_state should return JSON");
    let json: Value = serde_json::from_str(&response).expect("current_state response is JSON");

    assert_eq!(json["status"], "current");
    assert_eq!(json["current"]["staleness"]["source_anchor"], "error");
    assert!(json["current"]["staleness"]["error"]
        .as_str()
        .is_some_and(|error| error.contains("source-anchor staleness")));
}

#[test]
fn current_state_tool_rejects_empty_state_key() {
    let _test_dir = ScopedTestDataDir::new("mcp-current-state-empty");
    let server = MemoryServer::new().expect("memory server should initialize");

    let err = server
        .current_state(Parameters(CurrentStateParams {
            state_key: "  ".to_string(),
            project: None,
            r#type: None,
            owner_scope: None,
            owner_key: None,
            as_of_epoch: None,
        }))
        .expect_err("empty state_key should be rejected");
    let json = assert_mcp_error(err, McpErrorCode::InvalidRequest, "current_state", false);
    assert_eq!(json["error"]["message"], "state_key is required");
}

fn seed_mcp_current_state_conflict(conn: &rusqlite::Connection) {
    conn.execute(
        "INSERT INTO memory_state_keys
         (id, owner_scope, owner_key, memory_type, state_key, state_label,
          state_status, current_memory_id, created_at_epoch, updated_at_epoch)
         VALUES (10, 'repo', '/repo', 'decision', 'deploy-target',
                 'deploy target', 'active', NULL, 1700000000, 1700000010)",
        [],
    )
    .expect("state key inserted");
    for (id, title, content) in [
        (2_i64, "Deploy target", "Use production."),
        (3_i64, "Deploy target conflict", "Use staging."),
    ] {
        conn.execute(
            "INSERT INTO memories
             (id, session_id, project, topic_key, title, content, memory_type, files,
              created_at_epoch, updated_at_epoch, status, branch, scope, source_project,
              target_project, owner_scope, owner_key, context_class, state_key_id)
             VALUES (?1, NULL, '/repo', 'deploy-target', ?2, ?3, 'decision', NULL,
                     ?4, ?4, 'active', NULL, 'project', '/repo', '/repo', 'repo',
                     '/repo', 'startup_core', 10)",
            rusqlite::params![id, title, content, 1_700_000_000_i64 + id],
        )
        .expect("memory inserted");
    }
    conn.execute(
        "UPDATE memory_state_keys SET current_memory_id = 2 WHERE id = 10",
        [],
    )
    .expect("current memory pointer updated");
}

fn seed_mcp_current_state_source_error(conn: &rusqlite::Connection) {
    seed_mcp_current_state_conflict(conn);
    conn.execute("DELETE FROM memories WHERE id = 3", [])
        .expect("conflict memory deleted");
    conn.execute(
        "UPDATE memories
         SET session_id = 'session-bad-source', files = '[not-json', branch = 'main'
         WHERE id = 2",
        [],
    )
    .expect("current memory source corrupted");
}
