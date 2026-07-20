use anyhow::Result;
use rusqlite::{params, Connection};

use super::{run_compile_rules_job, run_compile_rules_sweep};
use crate::db::{self, test_support::ScopedTestDataDir};
use crate::rules::{artifact_path_for_project, load_artifact_fail_open, ArtifactLoad};

fn insert_sweep_preference(
    conn: &Connection,
    id: i64,
    project: &str,
    scope: &str,
    content: &str,
    machine_checkable: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_candidates
         (id, scope, memory_type, topic_key, text, evidence_event_ids,
          confidence, risk_class, review_status, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, 'preference', 'package-manager', ?3, '[1]',
                 0.95, 'low', 'approved', 1, ?1)",
        params![id, scope, content],
    )?;
    conn.execute(
        "INSERT INTO memories
         (id, project, title, content, memory_type, created_at_epoch,
          updated_at_epoch, status, scope, owner_scope, owner_key, source_candidate_id)
         VALUES (?1, ?2, 'pref', ?3, 'preference', 1, ?1,
                 'active', ?4, 'repo', ?2, ?1)",
        params![id, project, content, scope],
    )?;
    conn.execute(
        "INSERT INTO memory_preference_reinforcements
         (memory_id, reinforcement_count, last_reinforced_at_epoch,
          created_at_epoch, updated_at_epoch, machine_checkable, risk_class)
         VALUES (?1, 3, 1, 1, 1, ?2, 'low')",
        params![id, machine_checkable],
    )?;
    Ok(())
}

