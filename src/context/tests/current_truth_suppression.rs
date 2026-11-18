//! Policy suppression and live CurrentTruth winner selection.
//!
//! A policy-suppressed memory is not visible to the live projection, so a
//! visible rival in the same slot wins and the rest of SessionStart Core still
//! renders. Before this contract the suppressed row won the slot, winner
//! materialization filtered it out, and the fail-closed path emptied Core.

use crate::context::host::HostKind;
use crate::context::policy::{ContextLimits, ContextPolicy};
use crate::context::render::{render_context_output_from_inputs, RenderedContext};
use crate::context::render_inputs::load_context_render_inputs;
use crate::context::types::ContextRequest as RenderRequest;
use crate::context_bundle::{compile_session_start_bundle, DegradedMode};
use crate::db::test_support::ScopedTestDataDir;
use crate::memory::suppression::{create_suppression, SuppressRequest, SuppressionTarget};
use rusqlite::{params, Connection};

use super::{insert_memory, insert_memory_with_branch, setup_context_schema};

const PROJECT: &str = "demo/project";
const UNRELATED: i64 = 81;
const WINNER: i64 = 91;
const RIVAL: i64 = 92;
const UNRELATED_TITLE: &str = "Unrelated visible decision";
const WINNER_TITLE: &str = "Suppressed verified winner";
const RIVAL_TITLE: &str = "Visible lower-trust rival";
const SEMANTIC_STATE_KEY_ID: i64 = 6_300_091;

/// Core renders a second decision only while it is recent enough to clear the
/// additional-item score floor, so fixtures are dated just before the render.
fn recent_epoch() -> i64 {
    chrono::Utc::now().timestamp() - 60
}

fn conn_with_schema() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory connection");
    setup_context_schema(&conn);
    conn
}

fn render_request(branch: Option<&str>) -> RenderRequest {
    RenderRequest {
        cwd: "/tmp/remem-current-truth-suppression".to_string(),
        project: PROJECT.to_string(),
        session_id: Some("current-truth-suppression".to_string()),
        hook_source: Some("startup".to_string()),
        current_branch: branch.map(str::to_string),
        host: HostKind::CodexCli,
        use_colors: false,
    }
}

fn render(conn: &Connection, branch: Option<&str>) -> RenderedContext {
    let request = render_request(branch);
    let policy = ContextPolicy::from_limits(ContextLimits::default());
    let inputs = load_context_render_inputs(conn, &request, false, &policy);
    render_context_output_from_inputs(conn, &request, None, false, policy, inputs, None, true)
        .expect("render SessionStart context")
}

fn bundle_request() -> crate::context_bundle::ContextRequest {
    crate::context_bundle::ContextRequest {
        schema_version: crate::context_bundle::CONTEXT_BUNDLE_SCHEMA_VERSION,
        task: "resume the migration work".to_string(),
        project: crate::context_bundle::ProjectRef {
            key: PROJECT.to_string(),
        },
        branch: None,
        worktree: None,
        role: crate::context_bundle::AgentRole::Coder,
        as_of_epoch: 0,
        token_budget: 3_000,
        risk: crate::context_bundle::RiskClass::Low,
        include_superseded: false,
    }
}

fn suppress(conn: &Connection, kind: &str, id: Option<i64>, value: Option<&str>) {
    create_suppression(
        conn,
        &SuppressRequest {
            target: SuppressionTarget {
                kind: kind.to_string(),
                id,
                value: value.map(str::to_string),
            },
            reason: Some("hide one wrong decision"),
            actor: Some("test"),
        },
    )
    .expect("create policy suppression");
}

