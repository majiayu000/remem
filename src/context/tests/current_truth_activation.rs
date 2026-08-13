//! G3 activation: SessionStart Core emits selected CurrentTruth claims
//! and abstains equal-trust conflicts instead of mapping raw active rows.

use crate::context::host::HostKind;
use crate::context::policy::{ContextLimits, ContextPolicy};
use crate::context::render::render_context_output_from_inputs;
use crate::context::render_inputs::load_context_render_inputs;
use crate::context::types::ContextRequest as RenderRequest;
use crate::context_bundle::{compile_session_start_bundle, SourceKind};
use crate::db::test_support::ScopedTestDataDir;
use rusqlite::Connection;

use super::{insert_memory, setup_context_schema};

const PROJECT: &str = "demo/project";
const EPOCH: i64 = 1_710_000_000;

fn conn_with_schema() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory connection");
    setup_context_schema(&conn);
    conn
}

fn request() -> crate::context_bundle::ContextRequest {
    crate::context_bundle::ContextRequest {
        schema_version: crate::context_bundle::CONTEXT_BUNDLE_SCHEMA_VERSION,
        task: "resume the migration work".to_string(),
        project: crate::context_bundle::ProjectRef {
            key: PROJECT.to_string(),
        },
        branch: None,
        worktree: None,
        role: crate::context_bundle::AgentRole::Coder,
        as_of_epoch: EPOCH,
        token_budget: 3_000,
        risk: crate::context_bundle::RiskClass::Low,
        include_superseded: false,
    }
}

fn render_request() -> RenderRequest {
    RenderRequest {
        cwd: "/tmp/remem-current-truth-activation".to_string(),
        project: PROJECT.to_string(),
        session_id: Some("current-truth-activation".to_string()),
        hook_source: Some("startup".to_string()),
        current_branch: None,
        host: HostKind::CodexCli,
        use_colors: false,
    }
}

#[test]
fn session_start_core_abstains_equal_trust_conflict_instead_of_newest_wins() {
    let _dir = ScopedTestDataDir::new("current-truth-activation-render");
    let conn = conn_with_schema();
    for (id, title, body) in [
        (11, "Left decision", "Choose A"),
        (12, "Right decision", "Choose B"),
    ] {
        insert_memory(
            &conn,
            id,
            PROJECT,
            Some("tie"),
            "decision",
            title,
            body,
            EPOCH,
        );
    }
    let policy = ContextPolicy::from_limits(ContextLimits::default());
    let inputs = load_context_render_inputs(&conn, &render_request(), false, &policy);
    let rendered = render_context_output_from_inputs(
        &conn,
        &render_request(),
        None,
        false,
        policy,
        inputs,
        None,
        true,
    )
    .expect("render");

    let core = core_section(&rendered.output);
    assert!(
        core.contains("Abstained tie: unresolved_conflict"),
        "core should emit the abstention, got:\n{core}"
    );
    assert!(
        !core.contains("Left decision") && !core.contains("Right decision"),
        "conflict claims must not render as current:\n{core}"
    );
    let bundle = rendered.context_bundle.expect("sealed bundle");
    assert!(bundle
        .current_truth
        .iter()
        .all(|item| item.stable_key != "memory:11" && item.stable_key != "memory:12"));
    let abstention = bundle
        .current_truth
        .iter()
        .find(|item| item.stable_key == "current_truth:v1:tie")
        .expect("sealed abstention");
    assert_eq!(abstention.source_kind, SourceKind::GraphDerived);
}

#[test]
fn raw_active_core_row_does_not_bypass_unselected_current_truth() {
    let conn = conn_with_schema();
    insert_memory(
        &conn,
        21,
        PROJECT,
        Some("db"),
        "decision",
        "Use sqlite",
        "sqlite is current",
        EPOCH,
    );
    insert_memory(
        &conn,
        22,
        PROJECT,
        Some("db"),
        "decision",
        "Use postgres",
        "postgres is newer but unverified rival",
        EPOCH,
    );
    let bundle = compile_session_start_bundle(
        &conn,
        &request(),
        "/tmp/remem-current-truth-bypass",
        None,
        true,
    )
    .expect("compile");
    assert!(
        bundle
            .current_truth
            .iter()
            .all(|item| item.stable_key != "memory:21" && item.stable_key != "memory:22"),
        "equal-trust raw-active rivals must not occupy current_truth: {:?}",
        bundle
            .current_truth
            .iter()
            .map(|item| item.stable_key.as_str())
            .collect::<Vec<_>>()
    );
    assert!(bundle.audit.entries.iter().any(|entry| !entry.selected
        && (entry.stable_key == "memory:21" || entry.stable_key == "memory:22")));
}

fn core_section(output: &str) -> String {
    let rest = output.split_once("## Core\n").map(|(_, rest)| rest);
    let Some(rest) = rest else {
        return String::new();
    };
    rest.split("\n## ").next().unwrap_or(rest).to_string()
}