#[test]
fn sweep_isolates_project_failures_and_deduplicates_errors() -> Result<()> {
    let _data_dir = ScopedTestDataDir::new("rule-sweep-isolates-project-failures");
    crate::runtime_config::init_config()?;
    crate::runtime_config::set_config_value("rule_compilation.enabled", "true")?;
    let conn = db::open_db()?;
    insert_sweep_preference(&conn, 1, "/tmp/a-bad", "project", "I like clean code", 1)?;
    insert_sweep_preference(&conn, 2, "/tmp/z-good", "project", "Use bun, not npm", 1)?;
    drop(conn);

    let first = run_compile_rules_sweep()?;
    assert_eq!(first.projects_seen, 2);
    assert_eq!(first.artifacts_changed, 1);
    assert_eq!(first.failures, 1);

    let good_path = artifact_path_for_project(&db::absolute_data_dir()?, "/tmp/z-good");
    let ArtifactLoad::Loaded(good) = load_artifact_fail_open(good_path) else {
        anyhow::bail!("valid project artifact should survive another project's failure");
    };
    assert_eq!(good.rules.len(), 1);

    let second = run_compile_rules_sweep()?;
    assert_eq!(second.artifacts_changed, 0);
    assert_eq!(second.failures, 1);
    let conn = db::open_db()?;
    let errors: i64 = conn.query_row(
        "SELECT COUNT(*) FROM preference_rule_diagnostics
         WHERE project = '/tmp/a-bad' AND status = 'error'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(errors, 1);
    conn.execute(
        "UPDATE memory_preference_reinforcements
         SET machine_checkable = 0
         WHERE memory_id = 1",
        [],
    )?;
    drop(conn);

    let recovery = run_compile_rules_sweep()?;
    assert_eq!(recovery.artifacts_changed, 1);
    assert_eq!(recovery.failures, 0);

    let conn = db::open_db()?;
    conn.execute(
        "UPDATE memory_preference_reinforcements
         SET machine_checkable = 1
         WHERE memory_id = 1",
        [],
    )?;
    drop(conn);
    let recurrence = run_compile_rules_sweep()?;
    assert_eq!(recurrence.artifacts_changed, 0);
    assert_eq!(recurrence.failures, 1);

    let conn = db::open_db()?;
    let mut stmt = conn.prepare(
        "SELECT status FROM preference_rule_diagnostics
         WHERE project = '/tmp/a-bad'
         ORDER BY id",
    )?;
    let statuses = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(statuses, ["error", "ok", "error"]);
    Ok(())
}

#[test]
fn sweep_builds_global_rules_for_canonical_projects_without_local_memories() -> Result<()> {
    let _data_dir = ScopedTestDataDir::new("rule-sweep-canonical-projects");
    crate::runtime_config::init_config()?;
    crate::runtime_config::set_config_value("rule_compilation.enabled", "true")?;
    let conn = db::open_db()?;
    insert_sweep_preference(&conn, 1, "/tmp/source", "global", "Use bun, not npm", 1)?;
    // Production creates global preferences as owner_scope='user' / owner_key='user:default'
    // (src/user_context/claims.rs DEFAULT_OWNER_KEY); the shared fixture helper hardcodes the
    // project-scoped owner shape, so normalize this global to its real production shape. The
    // tightened eligibility gate (GH671/#813) correctly rejects the malformed 'repo' global.
    conn.execute(
        "UPDATE memories SET owner_scope = 'user', owner_key = 'user:default' WHERE id = 1",
        [],
    )?;
    conn.execute(
        "INSERT INTO workspaces
         (id, root_path, created_at_epoch, updated_at_epoch)
         VALUES (1, '/tmp', 1, 1)",
        [],
    )?;
    conn.execute(
        "INSERT INTO projects
         (workspace_id, project_path, project_key, created_at_epoch, updated_at_epoch)
         VALUES (1, '/tmp/new-project', 'new-project', 1, 1)",
        [],
    )?;
    drop(conn);

    let outcome = run_compile_rules_sweep()?;
    assert_eq!(outcome.failures, 0);
    let path = artifact_path_for_project(&db::absolute_data_dir()?, "/tmp/new-project");
    let ArtifactLoad::Loaded(artifact) = load_artifact_fail_open(path) else {
        anyhow::bail!("canonical project should receive the global rule artifact");
    };
    assert_eq!(artifact.rules.len(), 1);
    Ok(())
}

#[test]
fn sweep_discovers_rerouted_preference_authority_without_prior_rule_state() -> Result<()> {
    let _data_dir = ScopedTestDataDir::new("rule-sweep-rerouted-authority");
    crate::runtime_config::init_config()?;
    crate::runtime_config::set_config_value("rule_compilation.enabled", "true")?;
    let source = "/tmp/source-authority";
    let destination = "/tmp/destination-authority";
    let conn = db::open_db()?;
    insert_sweep_preference(&conn, 1, source, "project", "Use bun, not npm", 1)?;
    conn.execute(
        "UPDATE memories
         SET target_project = ?1, owner_scope = 'repo', owner_key = ?1
         WHERE id = 1",
        [destination],
    )?;

    let routing: (String, Option<String>, String, String) = conn.query_row(
        "SELECT project, target_project, owner_scope, owner_key
         FROM memories WHERE id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(
        routing,
        (
            source.to_string(),
            Some(destination.to_string()),
            "repo".to_string(),
            destination.to_string()
        )
    );
    let prior_destination_state: i64 = conn.query_row(
        "SELECT
             (SELECT COUNT(*) FROM jobs
              WHERE job_type = 'compile_rules' AND project = ?1)
           + (SELECT COUNT(*) FROM preference_rule_overrides WHERE project = ?1)
           + (SELECT COUNT(*) FROM preference_rule_diagnostics WHERE project = ?1)
           + (SELECT COUNT(*) FROM projects WHERE project_path = ?1)",
        [destination],
        |row| row.get(0),
    )?;
    assert_eq!(prior_destination_state, 0);
    drop(conn);

    let outcome = run_compile_rules_sweep()?;
    assert_eq!(outcome.projects_seen, 2);
    assert_eq!(outcome.failures, 0);

    let source_path = artifact_path_for_project(&db::absolute_data_dir()?, source);
    let ArtifactLoad::Loaded(source_artifact) = load_artifact_fail_open(source_path) else {
        anyhow::bail!("source project should retain sweep convergence");
    };
    assert!(source_artifact.rules.is_empty());

    let destination_path = artifact_path_for_project(&db::absolute_data_dir()?, destination);
    let ArtifactLoad::Loaded(destination_artifact) = load_artifact_fail_open(destination_path)
    else {
        anyhow::bail!("rerouted authority should receive its compiled rule artifact");
    };
    assert_eq!(destination_artifact.rules.len(), 1);
    assert_eq!(destination_artifact.rules[0].source_memory_id, 1);
    Ok(())
}

#[test]
fn unchanged_artifact_recovery_records_status_before_error_recurs() -> Result<()> {
    let _data_dir = ScopedTestDataDir::new("rule-sweep-unchanged-recovery");
    crate::runtime_config::init_config()?;
    crate::runtime_config::set_config_value("rule_compilation.enabled", "true")?;
    let project = "/tmp/recovery";
    let conn = db::open_db()?;
    insert_sweep_preference(&conn, 1, project, "project", "Use bun, not npm", 1)?;
    drop(conn);

    let initial = run_compile_rules_job(project)?.expect("initial compile should run");
    assert!(initial.artifact_changed);

    let conn = db::open_db()?;
    insert_sweep_preference(&conn, 2, project, "project", "I like clean code", 1)?;
    drop(conn);
    assert!(run_compile_rules_job(project).is_err());

    let conn = db::open_db()?;
    conn.execute(
        "UPDATE memory_preference_reinforcements
         SET machine_checkable = 0
         WHERE memory_id = 2",
        [],
    )?;
    drop(conn);
    let recovery = run_compile_rules_job(project)?.expect("recovery compile should run");
    assert!(!recovery.artifact_changed);

    let conn = db::open_db()?;
    conn.execute(
        "UPDATE memory_preference_reinforcements
         SET machine_checkable = 1
         WHERE memory_id = 2",
        [],
    )?;
    drop(conn);
    assert!(run_compile_rules_job(project).is_err());

    let conn = db::open_db()?;
    let mut stmt = conn.prepare(
        "SELECT status FROM preference_rule_diagnostics
         WHERE project = ?1
         ORDER BY id",
    )?;
    let statuses = stmt
        .query_map([project], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(statuses, ["ok", "error", "ok", "error"]);
    Ok(())
}

#[test]
fn persistent_conflict_warning_is_bounded_and_records_recovery() -> Result<()> {
    let _data_dir = ScopedTestDataDir::new("rule-sweep-conflict-warning");
    crate::runtime_config::init_config()?;
    crate::runtime_config::set_config_value("rule_compilation.enabled", "true")?;
    let project = "/tmp/conflict";
    let conn = db::open_db()?;
    insert_sweep_preference(&conn, 1, project, "project", "Use bun, not npm", 1)?;
    insert_sweep_preference(&conn, 2, project, "project", "Use npm, not yarn", 1)?;
    drop(conn);

    let initial = run_compile_rules_job(project)?.expect("initial compile should run");
    assert!(initial.artifact_changed);
    for _ in 0..3 {
        let unchanged = run_compile_rules_job(project)?.expect("repeat compile should run");
        assert!(!unchanged.artifact_changed);
    }

    let conn = db::open_db()?;
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM preference_rule_diagnostics WHERE project = ?1",
        [project],
        |row| row.get(0),
    )?;
    assert_eq!(count, 1);
    conn.execute(
        "UPDATE memory_preference_reinforcements
         SET machine_checkable = 0
         WHERE memory_id = 1",
        [],
    )?;
    drop(conn);
    let recovery = run_compile_rules_job(project)?.expect("recovery compile should run");
    assert!(!recovery.artifact_changed);

    let conn = db::open_db()?;
    conn.execute(
        "UPDATE memory_preference_reinforcements
         SET machine_checkable = 1
         WHERE memory_id = 1",
        [],
    )?;
    drop(conn);
    let recurrence = run_compile_rules_job(project)?.expect("recurrence compile should run");
    assert!(!recurrence.artifact_changed);

    let conn = db::open_db()?;
    let mut stmt = conn.prepare(
        "SELECT status FROM preference_rule_diagnostics
         WHERE project = ?1
         ORDER BY id",
    )?;
    let statuses = stmt
        .query_map([project], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(statuses, ["warn", "ok", "warn"]);
    Ok(())
}