/// Production-shaped semantic-state slot: the verified older winner and the
/// newer rival carry different topic keys but share one semantic state key
/// whose current pointer is the newer row, as semantic writes leave it.
fn seed_semantic_slot(conn: &Connection) {
    let epoch = recent_epoch();
    for (id, topic, title, body, updated_at) in [
        (
            UNRELATED,
            "unrelated-slot",
            UNRELATED_TITLE,
            "An independent slot that must stay in Core",
            epoch - 5,
        ),
        (
            WINNER,
            "winner-topic",
            WINNER_TITLE,
            "Hidden by policy zebrafish marker",
            epoch - 10,
        ),
        (
            RIVAL,
            "rival-topic",
            RIVAL_TITLE,
            "The only visible claim left in the semantic slot",
            epoch,
        ),
    ] {
        insert_memory(
            conn,
            id,
            PROJECT,
            Some(topic),
            "decision",
            title,
            body,
            updated_at,
        );
    }
    conn.execute(
        "UPDATE memories SET source_trust_class = 'user_prompt' WHERE id = ?1",
        [WINNER],
    )
    .expect("mark verified winner");
    conn.execute(
        "INSERT INTO memory_state_keys
         (id, owner_scope, owner_key, memory_type, state_key, current_memory_id,
          created_at_epoch, updated_at_epoch)
         VALUES (?1, 'repo', ?2, 'decision', 'semantic-release-slot', ?3, ?4, ?4)",
        params![SEMANTIC_STATE_KEY_ID, PROJECT, RIVAL, epoch],
    )
    .expect("semantic state key");
    conn.execute(
        "UPDATE memories
         SET state_key_id = CASE WHEN id IN (?2, ?3) THEN ?4 ELSE state_key_id END,
             source_project = ?1, target_project = ?1,
             owner_scope = 'repo', owner_key = ?1, context_class = 'startup_core'
         WHERE id IN (?2, ?3, ?5)",
        params![PROJECT, WINNER, RIVAL, SEMANTIC_STATE_KEY_ID, UNRELATED],
    )
    .expect("move winner and rival into one semantic slot");
}

fn core_section(output: &str) -> String {
    let Some((_, rest)) = output.split_once("## Core\n") else {
        return String::new();
    };
    rest.split("\n## ").next().unwrap_or(rest).to_string()
}

fn assert_rival_owns_core(rendered: &RenderedContext) {
    let core = core_section(&rendered.output);
    assert!(
        !rendered.has_load_errors,
        "policy suppression must not become a load error:\n{}",
        rendered.output
    );
    assert!(
        !rendered.output.contains("materialize CurrentTruth winners"),
        "suppressed winner must not reach materialization:\n{}",
        rendered.output
    );
    assert!(core.contains(UNRELATED_TITLE), "core lost a slot:\n{core}");
    assert!(
        core.contains(RIVAL_TITLE),
        "visible rival must win:\n{core}"
    );
    assert!(
        !rendered.output.contains(WINNER_TITLE) && !rendered.output.contains("zebrafish"),
        "suppressed memory leaked:\n{}",
        rendered.output
    );
    assert!(rendered.stats.core_ids.contains(&RIVAL));
    assert!(!rendered.stats.core_ids.contains(&WINNER));
    let bundle = rendered.context_bundle.as_ref().expect("sealed bundle");
    let current_truth = bundle
        .current_truth
        .iter()
        .map(|item| item.stable_key.as_str())
        .collect::<Vec<_>>();
    assert!(current_truth.contains(&"memory:92"), "{current_truth:?}");
    assert!(!current_truth.contains(&"memory:91"), "{current_truth:?}");
}

#[test]
fn memory_suppressed_semantic_slot_winner_yields_core_to_visible_rival() {
    let _dir = ScopedTestDataDir::new("current-truth-suppressed-memory-winner");
    let conn = conn_with_schema();
    seed_semantic_slot(&conn);
    let baseline = render(&conn, None);
    let baseline_core = core_section(&baseline.output);
    assert!(!baseline.has_load_errors, "{}", baseline.output);
    assert!(baseline_core.contains(WINNER_TITLE), "{baseline_core}");
    assert!(!baseline_core.contains(RIVAL_TITLE), "{baseline_core}");

    suppress(&conn, "memory", Some(WINNER), None);
    assert_rival_owns_core(&render(&conn, None));

    crate::memory::suppression::revoke_suppression_arg(&conn, "memory:91", None, Some("test"))
        .expect("revoke suppression");
    let restored = core_section(&render(&conn, None).output);
    assert!(restored.contains(WINNER_TITLE), "{restored}");
    assert!(!restored.contains(RIVAL_TITLE), "{restored}");
}

