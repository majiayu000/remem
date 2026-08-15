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

use super::{insert_memory, insert_owned_memory, setup_context_schema};

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
        .find(|item| item.stable_key.ends_with(":tie"))
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

#[test]
fn target_project_memory_enters_live_current_truth() {
    let conn = conn_with_schema();
    insert_owned_memory(
        &conn,
        31,
        "source/project",
        Some("targeted"),
        "decision",
        "Targeted decision",
        "This decision is routed to the active project.",
        EPOCH,
        "repo",
        "source/project",
        Some(PROJECT),
        None,
    );

    let bundle = compile_session_start_bundle(
        &conn,
        &request(),
        "/tmp/remem-current-truth-target",
        None,
        true,
    )
    .expect("compile");

    assert!(bundle
        .current_truth
        .iter()
        .any(|item| item.stable_key == "memory:31"));
}

#[test]
fn target_project_memories_keep_distinct_source_owner_slots() {
    let conn = conn_with_schema();
    for (id, owner) in [(33, "source/one"), (34, "source/two")] {
        insert_owned_memory(
            &conn,
            id,
            owner,
            Some("same-target-slot"),
            "decision",
            &format!("Decision from {owner}"),
            "Each source owns an independent routed state slot.",
            EPOCH,
            "repo",
            owner,
            Some(PROJECT),
            None,
        );
    }

    let projection =
        crate::context_bundle::project_for_scope(&conn, PROJECT, None, EPOCH, &[33, 34])
            .expect("project");
    let selected = projection
        .truths
        .iter()
        .filter_map(|truth| truth.claim.as_ref())
        .map(|claim| claim.canonical_ref.as_str())
        .collect::<Vec<_>>();
    assert!(
        selected.contains(&"memory:33"),
        "projection={projection:#?}"
    );
    assert!(
        selected.contains(&"memory:34"),
        "projection={projection:#?}"
    );
    assert_ne!(
        projection.truths[0].subject_key,
        projection.truths[1].subject_key
    );
}

#[test]
fn distinct_state_slots_with_the_same_topic_remain_distinct() {
    let conn = conn_with_schema();
    let epoch = chrono::Utc::now().timestamp();
    for (id, title) in [(41, "First slot"), (42, "Second slot")] {
        insert_memory(
            &conn,
            id,
            PROJECT,
            Some("shared-topic"),
            "decision",
            title,
            "Independent current value",
            epoch,
        );
    }
    conn.execute(
        "INSERT INTO memory_state_keys
         (id, owner_scope, owner_key, memory_type, state_key, created_at_epoch, updated_at_epoch)
         VALUES (6300042, 'repo', ?1, 'decision', 'independent-slot', ?2, ?2)",
        rusqlite::params![PROJECT, epoch],
    )
    .expect("independent state key");
    conn.execute(
        "UPDATE memories
         SET state_key_id = CASE WHEN id = 42 THEN 6300042 ELSE state_key_id END,
             source_project = ?1, target_project = ?1,
             owner_scope = 'repo', owner_key = ?1, context_class = 'startup_core'
         WHERE id IN (41, 42)",
        [PROJECT],
    )
    .expect("move second memory to independent state slot");

    let mut current_request = request();
    current_request.as_of_epoch = epoch;
    let bundle = compile_session_start_bundle(
        &conn,
        &current_request,
        "/tmp/remem-current-truth-distinct-slots",
        None,
        true,
    )
    .expect("compile");
    let selected = bundle
        .current_truth
        .iter()
        .map(|item| item.stable_key.as_str())
        .collect::<Vec<_>>();
    assert!(selected.contains(&"memory:41"), "selected={selected:?}");
    assert!(selected.contains(&"memory:42"), "selected={selected:?}");
}

