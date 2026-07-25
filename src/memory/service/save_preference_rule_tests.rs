use anyhow::Result;
use rusqlite::Connection;

use super::{save_memory, SaveMemoryRequest};
use crate::db::{self, test_support::ScopedTestDataDir};

fn seed_compilable_preference(conn: &Connection) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_candidates
         (id, scope, memory_type, topic_key, text, evidence_event_ids,
          confidence, risk_class, review_status, created_at_epoch, updated_at_epoch,
          source_trust_class)
         VALUES (1, 'project', 'preference', 'package-manager-choice',
                 'Use bun, not npm', '[1,2,3]', 0.95, 'low', 'approved', 1, 3,
                 'user_prompt')",
        [],
    )?;
    conn.execute(
        "INSERT INTO memories
         (id, project, topic_key, title, content, memory_type, created_at_epoch,
          updated_at_epoch, status, scope, source_project, target_project,
          owner_scope, owner_key, context_class, source_candidate_id,
          source_trust_class)
         VALUES (1, 'proj', 'package-manager-choice', 'Package manager',
                 'Use bun, not npm', 'preference', 1, 3, 'active', 'project',
                 'proj', 'proj', 'repo', 'proj', 'startup_core', 1, 'user_prompt')",
        [],
    )?;
    conn.execute(
        "INSERT INTO memory_preference_reinforcements
         (memory_id, reinforcement_count, source_evidence,
          last_reinforced_at_epoch, created_at_epoch, updated_at_epoch,
          machine_checkable, risk_class)
         VALUES (1, 3, '[1,2,3]', 3, 1, 3, 1, 'low')",
        [],
    )?;
    conn.execute(
        "INSERT INTO preference_rule_overrides
         (project, rule_id, source_memory_id, disabled, action_override,
          updated_by, updated_at_epoch)
         VALUES ('proj', 'pref-1-1', 1, 1, 'block', 'user', 4)",
        [],
    )?;
    Ok(())
}

#[test]
fn opposite_direct_save_drops_candidate_rule_state_and_enqueues_compile() -> Result<()> {
    let _dir = ScopedTestDataDir::new("direct-save-opposite-preference-rule-state");
    crate::runtime_config::init_config()?;
    crate::runtime_config::set_config_value("rule_compilation.enabled", "true")?;
    let conn = db::open_db()?;
    seed_compilable_preference(&conn)?;

    let saved = save_memory(
        &conn,
        &SaveMemoryRequest {
            text: "Use npm, not bun".to_string(),
            title: Some("Package manager".to_string()),
            project: Some("proj".to_string()),
            topic_key: Some("package-manager-choice".to_string()),
            memory_type: Some("preference".to_string()),
            scope: Some("project".to_string()),
            local_copy_enabled: Some(false),
            ..SaveMemoryRequest::default()
        },
    )?;

    assert_eq!(saved.id, 1);
    assert_eq!(saved.operation, "update");
    let (content, source_candidate_id): (String, Option<i64>) = conn.query_row(
        "SELECT content, source_candidate_id FROM memories WHERE id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(content, "Use npm, not bun");
    assert_eq!(source_candidate_id, None);
    let reinforcement_rows: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_preference_reinforcements WHERE memory_id = 1",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(reinforcement_rows, 0);
    let override_rows: i64 = conn.query_row(
        "SELECT COUNT(*) FROM preference_rule_overrides WHERE source_memory_id = 1",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(override_rows, 0);
    let compile_jobs: i64 = conn.query_row(
        "SELECT COUNT(*) FROM jobs
         WHERE job_type = 'compile_rules' AND project = 'proj' AND state = 'pending'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(compile_jobs, 1);
    Ok(())
}

#[test]
fn cross_type_direct_save_does_not_overwrite_preference_rule_source() -> Result<()> {
    let _dir = ScopedTestDataDir::new("direct-save-cross-type-preference-rule-state");
    crate::runtime_config::init_config()?;
    crate::runtime_config::set_config_value("rule_compilation.enabled", "true")?;
    let conn = db::open_db()?;
    seed_compilable_preference(&conn)?;

    let saved = save_memory(
        &conn,
        &SaveMemoryRequest {
            text: "npm and bun are installed in this repository".to_string(),
            title: Some("Installed package managers".to_string()),
            project: Some("proj".to_string()),
            topic_key: Some("package-manager-choice".to_string()),
            memory_type: Some("discovery".to_string()),
            scope: Some("project".to_string()),
            local_copy_enabled: Some(false),
            ..SaveMemoryRequest::default()
        },
    )?;

    assert_ne!(
        saved.id, 1,
        "different memory types must not share an upsert row"
    );
    let original: (String, String, Option<i64>) = conn.query_row(
        "SELECT memory_type, content, source_candidate_id FROM memories WHERE id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(
        original,
        (
            "preference".to_string(),
            "Use bun, not npm".to_string(),
            Some(1)
        )
    );
    let state_rows: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_preference_reinforcements WHERE memory_id = 1",
        [],
        |row| row.get(0),
    )?;
    let override_rows: i64 = conn.query_row(
        "SELECT COUNT(*) FROM preference_rule_overrides WHERE source_memory_id = 1",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(state_rows, 1);
    assert_eq!(override_rows, 1);
    Ok(())
}