#[test]
fn topic_key_suppression_hits_only_the_semantic_slot_winner() {
    let _dir = ScopedTestDataDir::new("current-truth-suppressed-topic-winner");
    let conn = conn_with_schema();
    seed_semantic_slot(&conn);
    suppress(&conn, "topic_key", None, Some("winner-topic"));
    assert_rival_owns_core(&render(&conn, None));
}

#[test]
fn pattern_suppression_of_the_slot_winner_keeps_core() {
    let _dir = ScopedTestDataDir::new("current-truth-suppressed-pattern-winner");
    let conn = conn_with_schema();
    seed_semantic_slot(&conn);
    suppress(&conn, "pattern", None, Some("ZebraFish"));
    assert_rival_owns_core(&render(&conn, None));
}

#[test]
fn entity_suppression_of_the_slot_winner_keeps_core() {
    let _dir = ScopedTestDataDir::new("current-truth-suppressed-entity-winner");
    let conn = conn_with_schema();
    seed_semantic_slot(&conn);
    conn.execute(
        "INSERT INTO entities (id, canonical_name, entity_type) VALUES (5091, 'LedgerVault', 'tool')",
        [],
    )
    .expect("entity");
    conn.execute(
        "INSERT INTO memory_entities (memory_id, entity_id) VALUES (?1, 5091)",
        [WINNER],
    )
    .expect("winner entity link");
    suppress(&conn, "entity", None, Some("ledgervault"));
    assert_rival_owns_core(&render(&conn, None));
}

#[test]
fn memory_suppressed_branch_neutral_winner_yields_to_branch_rival() {
    let _dir = ScopedTestDataDir::new("current-truth-suppressed-branch-winner");
    let conn = conn_with_schema();
    let epoch = recent_epoch();
    insert_memory(
        &conn,
        UNRELATED,
        PROJECT,
        Some("unrelated-slot"),
        "decision",
        UNRELATED_TITLE,
        "An independent slot that must stay in Core",
        epoch - 5,
    );
    insert_memory(
        &conn,
        WINNER,
        PROJECT,
        Some("shared-slot"),
        "decision",
        WINNER_TITLE,
        "Branch-neutral row hidden by policy",
        epoch - 10,
    );
    conn.execute(
        "UPDATE memories SET source_trust_class = 'user_prompt' WHERE id = ?1",
        [WINNER],
    )
    .expect("mark verified winner");
    insert_memory_with_branch(
        &conn,
        RIVAL,
        PROJECT,
        Some("shared-slot"),
        "decision",
        RIVAL_TITLE,
        "Branch row that stays visible",
        epoch,
        Some("feature/x"),
    );
    suppress(&conn, "memory", Some(WINNER), None);
    assert_rival_owns_core(&render(&conn, Some("feature/x")));
}

#[test]
fn suppressed_replacement_still_retires_the_row_it_replaced() {
    let _dir = ScopedTestDataDir::new("current-truth-suppressed-replacement");
    let conn = conn_with_schema();
    seed_semantic_slot(&conn);
    // memory_edges stores replacements as (from = old, to = new).
    conn.execute(
        "INSERT INTO memory_edges (edge_type, from_memory_id, to_memory_id, created_at_epoch)
         VALUES ('supersedes', ?1, ?2, ?3)",
        params![RIVAL, WINNER, recent_epoch()],
    )
    .expect("winner replaces rival");
    suppress(&conn, "memory", Some(WINNER), None);

    let rendered = render(&conn, None);
    let core = core_section(&rendered.output);
    assert!(!rendered.has_load_errors, "{}", rendered.output);
    assert!(core.contains(UNRELATED_TITLE), "core lost a slot:\n{core}");
    assert!(
        !core.contains(RIVAL_TITLE),
        "suppression must not revive replaced content:\n{core}"
    );
    assert!(
        !rendered.output.contains(WINNER_TITLE),
        "{}",
        rendered.output
    );
}