#[test]
fn verified_older_winner_is_materialized_after_newest_first_preselection() {
    let conn = conn_with_schema();
    insert_memory(
        &conn,
        51,
        PROJECT,
        Some("winner-slot"),
        "decision",
        "Verified decision",
        "The user-confirmed value",
        EPOCH - 10,
    );
    conn.execute(
        "UPDATE memories SET source_trust_class = 'user_prompt' WHERE id = 51",
        [],
    )
    .expect("mark verified source");
    insert_memory(
        &conn,
        52,
        PROJECT,
        Some("winner-slot"),
        "decision",
        "Newer generated decision",
        "A lower-trust later guess",
        EPOCH,
    );
    let bundle = compile_session_start_bundle(
        &conn,
        &request(),
        "/tmp/remem-current-truth-winner-materialization",
        None,
        true,
    )
    .expect("compile");

    assert!(bundle
        .current_truth
        .iter()
        .any(|item| item.stable_key == "memory:51"));
    assert!(bundle
        .current_truth
        .iter()
        .all(|item| item.stable_key != "memory:52"));
}

#[test]
fn winner_materialization_failure_disables_the_entire_core_channel() {
    let _dir = ScopedTestDataDir::new("current-truth-materialization-failure");
    let conn = conn_with_schema();
    insert_memory(
        &conn,
        91,
        PROJECT,
        Some("materialization-failure-slot"),
        "decision",
        "Verified hidden winner",
        "Must not leak after materialization failure",
        EPOCH - 10,
    );
    conn.execute(
        "UPDATE memories SET source_trust_class = 'user_prompt' WHERE id = 91",
        [],
    )
    .expect("mark verified source");
    insert_memory(
        &conn,
        92,
        PROJECT,
        Some("materialization-failure-slot"),
        "decision",
        "Loaded lower-trust rival",
        "Must not survive fail-closed Core",
        EPOCH,
    );
    let policy = ContextPolicy::from_limits(ContextLimits::default());
    let rendered = crate::context::current_truth::with_forced_materialization_failure(|| {
        let inputs = load_context_render_inputs(&conn, &render_request(), false, &policy);
        render_context_output_from_inputs(
            &conn,
            &render_request(),
            None,
            false,
            policy,
            inputs,
            None,
            true,
        )
        .expect("render visible materialization error")
    });

    assert!(rendered.has_load_errors);
    assert!(rendered.output.contains("Context Load Errors"));
    assert!(rendered.output.contains("materialize CurrentTruth winners"));
    assert!(!rendered.output.contains("Verified hidden winner"));
    assert!(!rendered.output.contains("Loaded lower-trust rival"));
}

#[test]
fn incomplete_winner_materialization_is_an_error() {
    let conn = conn_with_schema();
    insert_memory(
        &conn,
        93,
        PROJECT,
        Some("suppressed-winner-slot"),
        "decision",
        "Winner suppressed after projection",
        "A concurrent suppression must fail closed",
        EPOCH,
    );
    let projection = crate::context_bundle::project_for_scope(&conn, PROJECT, None, EPOCH, &[93])
        .expect("project winner before suppression");
    crate::memory::suppression::create_suppression(
        &conn,
        &crate::memory::suppression::SuppressRequest {
            target: crate::memory::suppression::SuppressionTarget {
                kind: "memory".to_string(),
                id: Some(93),
                value: None,
            },
            reason: Some("concurrent test suppression"),
            actor: Some("test"),
        },
    )
    .expect("suppress projected winner");
    let mut memories = Vec::new();
    let clustered = std::collections::HashSet::from([93]);
    let error = crate::context::current_truth::materialize_winners(
        &conn,
        &mut memories,
        &projection,
        &clustered,
    )
    .expect_err("filtered winner must not count as materialized");

    assert!(error.to_string().contains("materialization incomplete"));
    assert!(memories.is_empty());
}

