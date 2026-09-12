use super::MemoryServer;
use super::{assert_mcp_error, McpErrorCode};
use crate::db::test_support::ScopedTestDataDir;
use crate::mcp::types::{UpdateWorkStreamParams, WorkStreamsParams};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::ServerHandler;

#[test]
fn workstream_schemas_restrict_status_values() -> anyhow::Result<()> {
    let server = MemoryServer::new()?;
    for tool in ["workstreams", "update_workstream"] {
        let route = server
            .tool_router
            .map
            .get(tool)
            .unwrap_or_else(|| panic!("{tool} should be registered"));
        let status_schema = &route.attr.input_schema["properties"]["status"];
        assert_eq!(
            status_schema["enum"],
            serde_json::json!(["active", "paused", "completed", "abandoned"]),
            "{tool} status schema should expose only the accepted values"
        );
    }
    Ok(())
}

#[test]
fn update_workstream_rejects_unknown_status_without_mutating() -> anyhow::Result<()> {
    let _dir = ScopedTestDataDir::new("mcp-update-workstream-unknown-status");
    let server = MemoryServer::new()?;
    let conn = crate::db::open_db()?;
    conn.execute(
        "INSERT INTO workstreams
         (project, title, status, created_at_epoch, updated_at_epoch)
         VALUES ('test/proj', 'Strict status', 'paused', 1, 1)",
        [],
    )?;
    let id = conn.last_insert_rowid();

    let err = server
        .update_workstream(Parameters(UpdateWorkStreamParams {
            id,
            status: Some("running".to_string()),
            next_action: None,
            blockers: None,
        }))
        .expect_err("unknown status should be rejected");
    let json = assert_mcp_error(
        err,
        McpErrorCode::InvalidRequest,
        "update_workstream",
        false,
    );
    assert!(json["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("unknown status")));

    let status: String = conn.query_row(
        "SELECT status FROM workstreams WHERE id = ?1",
        [id],
        |row| row.get(0),
    )?;
    assert_eq!(status, "paused");
    Ok(())
}

#[test]
fn update_workstream_rejects_empty_update_without_touching_timestamp() -> anyhow::Result<()> {
    let _dir = ScopedTestDataDir::new("mcp-update-workstream-empty-update");
    let server = MemoryServer::new()?;
    let conn = crate::db::open_db()?;
    conn.execute(
        "INSERT INTO workstreams
         (project, title, status, created_at_epoch, updated_at_epoch)
         VALUES ('test/proj', 'No-op update', 'active', 1, 1)",
        [],
    )?;
    let id = conn.last_insert_rowid();

    let err = server
        .update_workstream(Parameters(UpdateWorkStreamParams {
            id,
            status: None,
            next_action: None,
            blockers: None,
        }))
        .expect_err("an update field should be required");
    let json = assert_mcp_error(
        err,
        McpErrorCode::InvalidRequest,
        "update_workstream",
        false,
    );
    assert!(json["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("at least one")));

    let updated_at: i64 = conn.query_row(
        "SELECT updated_at_epoch FROM workstreams WHERE id = ?1",
        [id],
        |row| row.get(0),
    )?;
    assert_eq!(updated_at, 1);
    Ok(())
}

#[test]
fn server_instructions_match_default_all_status_workstream_listing() -> anyhow::Result<()> {
    let server = MemoryServer::new()?;
    let instructions = server
        .get_info()
        .instructions
        .expect("MCP server should publish instructions");

    assert!(
        instructions.contains("all statuses by default"),
        "runtime guidance must match the unfiltered workstreams query: {instructions}"
    );
    Ok(())
}

#[test]
fn workstreams_rejects_unknown_status_filter() -> anyhow::Result<()> {
    let _dir = ScopedTestDataDir::new("mcp-workstreams-unknown-status");
    let server = MemoryServer::new()?;

    let err = server
        .workstreams(Parameters(WorkStreamsParams {
            project: Some("test/proj".to_string()),
            status: Some("complete".to_string()),
        }))
        .expect_err("unknown status filter should be rejected");
    let json = assert_mcp_error(err, McpErrorCode::InvalidRequest, "workstreams", false);
    assert!(json["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("unknown status")));
    Ok(())
}

#[test]
fn workstreams_redacts_secret_bearing_fields_on_output_projection() -> anyhow::Result<()> {
    let _dir = ScopedTestDataDir::new("mcp-workstreams-output-redaction");
    let server = MemoryServer::new()?;
    let conn = crate::db::open_db()?;
    conn.execute(
        "INSERT INTO workstreams
         (project, title, description, status, progress, next_action, blockers,
          created_at_epoch, updated_at_epoch, owner_scope, owner_key,
          session_intent, session_topic, session_intent_source)
         VALUES ('test/proj', 'Safe listing', 'token=desc-secret', 'active',
                 'token=progress-secret', 'token=next-secret',
                 'token=blocker-secret', 1735660800, 1735660800,
                 'repo', 'test/proj', 'fix', 'token=mcp-cli-workstream-secret', 'summary')",
        [],
    )?;

    let raw = crate::workstream::query_workstreams(&conn, "test/proj", Some("active"))?;
    assert_eq!(
        raw[0].session_topic.as_deref(),
        Some("token=mcp-cli-workstream-secret")
    );

    let encoded = server
        .workstreams(Parameters(WorkStreamsParams {
            project: Some("test/proj".to_string()),
            status: Some("active".to_string()),
        }))
        .expect("workstreams listing should succeed");
    assert!(
        !encoded.contains("mcp-cli-workstream-secret"),
        "session_topic/display_label leaked secret: {encoded}"
    );
    assert!(!encoded.contains("desc-secret"), "{encoded}");
    assert!(!encoded.contains("progress-secret"), "{encoded}");
    assert!(!encoded.contains("next-secret"), "{encoded}");
    assert!(!encoded.contains("blocker-secret"), "{encoded}");
    assert!(encoded.contains("token=[REDACTED]"), "{encoded}");
    Ok(())
}

#[test]
fn workstreams_redacts_short_inline_credential_assignments_on_output() -> anyhow::Result<()> {
    let _dir = ScopedTestDataDir::new("mcp-workstreams-inline-redaction");
    let server = MemoryServer::new()?;
    let conn = crate::db::open_db()?;
    conn.execute(
        "INSERT INTO workstreams
         (project, title, description, status, progress, next_action, blockers,
          created_at_epoch, updated_at_epoch, owner_scope, owner_key,
          session_intent, session_topic, session_intent_source)
         VALUES ('test/proj', 'Investigate token=abc123', 'Fix OAuth token=short-secret',
                 'active', NULL, 'Rotate token=xyz789', NULL, 1735660800, 1735660800,
                 'repo', 'test/proj', 'fix', 'Investigate token=abc123', 'summary')",
        [],
    )?;

    let encoded = server
        .workstreams(Parameters(WorkStreamsParams {
            project: Some("test/proj".to_string()),
            status: Some("active".to_string()),
        }))
        .expect("workstreams listing should succeed");
    assert!(!encoded.contains("abc123"), "{encoded}");
    assert!(!encoded.contains("short-secret"), "{encoded}");
    assert!(!encoded.contains("xyz789"), "{encoded}");
    assert!(encoded.contains("token=[REDACTED]"), "{encoded}");
    Ok(())
}
