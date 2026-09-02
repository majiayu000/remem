use std::collections::BTreeMap;

use rmcp::handler::server::wrapper::Parameters;
use serde_json::Value;

use super::{assert_mcp_error, McpErrorCode, MemoryServer};
use crate::db::test_support::ScopedTestDataDir;
use crate::mcp::types::{GovernMemoryParams, UserRecallParams};

fn insert_governance_fixture(topic: &str, title: &str) -> (i64, i64) {
    let conn = crate::db::open_db().expect("database should open");
    let id = crate::memory::insert_memory(
        &conn,
        Some("session-1"),
        "proj",
        Some(topic),
        title,
        "Guard this memory governance operation.",
        "decision",
        None,
    )
    .expect("memory should insert");
    let version = conn
        .query_row("SELECT version FROM memories WHERE id = ?1", [id], |row| {
            row.get(0)
        })
        .expect("version should load");
    (id, version)
}

#[test]
fn recall_user_context_requires_explicit_project_or_cwd() {
    let _dir = ScopedTestDataDir::new("mcp-user-recall-explicit-scope");
    let server = MemoryServer::new().expect("memory server should initialize");

    let err = server
        .recall_user_context(Parameters(UserRecallParams {
            query: "recall MCP".to_string(),
            project: None,
            cwd: None,
            task_intent: None,
            current_files: None,
            host: None,
            owner_scope: None,
            owner_key: None,
            state_keys: None,
            include_sensitive: None,
            include_suppressed: None,
            limit: None,
            budget_chars: None,
        }))
        .expect_err("recall without explicit scope should fail");

    let json = assert_mcp_error(
        err,
        McpErrorCode::InvalidRequest,
        "recall_user_context",
        false,
    );
    assert_eq!(json["error"]["message"], "project or cwd is required");
}

#[test]
fn govern_memory_dry_run_reports_current_versions() {
    let _dir = ScopedTestDataDir::new("mcp-govern-version-preview");
    let (memory_id, version) = insert_governance_fixture("version-preview", "Version preview");
    let server = MemoryServer::new().expect("memory server should initialize");

    let response = server
        .govern_memory(Parameters(GovernMemoryParams {
            ids: vec![memory_id],
            project: Some("proj".to_string()),
            action: "stale".to_string(),
            acknowledge_pattern: None,
            reason: None,
            actor: None,
            dry_run: Some(true),
            confirm_destructive: None,
            expected_versions: None,
        }))
        .expect("dry-run should succeed");
    let json: Value = serde_json::from_str(&response).expect("response should be json");
    assert_eq!(json["expected_versions"][memory_id.to_string()], version);

    let response = server
        .govern_memory(Parameters(GovernMemoryParams {
            ids: vec![memory_id],
            project: Some("proj".to_string()),
            action: "stale".to_string(),
            acknowledge_pattern: None,
            reason: Some("decision is obsolete".to_string()),
            actor: Some("test".to_string()),
            dry_run: Some(false),
            confirm_destructive: Some(true),
            expected_versions: Some(BTreeMap::from([(memory_id, version)])),
        }))
        .expect("matching expected version should permit mutation");
    let json: Value = serde_json::from_str(&response).expect("response should be json");
    assert!(json.get("expected_versions").is_none());
    assert_eq!(json["affected"][0]["new_status"], "stale");
}

#[test]
fn govern_memory_rejects_stale_versions_without_partial_batch() {
    let _dir = ScopedTestDataDir::new("mcp-govern-stale-version");
    let (first, first_version) = insert_governance_fixture("version-first", "Version first");
    let (second, second_version) = insert_governance_fixture("version-second", "Version second");
    let server = MemoryServer::new().expect("memory server should initialize");

    let err = server
        .govern_memory(Parameters(GovernMemoryParams {
            ids: vec![first, second],
            project: Some("proj".to_string()),
            action: "stale".to_string(),
            acknowledge_pattern: None,
            reason: Some("replace outdated decisions".to_string()),
            actor: Some("test".to_string()),
            dry_run: Some(false),
            confirm_destructive: Some(true),
            expected_versions: Some(BTreeMap::from([
                (first, first_version),
                (second, second_version + 1),
            ])),
        }))
        .expect_err("stale expected version should fail");
    let json = assert_mcp_error(err, McpErrorCode::InvalidRequest, "govern_memory", false);
    assert!(json["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("expected_versions")));

    let conn = crate::db::open_db().expect("database should reopen");
    for id in [first, second] {
        let status: String = conn
            .query_row("SELECT status FROM memories WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .expect("status should load");
        assert_eq!(status, "active");
    }
    let audit_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM events WHERE event_type = 'memory_governance'",
            [],
            |row| row.get(0),
        )
        .expect("audit count should load");
    assert_eq!(audit_count, 0);
}

#[test]
fn govern_memory_input_schema_exposes_per_id_expected_versions() -> anyhow::Result<()> {
    let server = MemoryServer::new()?;
    let tool = server
        .tool_router
        .get("govern_memory")
        .expect("govern_memory tool should be registered");
    let wire = serde_json::to_value(tool)?;
    let properties = wire["inputSchema"]["properties"]
        .as_object()
        .expect("govern_memory input schema should expose object properties");
    assert!(properties.contains_key("expected_versions"));

    let parsed = serde_json::from_value::<GovernMemoryParams>(serde_json::json!({
        "ids": [7],
        "action": "stale",
        "expected_versions": { "7": 3 }
    }))?;
    assert_eq!(
        parsed
            .expected_versions
            .and_then(|versions| versions.get(&7).copied()),
        Some(3)
    );
    Ok(())
}