#[test]
fn g2_rejected_rival_cannot_supersede_an_admitted_winner() {
    let conn = conn_with_schema();
    insert_memory(
        &conn,
        71,
        PROJECT,
        Some("g2-relation-slot"),
        "decision",
        "Proven current decision",
        "This claim remains authoritative.",
        EPOCH - 10,
    );
    insert_memory(
        &conn,
        72,
        PROJECT,
        Some("g2-relation-slot"),
        "decision",
        "Unverified replacement",
        "This rejected candidate must not affect current truth.",
        EPOCH,
    );
    conn.execute(
        "UPDATE memory_candidates SET review_status = 'rejected'
         WHERE id = (SELECT source_candidate_id FROM memories WHERE id = 72)",
        [],
    )
    .expect("reject rival proof");
    conn.execute(
        "INSERT INTO memory_edges
         (edge_type, from_memory_id, to_memory_id, created_at_epoch)
         VALUES ('supersedes', 71, 72, ?1)",
        [EPOCH],
    )
    .expect("replacement edge");

    let bundle = compile_session_start_bundle(
        &conn,
        &request(),
        "/tmp/remem-current-truth-g2-relation",
        None,
        true,
    )
    .expect("compile");

    assert!(bundle
        .current_truth
        .iter()
        .any(|item| item.stable_key == "memory:71"));
    assert!(bundle
        .current_truth
        .iter()
        .all(|item| item.stable_key != "memory:72"));
}

#[test]
fn projection_failure_blocks_raw_core_from_session_start_output() {
    let _dir = ScopedTestDataDir::new("current-truth-projection-failure");
    let conn = conn_with_schema();
    insert_memory(
        &conn,
        81,
        PROJECT,
        Some("projection-failure"),
        "decision",
        "Must not fail open",
        "Raw active Core content",
        EPOCH,
    );
    conn.execute("DROP TABLE graph_edges", [])
        .expect("break projection relation load");
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
    .expect("render visible load error");

    assert!(rendered.has_load_errors);
    assert!(rendered.output.contains("Context Load Errors"));
    assert!(!rendered.output.contains("Must not fail open"));
    assert!(!rendered.output.contains("Raw active Core content"));
}

#[test]
fn live_projection_canonicalizes_historical_project_aliases() {
    let conn = conn_with_schema();
    insert_memory(
        &conn,
        61,
        "/old/project",
        Some("alias-slot"),
        "decision",
        "Historical checkout decision",
        "The claim belongs to the canonical project.",
        EPOCH,
    );
    conn.execute(
        "UPDATE memories
         SET source_project = '/old/project', target_project = '/old/project',
             owner_scope = 'repo', owner_key = '/old/project',
             context_class = 'startup_core'
         WHERE id = 61",
        [],
    )
    .expect("production-shaped historical owner");
    conn.execute_batch(
        "CREATE TABLE projects (
             id INTEGER PRIMARY KEY,
             project_path TEXT NOT NULL
         );
         CREATE TABLE project_identity_aliases (
             alias_path TEXT PRIMARY KEY,
             canonical_project_id INTEGER NOT NULL,
             status TEXT NOT NULL
         );
         INSERT INTO projects(id, project_path) VALUES(1, '/context-fixture');
         INSERT INTO project_identity_aliases(alias_path, canonical_project_id, status)
         VALUES('/old/project', 1, 'active');",
    )
    .expect("alias registry");

    let projection =
        crate::context_bundle::project_for_scope(&conn, "/context-fixture", None, EPOCH, &[61])
            .expect("project");
    let selected = projection
        .truths
        .iter()
        .find_map(|truth| truth.claim.as_ref());
    assert!(selected.is_some(), "projection={projection:#?}");
    let selected = selected.expect("asserted selected claim");
    assert_eq!(selected.canonical_ref, "memory:61");
    assert!(selected
        .subject_key
        .starts_with("state:repo:/context-fixture:decision:"));
}

fn core_section(output: &str) -> String {
    let rest = output.split_once("## Core\n").map(|(_, rest)| rest);
    let Some(rest) = rest else {
        return String::new();
    };
    rest.split("\n## ").next().unwrap_or(rest).to_string()
}