fn assert_no_suppressed_ref(label: &str, wire: &str) {
    assert!(
        !wire.contains("memory:91"),
        "{label}: suppressed ref leaked: {wire}"
    );
    assert!(
        !wire.contains(WINNER_TITLE) && !wire.contains("zebrafish"),
        "{label}: suppressed text leaked: {wire}"
    );
}

#[test]
fn suppressed_replacement_keeps_its_ref_out_of_the_shadow_audit() {
    let _dir = ScopedTestDataDir::new("current-truth-suppressed-replacement-audit");
    let conn = conn_with_schema();
    seed_semantic_slot(&conn);
    conn.execute(
        "INSERT INTO memory_edges (edge_type, from_memory_id, to_memory_id, created_at_epoch)
         VALUES ('supersedes', ?1, ?2, ?3)",
        params![RIVAL, WINNER, recent_epoch()],
    )
    .expect("winner replaces rival");
    suppress(&conn, "memory", Some(WINNER), None);

    // The slot abstains (its only visible row was retired), and the abstention
    // must not name the suppressed replacement in either bundle.
    let bundle = compile_session_start_bundle(
        &conn,
        &bundle_request(),
        "/tmp/remem-current-truth-suppressed-replacement-audit",
        None,
        true,
    )
    .expect("compile bundle");
    assert_ne!(bundle.degraded_mode, DegradedMode::Blocked, "{bundle:#?}");
    let wire = serde_json::to_string(&bundle).expect("serialize bundle");
    assert_no_suppressed_ref("mcp bundle", &wire);

    let rendered = render(&conn, None);
    assert!(!rendered.has_load_errors, "{}", rendered.output);
    assert_no_suppressed_ref("sessionstart output", &rendered.output);
    let sealed = rendered.context_bundle.as_ref().expect("sealed bundle");
    let sealed_wire = serde_json::to_string(sealed).expect("serialize sealed bundle");
    assert_no_suppressed_ref("sealed bundle", &sealed_wire);
}

#[test]
fn suppressed_conflict_party_yields_core_to_visible_rival() {
    let _dir = ScopedTestDataDir::new("current-truth-suppressed-conflict-party");
    let conn = conn_with_schema();
    seed_semantic_slot(&conn);
    conn.execute(
        "INSERT INTO memory_edges (edge_type, from_memory_id, to_memory_id, created_at_epoch)
         VALUES ('conflicts', ?1, ?2, ?3)",
        params![RIVAL, WINNER, recent_epoch()],
    )
    .expect("rival conflicts with winner");
    suppress(&conn, "memory", Some(WINNER), None);

    // Without the fix the slot abstained as an unresolved conflict that named
    // the suppressed memory and hid the visible rival.
    let rendered = render(&conn, None);
    assert_rival_owns_core(&rendered);
    assert_no_suppressed_ref("sessionstart output", &rendered.output);

    let bundle = compile_session_start_bundle(
        &conn,
        &bundle_request(),
        "/tmp/remem-current-truth-suppressed-conflict-party",
        None,
        true,
    )
    .expect("compile bundle");
    let wire = serde_json::to_string(&bundle).expect("serialize bundle");
    assert_no_suppressed_ref("mcp bundle", &wire);
}

#[test]
fn live_projection_treats_suppressed_claim_as_not_visible() {
    let conn = conn_with_schema();
    seed_semantic_slot(&conn);
    suppress(&conn, "memory", Some(WINNER), None);

    let projection =
        crate::context_bundle::project_for_scope(&conn, PROJECT, None, 0, &[RIVAL, UNRELATED])
            .expect("live projection");
    let slot = projection
        .truths
        .iter()
        .find(|truth| {
            truth
                .subject_key
                .ends_with(":decision:semantic-release-slot")
        })
        .expect("semantic slot truth");
    let winner = slot.claim.as_ref().expect("visible rival selected");
    assert_eq!(winner.canonical_ref, "memory:92");
    assert_eq!(
        slot.selected_reason,
        crate::truth::TruthSelectionReason::OnlySurvivingClaim
    );
    assert!(
        !slot.rejected.iter().any(|claim| claim == "memory:91"),
        "a policy-suppressed ref must not surface as a rejected candidate: {:?}",
        slot.rejected
    );
}

