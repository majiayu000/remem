use super::MemoryServer;
use super::{assert_mcp_error, McpErrorCode};
use crate::db::test_support::ScopedTestDataDir;
use crate::mcp::types::{
    GetObservationsParams, SearchParams, TimelineParams, TimelineReportParams,
    UpdateWorkStreamParams, WorkStreamsParams,
};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::Tool;
use rmcp::ServerHandler;
use std::collections::BTreeSet;

#[derive(Debug)]
struct ExpectedToolMetadata {
    name: &'static str,
    title: &'static str,
    read_only: bool,
    destructive: bool,
    idempotent: bool,
    open_world: bool,
    required_output_fields: &'static [&'static str],
}

const EXPECTED_TOOL_METADATA: &[ExpectedToolMetadata] = &[
    ExpectedToolMetadata {
        name: "current_state",
        title: "Current State",
        read_only: true,
        destructive: false,
        idempotent: true,
        open_world: false,
        required_output_fields: &["status"],
    },
    ExpectedToolMetadata {
        name: "search",
        title: "Search Memories",
        read_only: true,
        destructive: false,
        idempotent: true,
        open_world: true,
        required_output_fields: &["mode", "results", "next_step"],
    },
    ExpectedToolMetadata {
        name: "recall_user_context",
        title: "Recall User Context",
        read_only: false,
        destructive: true,
        idempotent: false,
        open_world: true,
        required_output_fields: &["query", "context", "included", "dropped", "diagnostics"],
    },
    ExpectedToolMetadata {
        name: "context_bundle",
        title: "Compile Context Bundle (Experimental)",
        read_only: false,
        destructive: true,
        idempotent: false,
        open_world: false,
        required_output_fields: &[
            "schema_version",
            "plan_hash",
            "degraded_mode",
            "current_truth",
            "audit",
        ],
    },
    ExpectedToolMetadata {
        name: "timeline",
        title: "Memory Timeline",
        read_only: true,
        destructive: false,
        idempotent: true,
        open_world: true,
        required_output_fields: &["observations"],
    },
    ExpectedToolMetadata {
        name: "get_observations",
        title: "Get Observation Details",
        read_only: false,
        destructive: true,
        idempotent: false,
        open_world: false,
        required_output_fields: &["details"],
    },
    ExpectedToolMetadata {
        name: "lookup_commit",
        title: "Lookup Commit",
        read_only: false,
        destructive: true,
        idempotent: false,
        open_world: false,
        required_output_fields: &["commits"],
    },
    ExpectedToolMetadata {
        name: "commits_for_session",
        title: "List Session Commits",
        read_only: false,
        destructive: true,
        idempotent: false,
        open_world: false,
        required_output_fields: &["commits"],
    },
    ExpectedToolMetadata {
        name: "save_memory",
        title: "Save Memory",
        read_only: false,
        destructive: true,
        idempotent: false,
        open_world: true,
        required_output_fields: &["status", "operation", "next_step"],
    },
    ExpectedToolMetadata {
        name: "govern_memory",
        title: "Govern Memory",
        read_only: false,
        destructive: true,
        idempotent: false,
        open_world: false,
        required_output_fields: &["dry_run", "action", "reason", "affected"],
    },
    ExpectedToolMetadata {
        name: "timeline_report",
        title: "Timeline Report",
        read_only: true,
        destructive: false,
        idempotent: true,
        open_world: false,
        required_output_fields: &[],
    },
    ExpectedToolMetadata {
        name: "workstreams",
        title: "List Workstreams",
        read_only: true,
        destructive: false,
        idempotent: true,
        open_world: false,
        required_output_fields: &["workstreams"],
    },
    ExpectedToolMetadata {
        name: "update_workstream",
        title: "Update Workstream",
        read_only: false,
        destructive: true,
        idempotent: false,
        open_world: false,
        required_output_fields: &["id", "updated"],
    },
    ExpectedToolMetadata {
        name: "search_raw",
        title: "Search Raw Archive",
        read_only: true,
        destructive: false,
        idempotent: true,
        open_world: false,
        required_output_fields: &["query", "count", "has_more", "results"],
    },
    ExpectedToolMetadata {
        name: "list_raw_sessions",
        title: "List Raw Sessions",
        read_only: true,
        destructive: false,
        idempotent: true,
        open_world: false,
        required_output_fields: &["sample", "count", "sessions"],
    },
];

fn registered_tool<'a>(server: &'a MemoryServer, name: &str) -> &'a Tool {
    server
        .tool_router
        .get(name)
        .unwrap_or_else(|| panic!("{name} tool should be registered"))
}

fn tool_description<'a>(server: &'a MemoryServer, name: &str) -> &'a str {
    registered_tool(server, name)
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
        .collect::<BTreeSet<_>>();
    let expected = EXPECTED_TOOL_METADATA
        .iter()
        .map(|tool| tool.name)
        .collect::<BTreeSet<_>>();

    assert_eq!(actual, expected);
    Ok(())
}

