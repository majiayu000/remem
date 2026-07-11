use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};

use crate::db::test_support::ScopedTestDataDir;
use crate::memory::scope_cleanup::{
    apply_memory_cleanup_plan, archive_objects, build_preference_cleanup_plan, reroute_objects,
    ArchiveRequest, ObjectRef, RerouteRequest, TargetProjectUpdate,
};
use crate::runtime_config::RuleCompilationConfig;

const PROJECT: &str = "/tmp/preference-cleanup";
const PREFERENCE: &str = "Use bun, not npm";

fn setup_rule_cleanup() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    crate::runtime_config::init_config()?;
    crate::runtime_config::set_config_value("rule_compilation.enabled", "true")?;

    for (candidate_id, memory_id, updated_at, evidence, count) in
        [(11, 101, 100, "[1,2]", 2), (12, 102, 200, "[3]", 1)]
    {
        conn.execute(
            "INSERT INTO memory_candidates
             (id, scope, memory_type, topic_key, text, evidence_event_ids,
              confidence, risk_class, review_status, created_at_epoch,
              updated_at_epoch, source_trust_class)
             VALUES (?1, 'project', 'preference', 'package-manager-choice', ?2, ?3,
                     0.95, 'low', 'approved', 1, ?4, 'user_prompt')",
            params![candidate_id, PREFERENCE, evidence, updated_at],
        )?;
        conn.execute(
            "INSERT INTO memories
             (id, project, topic_key, title, content, memory_type, created_at_epoch,
              updated_at_epoch, status, scope, source_project, target_project,
              owner_scope, owner_key, context_class, source_candidate_id,
              source_trust_class)
             VALUES (?1, ?2, 'package-manager-choice', 'Package manager', ?3,
                     'preference', 1, ?4, 'active', 'project', ?2, ?2, 'repo', ?2,
                     'startup_core', ?5, 'user_prompt')",
            params![memory_id, PROJECT, PREFERENCE, updated_at, candidate_id],
        )?;
        conn.execute(
            "INSERT INTO memory_preference_reinforcements
             (memory_id, reinforcement_count, source_evidence,
              last_reinforced_at_epoch, created_at_epoch, updated_at_epoch,
              machine_checkable, risk_class)
             VALUES (?1, ?2, ?3, ?4, 1, ?4, 1, 'low')",
            params![memory_id, count, evidence, updated_at],
        )?;
    }
    conn.execute(
        "INSERT INTO preference_rule_overrides
         (project, rule_id, source_memory_id, disabled, action_override,
          updated_by, updated_at_epoch)
         VALUES (?1, 'pref-101-1', 101, 1, 'block', 'user', 150)",
        [PROJECT],
    )?;
    Ok(conn)
}

