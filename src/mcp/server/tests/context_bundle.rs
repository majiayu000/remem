use std::collections::BTreeSet;

use rmcp::handler::server::wrapper::Parameters;
use serde_json::{json, Value};

use super::super::MemoryServer;
use super::{assert_mcp_error, McpErrorCode};
use crate::db::test_support::ScopedTestDataDir;
use crate::mcp::types::ContextBundleParams;

fn v1_params() -> ContextBundleParams {
    ContextBundleParams {
        schema_version: 1,
        task: "resume the context compiler work".to_string(),
        project: Some("/repo".to_string()),
        cwd: Some("/repo".to_string()),
        worktree: None,
        branch: None,
        role: None,
        as_of_epoch: Some(0),
        token_budget: Some(4_000),
        risk: None,
        include_superseded: None,
    }
}

#[test]
fn frozen_minimal_v1_request_compiles_a_versioned_bundle() -> anyhow::Result<()> {
    let _dir = ScopedTestDataDir::new("mcp-context-bundle-v1");
    let conn = crate::db::open_db()?;
    let memory_id = crate::memory::insert_memory(
        &conn,
        None,
        "/repo",
        Some("context-bundle-wire"),
        "Context compiler decision",
        "The MCP adapter returns a versioned audited bundle.",
        "decision",
        None,
    )?;
    crate::truth::test_support::seed_current_memory_proof(&conn, memory_id)?;
    drop(conn);

    // Frozen minimal JSON proves existing v1 callers can continue omitting
    // every field that has a documented default.
    let params: ContextBundleParams = serde_json::from_value(json!({
        "schema_version": 1,
        "task": "resume the context compiler work",
        "project": "/repo",
        "cwd": "/repo"
    }))?;
    let server = MemoryServer::new()?;
    let response = server
        .context_bundle(Parameters(params))
        .map_err(anyhow::Error::msg)?;
    let bundle: Value = serde_json::from_str(&response)?;

    assert_eq!(bundle["schema_version"], 1);
    assert_eq!(bundle["audit"]["schema_version"], 1);
    assert_eq!(bundle["audit"]["policy_version"], "retrieval_router_v2");
    assert_eq!(bundle["audit"]["token_budget"], 4_000);
    assert_eq!(bundle["degraded_mode"], "full");
    assert_eq!(
        bundle["current_truth"][0]["title"],
        "Context compiler decision"
    );
    assert_eq!(bundle["plan_hash"], bundle["audit"]["plan_hash"]);
    Ok(())
}

#[test]
fn v1_input_and_output_schemas_are_closed_and_pinned() -> anyhow::Result<()> {
    let server = MemoryServer::new()?;
    let tool = server
        .tool_router
        .get("context_bundle")
        .expect("context_bundle tool should be registered");

    let input = tool.input_schema.as_ref();
    assert_eq!(input.get("additionalProperties"), Some(&Value::Bool(false)));
    let input_properties = input["properties"]
        .as_object()
        .expect("input properties")
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        input_properties,
        BTreeSet::from([
            "as_of_epoch",
            "branch",
            "cwd",
            "include_superseded",
            "project",
            "risk",
            "role",
            "schema_version",
            "task",
            "token_budget",
            "worktree",
        ])
    );
    let required = input["required"]
        .as_array()
        .expect("input required")
        .iter()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(required, BTreeSet::from(["schema_version", "task"]));

    let output = tool.output_schema.as_deref().expect("output schema");
    assert_eq!(
        output.get("additionalProperties"),
        Some(&Value::Bool(false))
    );
    let output_properties = output["properties"]
        .as_object()
        .expect("output properties")
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        output_properties,
        BTreeSet::from([
            "audit",
            "current_truth",
            "degraded_mode",
            "failure_lessons",
            "memory_index",
            "plan_hash",
            "preferences",
            "recent_sessions",
            "schema_version",
            "workstreams",
        ])
    );
    Ok(())
}

#[test]
fn context_bundle_rejects_incompatible_or_invalid_v1_requests() -> anyhow::Result<()> {
    let _dir = ScopedTestDataDir::new("mcp-context-bundle-invalid");
    let server = MemoryServer::new()?;

    let mut cases = Vec::new();
    let mut unsupported = v1_params();
    unsupported.schema_version = 2;
    cases.push((unsupported, "unsupported schema_version"));
    let mut blank_task = v1_params();
    blank_task.task = "  ".to_string();
    cases.push((blank_task, "task is required"));
    let mut invalid_role = v1_params();
    invalid_role.role = Some("owner".to_string());
    cases.push((invalid_role, "unknown role"));
    let mut invalid_risk = v1_params();
    invalid_risk.risk = Some("extreme".to_string());
    cases.push((invalid_risk, "unknown risk"));
    let mut blank_project = v1_params();
    blank_project.project = Some("  ".to_string());
    cases.push((blank_project, "project must not be blank"));
    let mut blank_branch = v1_params();
    blank_branch.branch = Some("  ".to_string());
    cases.push((blank_branch, "branch must not be blank"));
    let mut zero_budget = v1_params();
    zero_budget.token_budget = Some(0);
    cases.push((zero_budget, "token_budget must be greater than zero"));
    let mut historical = v1_params();
    historical.as_of_epoch = Some(1_700_000_000);
    cases.push((historical, "historical as_of_epoch is not supported"));
    let mut superseded = v1_params();
    superseded.include_superseded = Some(true);
    cases.push((superseded, "include_superseded=true is not supported"));

    for (params, message) in cases {
        let error = server
            .context_bundle(Parameters(params))
            .expect_err("invalid request must fail");
        let json = assert_mcp_error(error, McpErrorCode::InvalidRequest, "context_bundle", false);
        assert!(
            json["error"]["message"]
                .as_str()
                .is_some_and(|actual| actual.contains(message)),
            "expected {message:?}, got {json}"
        );
    }
    Ok(())
}
