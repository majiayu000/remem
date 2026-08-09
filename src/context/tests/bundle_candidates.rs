//! GH-932: the SessionStart loaders feeding the Context Bundle executor.
//!
//! These are the first tests that run the bundle contract over a real
//! database rather than caller-supplied candidates.

use rusqlite::Connection;

use crate::context::host::HostKind;
use crate::context::load_session_start_candidates;
use crate::context::policy::{ContextLimits, ContextPolicy};
use crate::context::render::render_context_output_from_inputs;
use crate::context::render_inputs::load_context_render_inputs;
use crate::context::types::ContextRequest as RenderRequest;
use crate::context_bundle::{
    compile_session_start_bundle, AgentRole, ChannelKind, ContextRequest, DegradedMode, ProjectRef,
    RiskClass, TrustClass, CONTEXT_BUNDLE_SCHEMA_VERSION,
};

use super::{insert_memory, setup_context_schema};

const PROJECT: &str = "demo/project";
const EPOCH: i64 = 1_710_000_000;

fn conn_with_schema() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory connection");
    setup_context_schema(&conn);
    conn
}

fn request() -> ContextRequest {
    ContextRequest {
        schema_version: CONTEXT_BUNDLE_SCHEMA_VERSION,
        task: "resume the migration work".to_string(),
        project: ProjectRef {
            key: PROJECT.to_string(),
        },
        branch: None,
        worktree: None,
        role: AgentRole::Coder,
        as_of_epoch: EPOCH,
        token_budget: 3_000,
        risk: RiskClass::Low,
        include_superseded: false,
    }
}

#[test]
fn candidates_route_memories_to_their_sections() {
    let conn = conn_with_schema();
    insert_memory(
        &conn,
        1,
        PROJECT,
        None,
        "decision",
        "Chose sqlite",
        "Body",
        EPOCH,
    );
    insert_memory(
        &conn,
        2,
        PROJECT,
        None,
        "preference",
        "Always run fmt",
        "Body",
        EPOCH,
    );
    insert_memory(
        &conn,
        3,
        PROJECT,
        None,
        "session_activity",
        "Ran the suite",
        "Body",
        EPOCH,
    );

    let candidates = load_session_start_candidates(&conn, PROJECT, "/tmp/remem-bundle-test", None)
        .expect("load");

    let channel_of = |key: &str| {
        candidates
            .iter()
            .find(|item| item.stable_key == key)
            .unwrap_or_else(|| panic!("missing candidate {key}"))
            .channel
    };
    assert_eq!(channel_of("memory:1"), ChannelKind::Core);
    assert_eq!(channel_of("memory:2"), ChannelKind::Preferences);
    assert_eq!(channel_of("memory:3"), ChannelKind::MemoryIndex);
}

#[test]
fn candidates_carry_canonical_attribution_and_project_scope() {
    let conn = conn_with_schema();
    insert_memory(
        &conn,
        1,
        PROJECT,
        None,
        "decision",
        "Chose sqlite",
        "Body",
        EPOCH,
    );

    let candidates = load_session_start_candidates(&conn, PROJECT, "/tmp/remem-bundle-test", None)
        .expect("load");
    let item = candidates
        .iter()
        .find(|item| item.stable_key == "memory:1")
        .expect("candidate");

    assert_eq!(item.canonical_ref.as_deref(), Some("memory:1"));
    assert_eq!(item.projection_ref, None);
    assert_eq!(item.project.as_deref(), Some(PROJECT));
    assert_eq!(item.trust, TrustClass::Standard);
}

/// A partial load must not reach the executor: fewer candidates is
/// indistinguishable downstream from a project that genuinely has little
/// memory.
#[test]
fn canonical_load_failure_is_an_error_not_a_shorter_list() {
    let conn = conn_with_schema();
    conn.execute("DROP TABLE memories", []).expect("drop");

    let error = load_session_start_candidates(&conn, PROJECT, "/tmp/remem-bundle-test", None)
        .expect_err("a failed canonical load must not return candidates");

    assert!(
        error.to_string().contains("canonical context load failed"),
        "unexpected error: {error}"
    );
}