#[test]
fn public_tool_metadata_matches_the_contract_matrix() -> anyhow::Result<()> {
    let server = MemoryServer::new()?;

    for expected in EXPECTED_TOOL_METADATA {
        let tool = registered_tool(&server, expected.name);
        assert_eq!(
            tool.title.as_deref(),
            Some(expected.title),
            "{} title",
            expected.name
        );

        let annotations = tool
            .annotations
            .as_ref()
            .unwrap_or_else(|| panic!("{} should publish annotations", expected.name));
        assert_eq!(
            annotations.title.as_deref(),
            Some(expected.title),
            "{} annotation title",
            expected.name
        );
        assert_eq!(
            annotations.read_only_hint,
            Some(expected.read_only),
            "{} readOnlyHint",
            expected.name
        );
        assert_eq!(
            annotations.destructive_hint,
            Some(expected.destructive),
            "{} destructiveHint",
            expected.name
        );
        assert_eq!(
            annotations.idempotent_hint,
            Some(expected.idempotent),
            "{} idempotentHint",
            expected.name
        );
        assert_eq!(
            annotations.open_world_hint,
            Some(expected.open_world),
            "{} openWorldHint",
            expected.name
        );
    }

    Ok(())
}

#[test]
fn get_observations_metadata_matches_access_accounting_side_effect() -> anyhow::Result<()> {
    let _dir = ScopedTestDataDir::new("mcp-get-observations-access-metadata");
    let server = MemoryServer::new()?;
    let tool = registered_tool(&server, "get_observations");
    let annotations = tool
        .annotations
        .as_ref()
        .expect("get_observations should publish annotations");
    assert_eq!(annotations.read_only_hint, Some(false));
    assert_eq!(annotations.destructive_hint, Some(true));
    assert_eq!(annotations.idempotent_hint, Some(false));

    let conn = crate::db::open_db()?;
    conn.execute(
        "INSERT INTO memories
         (session_id, project, title, content, memory_type, created_at_epoch,
          updated_at_epoch, status, scope, access_count)
         VALUES (NULL, '/repo', 'Accessed memory', 'body', 'decision', 1, 1,
                 'active', 'project', 0)",
        [],
    )?;
    let id = conn.last_insert_rowid();

    for _ in 0..2 {
        server
            .get_observations(Parameters(GetObservationsParams {
                ids: vec![id],
                project: Some("/repo".to_string()),
                source: Some("memory".to_string()),
                include_suppressed: None,
            }))
            .map_err(anyhow::Error::msg)?;
    }

    let (access_count, last_accessed_epoch): (i64, Option<i64>) = conn.query_row(
        "SELECT access_count, last_accessed_epoch FROM memories WHERE id = ?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(access_count, 2);
    assert!(last_accessed_epoch.is_some());
    Ok(())
}

#[test]
fn json_tools_publish_object_output_schemas_with_stable_required_fields() -> anyhow::Result<()> {
    let server = MemoryServer::new()?;

    for expected in EXPECTED_TOOL_METADATA {
        let tool = registered_tool(&server, expected.name);
        if expected.name == "timeline_report" {
            assert!(
                tool.output_schema.is_none(),
                "timeline_report should remain Markdown without outputSchema"
            );
            continue;
        }

        let schema = tool
            .output_schema
            .as_deref()
            .unwrap_or_else(|| panic!("{} should publish outputSchema", expected.name));
        assert_eq!(
            schema.get("type").and_then(serde_json::Value::as_str),
            Some("object"),
            "{} outputSchema root type",
            expected.name
        );

        let required = schema
            .get("required")
            .and_then(serde_json::Value::as_array)
            .unwrap_or_else(|| {
                panic!(
                    "{} outputSchema should declare required fields",
                    expected.name
                )
            })
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect::<BTreeSet<_>>();
        for field in expected.required_output_fields {
            assert!(
                required.contains(field),
                "{} outputSchema should require root field {field}",
                expected.name
            );
        }
    }

    Ok(())
}

#[test]
fn json_tools_publish_closed_object_output_schemas() -> anyhow::Result<()> {
    let server = MemoryServer::new()?;

    for expected in EXPECTED_TOOL_METADATA {
        if expected.name == "timeline_report" {
            continue;
        }
        let schema = registered_tool(&server, expected.name)
            .output_schema
            .as_deref()
            .unwrap_or_else(|| panic!("{} should publish outputSchema", expected.name));
        assert_eq!(
            schema.get("additionalProperties"),
            Some(&serde_json::Value::Bool(false)),
            "{} outputSchema root must reject undeclared fields",
            expected.name
        );
    }

    Ok(())
}

#[test]
fn serialized_tool_descriptors_use_mcp_camel_case_keys() -> anyhow::Result<()> {
    let server = MemoryServer::new()?;

    for expected in EXPECTED_TOOL_METADATA {
        let wire = serde_json::to_value(registered_tool(&server, expected.name))?;
        let descriptor = wire
            .as_object()
            .unwrap_or_else(|| panic!("{} descriptor should be an object", expected.name));
        assert_eq!(
            descriptor.get("title").and_then(serde_json::Value::as_str),
            Some(expected.title),
            "{} wire title",
            expected.name
        );
        assert!(
            descriptor.contains_key("annotations"),
            "{} annotations",
            expected.name
        );
        assert!(
            !descriptor.contains_key("output_schema"),
            "{} output_schema",
            expected.name
        );
        assert_eq!(
            descriptor.contains_key("outputSchema"),
            expected.name != "timeline_report",
            "{} outputSchema presence",
            expected.name
        );

        let annotations = descriptor["annotations"]
            .as_object()
            .unwrap_or_else(|| panic!("{} wire annotations should be an object", expected.name));
        assert_eq!(
            annotations.get("title").and_then(serde_json::Value::as_str),
            Some(expected.title),
            "{} wire annotation title",
            expected.name
        );
        for (camel_case, snake_case, expected_value) in [
            ("readOnlyHint", "read_only_hint", expected.read_only),
            ("destructiveHint", "destructive_hint", expected.destructive),
            ("idempotentHint", "idempotent_hint", expected.idempotent),
            ("openWorldHint", "open_world_hint", expected.open_world),
        ] {
            assert_eq!(
                annotations
                    .get(camel_case)
                    .and_then(serde_json::Value::as_bool),
                Some(expected_value),
                "{} wire {camel_case}",
                expected.name
            );
            assert!(
                !annotations.contains_key(snake_case),
                "{} should not serialize {snake_case}",
                expected.name
            );
        }
    }

    Ok(())
}

#[test]
fn search_and_context_descriptions_explain_selection_boundaries() -> anyhow::Result<()> {
    let server = MemoryServer::new()?;
    let current_state = tool_description(&server, "current_state");
    assert!(current_state.contains("Read-only"));
    assert!(current_state.contains("no_current"));
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
    assert!(!recall.contains("Read-only"));
    assert!(recall.contains("poisoning gate"));
    assert!(recall.contains("may quarantine"));
    assert!(recall.contains("unsafe legacy or session summary"));
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
fn commit_tool_descriptions_disclose_linked_summary_quarantine() -> anyhow::Result<()> {
    let server = MemoryServer::new()?;

    for name in ["lookup_commit", "commits_for_session"] {
        let description = tool_description(&server, name);
        assert!(description.contains("poisoning gate"), "{name}");
        assert!(description.contains("may quarantine"), "{name}");
        assert!(
            description.contains("unsafe linked session summary"),
            "{name}"
        );
    }

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
fn timeline_rejects_blank_query_without_selecting_recent_observation() -> anyhow::Result<()> {
    let _dir = ScopedTestDataDir::new("mcp-timeline-blank-query");
    let server = MemoryServer::new()?;

    let err = server
        .timeline(Parameters(TimelineParams {
            anchor: None,
            query: Some("  ".to_string()),
            depth_before: None,
            depth_after: None,
            project: Some("test/proj".to_string()),
        }))
        .expect_err("blank timeline query should be rejected");
    let json = assert_mcp_error(err, McpErrorCode::InvalidRequest, "timeline", false);
    assert_eq!(
        json["error"]["message"],
        "anchor or non-blank query required"
    );
    Ok(())
}

#[test]
fn timeline_report_propagates_token_economics_query_failure() -> anyhow::Result<()> {
    let _dir = ScopedTestDataDir::new("mcp-timeline-report-query-failure");
    let server = MemoryServer::new()?;
    let conn = crate::db::open_db()?;
    for session_id in ["overflow-a", "overflow-b"] {
        conn.execute(
            "INSERT INTO observations
             (memory_session_id, project, type, title, created_at_epoch, discovery_tokens)
             VALUES (?1, 'test/proj', 'decision', 'Overflow metric', 1, ?2)",
            rusqlite::params![session_id, i64::MAX],
        )?;
    }

    let err = server
        .timeline_report(Parameters(TimelineReportParams {
            project: "test/proj".to_string(),
            full: Some(false),
        }))
        .expect_err("timeline report should expose metric query failure");
    let json = assert_mcp_error(err, McpErrorCode::DbQueryFailed, "timeline_report", true);
    assert!(json["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("integer overflow")));
    Ok(())
}

#[test]
fn search_rejects_multi_hop_without_non_blank_query() -> anyhow::Result<()> {
    let _dir = ScopedTestDataDir::new("mcp-multi-hop-blank-query");
    let server = MemoryServer::new()?;

    for query in [None, Some("  ".to_string())] {
        let err = server
            .search(Parameters(SearchParams {
                query,
                limit: None,
                project: Some("test/proj".to_string()),
                r#type: None,
                offset: None,
                include_stale: None,
                include_suppressed: None,
                branch: None,
                multi_hop: Some(true),
                explain: None,
                task_intent: None,
                role: None,
                risk: None,
                token_budget: None,
                include_superseded: None,
            }))
            .expect_err("multi-hop should require a query");
        let json = assert_mcp_error(err, McpErrorCode::InvalidRequest, "search", false);
        assert!(json["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("multi_hop requires")));
    }
    Ok(())
}