#[test]
fn mcp_context_bundle_selects_visible_rival_over_suppressed_winner() {
    let _dir = ScopedTestDataDir::new("current-truth-suppressed-bundle-winner");
    let conn = conn_with_schema();
    seed_semantic_slot(&conn);
    suppress(&conn, "memory", Some(WINNER), None);

    let bundle = compile_session_start_bundle(
        &conn,
        &bundle_request(),
        "/tmp/remem-current-truth-suppression-bundle",
        None,
        true,
    )
    .expect("compile bundle");

    assert_ne!(bundle.degraded_mode, DegradedMode::Blocked, "{bundle:#?}");
    let current_truth = bundle
        .current_truth
        .iter()
        .map(|item| item.stable_key.as_str())
        .collect::<Vec<_>>();
    assert!(current_truth.contains(&"memory:81"), "{current_truth:?}");
    assert!(current_truth.contains(&"memory:92"), "{current_truth:?}");
    let wire = serde_json::to_string(&bundle).expect("serialize bundle");
    assert!(!wire.contains("memory:91"), "suppressed ref leaked: {wire}");
    assert!(
        !wire.contains(WINNER_TITLE),
        "suppressed text leaked: {wire}"
    );
}

/// Suppressing the slot's current pointer (the lower-trust rival) must not take
/// the verified winner out of Core with it (#1139).
fn assert_verified_winner_survives_pointer_suppression(rendered: &RenderedContext) {
    let core = core_section(&rendered.output);
    assert!(!rendered.has_load_errors, "{}", rendered.output);
    assert!(core.contains(UNRELATED_TITLE), "core lost a slot:\n{core}");
    assert!(
        core.contains(WINNER_TITLE),
        "verified winner left Core:\n{core}"
    );
    assert!(rendered.stats.core_ids.contains(&WINNER));
    assert!(!rendered.stats.core_ids.contains(&RIVAL));
    assert!(
        !rendered.output.contains(RIVAL_TITLE)
            && !rendered.output.contains("only visible claim left"),
        "suppressed pointer leaked:\n{}",
        rendered.output
    );
    let bundle = rendered.context_bundle.as_ref().expect("sealed bundle");
    let wire = serde_json::to_string(bundle).expect("serialize sealed bundle");
    assert!(wire.contains("memory:91"), "{wire}");
    assert!(!wire.contains("memory:92"), "suppressed ref leaked: {wire}");
}

#[test]
fn suppressing_the_slot_pointer_keeps_the_verified_winner_in_core() {
    for (label, kind, id, value) in [
        ("memory", "memory", Some(RIVAL), None),
        ("topic", "topic_key", None, Some("rival-topic")),
        ("pattern", "pattern", None, Some("Lower-Trust Rival")),
    ] {
        let _dir = ScopedTestDataDir::new(&format!("current-truth-suppressed-pointer-{label}"));
        let conn = conn_with_schema();
        seed_semantic_slot(&conn);
        let baseline = render(&conn, None);
        assert!(baseline.stats.core_ids.contains(&WINNER), "{label}");
        assert!(!baseline.stats.core_ids.contains(&RIVAL), "{label}");

        suppress(&conn, kind, id, value);
        assert_verified_winner_survives_pointer_suppression(&render(&conn, None));
    }
}