#[test]
fn cleanup_merges_same_predicate_state_override_and_compile_wakeup() -> Result<()> {
    let _dir = ScopedTestDataDir::new("cleanup-same-predicate-rule-state");
    let conn = setup_rule_cleanup()?;
    let plan = build_preference_cleanup_plan(&conn, PROJECT)?;
    assert_eq!(plan.groups.len(), 1);
    assert_eq!(plan.groups[0].current_id, 102);
    assert_eq!(plan.groups[0].stale_ids, vec![101]);

    apply_memory_cleanup_plan(&conn, &plan)?;

    let (count, evidence): (i64, Option<String>) = conn.query_row(
        "SELECT reinforcement_count, source_evidence
         FROM memory_preference_reinforcements WHERE memory_id = 102",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(count, 3);
    assert_eq!(evidence.as_deref(), Some("[1,2,3]"));
    let stale_state: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_preference_reinforcements WHERE memory_id = 101",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(stale_state, 0);
    let transferred_override: (String, Option<i64>) = conn.query_row(
        "SELECT rule_id, source_memory_id FROM preference_rule_overrides",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(transferred_override, ("pref-102-1".to_string(), Some(102)));
    let compile_jobs: i64 = conn.query_row(
        "SELECT COUNT(*) FROM jobs
         WHERE job_type = 'compile_rules' AND project = ?1 AND state = 'pending'",
        [PROJECT],
        |row| row.get(0),
    )?;
    assert_eq!(compile_jobs, 1);
    Ok(())
}

#[test]
fn cleanup_predicate_change_drops_prior_rule_provenance() -> Result<()> {
    let _dir = ScopedTestDataDir::new("cleanup-predicate-change-rule-state");
    let conn = setup_rule_cleanup()?;
    let mut plan = build_preference_cleanup_plan(&conn, PROJECT)?;
    let group = plan.groups.first_mut().context("cleanup group")?;
    group.merged_content = Some("Use npm, not bun".to_string());

    apply_memory_cleanup_plan(&conn, &plan)?;

    let source_candidate_id: Option<i64> = conn.query_row(
        "SELECT source_candidate_id FROM memories WHERE id = 102",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(source_candidate_id, None);
    let reinforcement_rows: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_preference_reinforcements
         WHERE memory_id IN (101, 102)",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(reinforcement_rows, 0);
    let override_rows: i64 = conn.query_row(
        "SELECT COUNT(*) FROM preference_rule_overrides
         WHERE source_memory_id IN (101, 102)",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(override_rows, 0);
    let compile_jobs: i64 = conn.query_row(
        "SELECT COUNT(*) FROM jobs
         WHERE job_type = 'compile_rules' AND project = ?1 AND state = 'pending'",
        [PROJECT],
        |row| row.get(0),
    )?;
    assert_eq!(compile_jobs, 1);
    Ok(())
}

#[test]
fn cleanup_edited_plan_cannot_adopt_stale_predicate_state() -> Result<()> {
    let _dir = ScopedTestDataDir::new("cleanup-edited-plan-stale-predicate");
    let conn = setup_rule_cleanup()?;
    let mut plan = build_preference_cleanup_plan(&conn, PROJECT)?;
    let replacement = "Use npm, not bun";
    conn.execute(
        "UPDATE memories SET content = ?1 WHERE id = 101",
        [replacement],
    )?;
    conn.execute(
        "UPDATE memory_candidates SET text = ?1 WHERE id = 11",
        [replacement],
    )?;
    let group = plan.groups.first_mut().context("cleanup group")?;
    group.merged_content = Some(replacement.to_string());
    group
        .row_snapshots
        .iter_mut()
        .find(|snapshot| snapshot.id == 101)
        .context("stale cleanup snapshot")?
        .content_sha256 = content_sha256(replacement);

    apply_memory_cleanup_plan(&conn, &plan)?;

    let reinforcement_rows: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_preference_reinforcements
         WHERE memory_id IN (101, 102)",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(
        reinforcement_rows, 0,
        "canonical predicate drift must not adopt stale-row confidence"
    );
    let source_candidate_id: Option<i64> = conn.query_row(
        "SELECT source_candidate_id FROM memories WHERE id = 102",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(source_candidate_id, None);
    let override_rows: i64 = conn.query_row(
        "SELECT COUNT(*) FROM preference_rule_overrides",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(override_rows, 0);
    Ok(())
}

#[test]
fn archive_preference_enqueues_rule_removal_without_dropping_reinforcement() -> Result<()> {
    let _dir = ScopedTestDataDir::new("archive-preference-rule-state");
    let conn = setup_rule_cleanup()?;

    archive_objects(
        &conn,
        &ArchiveRequest {
            refs: &[ObjectRef::memory(102)],
            reason: Some("no longer authoritative"),
            dry_run: false,
            confirm: true,
        },
    )?;

    let state: (String, i64) = conn.query_row(
        "SELECT m.status, r.reinforcement_count
         FROM memories m
         JOIN memory_preference_reinforcements r ON r.memory_id = m.id
         WHERE m.id = 102",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(state, ("archived".to_string(), 1));
    assert_pending_compile(&conn, PROJECT)?;
    Ok(())
}

#[test]
fn reroute_preference_moves_override_and_recompiles_both_authorities() -> Result<()> {
    let _dir = ScopedTestDataDir::new("reroute-preference-rule-state");
    let conn = setup_rule_cleanup()?;
    let destination = "/tmp/preference-cleanup-destination";

    reroute_objects(
        &conn,
        &RerouteRequest {
            refs: &[ObjectRef::memory(101)],
            owner_scope: "repo",
            owner_key: destination,
            target_project: TargetProjectUpdate::Set(destination.to_string()),
            topic_domain: None,
            context_class: None,
            routing_confidence: Some(1.0),
            reason: Some("move repository authority"),
            dry_run: false,
            confirm: true,
        },
    )?;

    assert_pending_compile(&conn, PROJECT)?;
    assert_pending_compile(&conn, destination)?;
    let override_project: String = conn.query_row(
        "SELECT project FROM preference_rule_overrides WHERE source_memory_id = 101",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(override_project, destination);

    let config = RuleCompilationConfig {
        enabled: true,
        min_reinforcement: 1,
    };
    let old_rules = crate::rules::compile_project_rules(&conn, PROJECT, config)?.rules;
    assert!(old_rules.iter().all(|rule| rule.source_memory_id != 101));
    let destination_rules = crate::rules::compile_project_rules(&conn, destination, config)?.rules;
    assert_eq!(destination_rules.len(), 1);
    assert_eq!(destination_rules[0].source_memory_id, 101);
    Ok(())
}

fn assert_pending_compile(conn: &Connection, project: &str) -> Result<()> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM jobs
         WHERE job_type = 'compile_rules' AND project = ?1 AND state = 'pending'",
        [project],
        |row| row.get(0),
    )?;
    assert_eq!(count, 1, "expected one pending compile for {project}");
    Ok(())
}

fn content_sha256(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}
