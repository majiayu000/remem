use super::MemoryServer;
use super::{assert_mcp_error, McpErrorCode};
use crate::db::test_support::ScopedTestDataDir;
use crate::mcp::types::UpdateWorkStreamParams;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::ServerHandler;

fn tool_description<'a>(server: &'a MemoryServer, name: &str) -> &'a str {
    server
        .tool_router
        .map
        .get(name)
        .unwrap_or_else(|| panic!("{name} tool should be registered"))
        .attr
        .description
        .as_deref()
        .unwrap_or_else(|| panic!("{name} tool should have a description"))
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
fn get_observations_tool_description_labels_observations_as_current() -> anyhow::Result<()> {
    let server = MemoryServer::new()?;
    let description = tool_description(&server, "get_observations");

    assert!(description.contains("source='observation'"));
    assert!(description.contains("current extracted observations"));
    assert!(!description.contains("legacy observations"));
    Ok(())
}

#[test]
fn public_tool_names_remain_compact_and_compatible() -> anyhow::Result<()> {
    let server = MemoryServer::new()?;
    let actual = server
        .tool_router
        .map
        .keys()
        .map(|name| name.as_ref())
        .collect::<std::collections::BTreeSet<_>>();
    let expected = std::collections::BTreeSet::from([
        "commits_for_session",
        "current_state",
        "get_observations",
        "govern_memory",
        "list_raw_sessions",
        "lookup_commit",
        "recall_user_context",
        "save_memory",
        "search",
        "search_raw",
        "timeline",
        "timeline_report",
        "update_workstream",
        "workstreams",
    ]);

    assert_eq!(actual, expected);
    Ok(())
}

#[test]
fn search_and_context_descriptions_explain_selection_boundaries() -> anyhow::Result<()> {
    let server = MemoryServer::new()?;
    let current_state = tool_description(&server, "current_state");
    assert!(current_state.contains("Read-only"));
    assert!(current_state.contains("Use this instead of search"));
    assert!(current_state.contains("use timeline"));
    assert!(current_state.contains("JSON object"));

    let search = tool_description(&server, "search");
    assert!(search.contains("Search or list curated memories"));
    assert!(search.contains("query is optional"));
    assert!(search.contains("Use current_state"));
    assert!(search.contains("timeline"));
    assert!(search.contains("search_raw"));
    assert!(search.contains("raw_hits_error"));
    assert!(search.contains("preserves the curated results"));

    let recall = tool_description(&server, "recall_user_context");
    assert!(recall.contains("Read-only"));
    assert!(recall.contains("Use search for exhaustive memory matches"));
    assert!(recall.contains("current_state"));
    assert!(recall.contains("bounded context bundle"));
    assert!(recall.contains("current process working directory"));

    let timeline = tool_description(&server, "timeline");
    assert!(timeline.contains("Read-only"));
    assert!(timeline.contains("anchor takes precedence"));
    assert!(timeline.contains("use timeline_report"));
    assert!(timeline.contains("JSON array"));
    Ok(())
}

#[test]
fn detail_and_workstream_descriptions_disclose_side_effects_and_shapes() -> anyhow::Result<()> {
    let server = MemoryServer::new()?;
    let details = tool_description(&server, "get_observations");
    assert!(details.contains("record last-accessed metadata"));
    assert!(details.contains("Use after search, not for discovery"));
    assert!(details.contains("source defaults to 'memory'"));
    assert!(details.contains("best-effort"));
    assert!(details.contains("details still return"));
    assert!(details.contains("any missing requested ID"));

    let workstreams = tool_description(&server, "workstreams");
    assert!(workstreams.contains("Read-only"));
    assert!(workstreams.contains("required project"));
    assert!(workstreams.contains("Use update_workstream"));
    assert!(workstreams.contains("does not create, update, or delete"));

    let update = tool_description(&server, "update_workstream");
    assert!(update.contains("Mutates one existing workstream"));
    assert!(update.contains("At least one"));
    assert!(update.contains("accepts only"));
    assert!(update.contains("updated=false means no row matched"));
    assert!(update.contains("Use workstreams"));
    assert!(update.contains("does not create or delete"));
    Ok(())
}

#[test]
fn timeline_report_description_distinguishes_report_from_observation_context() -> anyhow::Result<()>
{
    let server = MemoryServer::new()?;
    let description = tool_description(&server, "timeline_report");
    assert!(description.contains("Read-only"));
    assert!(description.contains("aggregated Markdown report"));
    assert!(description.contains("full defaults to false"));
    assert!(description.contains("use timeline"));
    Ok(())
}

#[test]
fn update_workstream_schema_restricts_status_values() -> anyhow::Result<()> {
    let server = MemoryServer::new()?;
    let route = server
        .tool_router
        .map
        .get("update_workstream")
        .expect("update_workstream tool should be registered");
    let status_schema = &route.attr.input_schema["properties"]["status"];
    assert_eq!(
        status_schema["enum"],
        serde_json::json!(["active", "paused", "completed", "abandoned"]),
        "status schema should expose only the accepted values"
    );
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