#[test]
fn canonical_load_failure_produces_a_blocked_bundle() {
    let conn = conn_with_schema();
    conn.execute("DROP TABLE memories", []).expect("drop");

    let bundle =
        compile_session_start_bundle(&conn, &request(), "/tmp/remem-bundle-test", None, true)
            .expect("compile");

    assert_eq!(bundle.degraded_mode, DegradedMode::Blocked);
    assert_eq!(bundle.audit.degraded_mode, DegradedMode::Blocked);
    assert_eq!(
        bundle.audit.truncation_reason.as_deref(),
        Some("canonical_load_failed")
    );
    assert!(bundle.current_truth.is_empty());
    assert!(bundle.memory_index.is_empty());
    assert_eq!(bundle.audit.selected_count, 0);
}

#[test]
fn healthy_load_compiles_a_full_bundle_from_the_database() {
    let conn = conn_with_schema();
    insert_memory(
        &conn,
        1,
        PROJECT,
        None,
        "decision",
        "Chose sqlite",
        "Body",
        EPOCH,
    );

    let bundle =
        compile_session_start_bundle(&conn, &request(), "/tmp/remem-bundle-test", None, true)
            .expect("compile");

    assert_eq!(bundle.degraded_mode, DegradedMode::Full);
    assert!(bundle.audit.truncation_reason.is_none());
    assert_eq!(
        bundle
            .current_truth
            .iter()
            .map(|item| item.stable_key.as_str())
            .collect::<Vec<_>>(),
        vec!["memory:1"]
    );
    assert!(!bundle.audit.plan_hash.is_empty());
}

/// Without the enrichment stack the bundle degrades rather than serving
/// derived text as canonical.
#[test]
fn missing_enrichment_degrades_to_canonical_only() {
    let conn = conn_with_schema();
    insert_memory(
        &conn,
        1,
        PROJECT,
        None,
        "decision",
        "Chose sqlite",
        "Body",
        EPOCH,
    );

    let bundle =
        compile_session_start_bundle(&conn, &request(), "/tmp/remem-bundle-test", None, false)
            .expect("compile");

    assert_eq!(bundle.degraded_mode, DegradedMode::CanonicalOnly);
    // Canonical rows still survive canonical_only.
    assert_eq!(bundle.current_truth.len(), 1);
}

#[test]
fn production_bundle_renderer_is_byte_compatible_and_keeps_index_fallback() {
    let conn = conn_with_schema();
    insert_memory(
        &conn,
        1,
        PROJECT,
        None,
        "decision",
        "Primary decision",
        "Primary body",
        EPOCH + 1,
    );
    insert_memory(
        &conn,
        2,
        PROJECT,
        None,
        "decision",
        "Secondary decision",
        "Secondary body",
        EPOCH,
    );
    insert_memory(
        &conn,
        3,
        PROJECT,
        None,
        "preference",
        "Always run fmt",
        "Always run fmt before checks",
        EPOCH,
    );
    let request = RenderRequest {
        cwd: "/tmp/remem-bundle-render-test".to_string(),
        project: PROJECT.to_string(),
        session_id: Some("bundle-render-test".to_string()),
        hook_source: Some("startup".to_string()),
        current_branch: None,
        host: HostKind::CodexCli,
        use_colors: false,
    };
    let policy = ContextPolicy::from_limits(ContextLimits {
        core_item_limit: 1,
        ..ContextLimits::default()
    });
    let inputs = load_context_render_inputs(&conn, &request, false, &policy);

    let legacy = render_context_output_from_inputs(
        &conn,
        &request,
        None,
        false,
        policy.clone(),
        inputs.clone(),
        None,
        false,
    )
    .expect("legacy render");
    let bundled =
        render_context_output_from_inputs(&conn, &request, None, false, policy, inputs, None, true)
            .expect("bundle render");

    assert_eq!(bundled.output.as_bytes(), legacy.output.as_bytes());
    let bundle = bundled.context_bundle.expect("sealed context bundle");
    assert_eq!(bundle.current_truth.len(), 1);
    assert_eq!(bundle.current_truth[0].stable_key, "memory:1");
    let fallback = bundle
        .audit
        .entries
        .iter()
        .find(|entry| entry.stable_key == "memory:2")
        .expect("unselected core memory remains an index candidate");
    assert_eq!(fallback.channel, ChannelKind::MemoryIndex);
    assert_eq!(fallback.reason, "below_sessionstart_relevance_threshold");
    assert_eq!(bundle.audit.selected_count as usize, 2);
}