#[test]
fn lower_trust_stand_in_still_hands_core_to_the_verified_winner() {
    const STAND_IN: i64 = 94;
    const STAND_IN_TITLE: &str = "Newer unverified stand-in";
    let _dir = ScopedTestDataDir::new("current-truth-suppressed-pointer-stand-in");
    let conn = conn_with_schema();
    seed_semantic_slot(&conn);
    insert_memory(
        &conn,
        STAND_IN,
        PROJECT,
        Some("stand-in-topic"),
        "decision",
        STAND_IN_TITLE,
        "A newer unverified row in the same slot",
        recent_epoch() - 3,
    );
    conn.execute(
        "UPDATE memories
         SET state_key_id = ?1, source_project = ?2, target_project = ?2,
             owner_scope = 'repo', owner_key = ?2, context_class = 'startup_core'
         WHERE id = ?3",
        params![SEMANTIC_STATE_KEY_ID, PROJECT, STAND_IN],
    )
    .expect("move stand-in into the semantic slot");

    // The stand-in seeds the slot; CurrentTruth still picks the verified row,
    // and materialization pulls it in because the displaced stand-in is loaded.
    suppress(&conn, "memory", Some(RIVAL), None);
    let rendered = render(&conn, None);
    assert_verified_winner_survives_pointer_suppression(&rendered);
    assert!(!rendered.stats.core_ids.contains(&STAND_IN));
    assert!(
        !core_section(&rendered.output).contains(STAND_IN_TITLE),
        "{}",
        rendered.output
    );
}

#[test]
fn state_slot_filter_admits_one_stand_in_while_the_pointer_is_suppressed() {
    const OLDER: i64 = 93;
    let _dir = ScopedTestDataDir::new("current-truth-suppressed-pointer-filter");
    let conn = conn_with_schema();
    seed_semantic_slot(&conn);
    insert_memory(
        &conn,
        OLDER,
        PROJECT,
        Some("older-topic"),
        "decision",
        "Older row in the same slot",
        "Kept out while a newer visible row exists",
        recent_epoch() - 20,
    );
    conn.execute(
        "UPDATE memories SET state_key_id = ?1 WHERE id = ?2",
        params![SEMANTIC_STATE_KEY_ID, OLDER],
    )
    .expect("move older row into the semantic slot");
    let admitted = |conn: &Connection| -> Vec<i64> {
        let mut conditions = Vec::new();
        crate::context::filters::push_context_state_slot_filter("memories", &mut conditions);
        conditions.push(crate::memory::suppression::memory_policy_filter_sql(
            "memories",
        ));
        let sql = format!(
            "SELECT id FROM memories WHERE {} ORDER BY id",
            conditions.join(" AND ")
        );
        let mut stmt = conn.prepare(&sql).expect("prepare slot filter probe");
        stmt.query_map([], |row| row.get(0))
            .expect("query slot filter probe")
            .collect::<Result<Vec<i64>, _>>()
            .expect("collect slot filter probe")
    };

    // A visible pointer represents its slot alone.
    assert_eq!(admitted(&conn), vec![UNRELATED, RIVAL]);
    // A suppressed pointer hands the slot to its newest visible row only.
    suppress(&conn, "memory", Some(RIVAL), None);
    assert_eq!(admitted(&conn), vec![UNRELATED, WINNER]);
    // The stand-in moves down the slot as more rows are hidden.
    suppress(&conn, "memory", Some(WINNER), None);
    assert_eq!(admitted(&conn), vec![UNRELATED, OLDER]);
}

#[test]
fn mcp_context_bundle_keeps_verified_winner_when_slot_pointer_is_suppressed() {
    let _dir = ScopedTestDataDir::new("current-truth-suppressed-pointer-bundle");
    let conn = conn_with_schema();
    seed_semantic_slot(&conn);
    suppress(&conn, "memory", Some(RIVAL), None);

    let bundle = compile_session_start_bundle(
        &conn,
        &bundle_request(),
        "/tmp/remem-current-truth-suppression-bundle",
        None,
        true,
    )
    .expect("compile bundle");

    assert_ne!(bundle.degraded_mode, DegradedMode::Blocked, "{bundle:#?}");
    let current_truth = bundle
        .current_truth
        .iter()
        .map(|item| item.stable_key.as_str())
        .collect::<Vec<_>>();
    assert!(current_truth.contains(&"memory:81"), "{current_truth:?}");
    assert!(current_truth.contains(&"memory:91"), "{current_truth:?}");
    let wire = serde_json::to_string(&bundle).expect("serialize bundle");
    assert!(!wire.contains("memory:92"), "suppressed ref leaked: {wire}");
    assert!(
        !wire.contains(RIVAL_TITLE),
        "suppressed text leaked: {wire}"
    );
}
