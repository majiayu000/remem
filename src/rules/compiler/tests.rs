use anyhow::Result;
use rusqlite::{params, Connection};

use super::{compile_project_rules, run_compile_rules_job};
use crate::db::{self, test_support::ScopedTestDataDir};
use crate::rules::store::{artifact_path_for_project, load_artifact_fail_open, ArtifactLoad};
use crate::rules::{RuleAction, RulePredicate};
use crate::runtime_config::RuleCompilationConfig;

const PROJECT: &str = "/tmp/remem";

fn config(min: i64) -> RuleCompilationConfig {
    RuleCompilationConfig {
        enabled: true,
        min_reinforcement: min,
    }
}

struct PrefSpec<'a> {
    id: i64,
    content: &'a str,
    status: &'a str,
    scope: &'a str,
    owner_scope: Option<&'a str>,
    updated_at: i64,
    expires_at: Option<i64>,
    reinforcement: i64,
    machine_checkable: i64,
}

impl Default for PrefSpec<'_> {
    fn default() -> Self {
        Self {
            id: 1,
            content: "Use bun, not npm",
            status: "active",
            scope: "project",
            owner_scope: Some("repo"),
            updated_at: 100,
            expires_at: None,
            reinforcement: 3,
            machine_checkable: 1,
        }
    }
}

fn insert_pref(conn: &Connection, spec: &PrefSpec<'_>) -> Result<()> {
    conn.execute(
        "INSERT INTO memories
         (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch,
          status, scope, owner_scope, owner_key, expires_at_epoch)
         VALUES (?1, ?2, 'pref', ?3, 'preference', 1, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            spec.id,
            PROJECT,
            spec.content,
            spec.updated_at,
            spec.status,
            spec.scope,
            spec.owner_scope,
            spec.owner_scope.map(|_| PROJECT),
            spec.expires_at,
        ],
    )?;
    conn.execute(
        "INSERT INTO memory_preference_reinforcements
         (memory_id, reinforcement_count, source_evidence,
          last_reinforced_at_epoch, created_at_epoch, updated_at_epoch, machine_checkable)
         VALUES (?1, ?2, NULL, ?3, ?3, ?3, ?4)",
        params![
            spec.id,
            spec.reinforcement,
            spec.updated_at,
            spec.machine_checkable
        ],
    )?;
    Ok(())
}

fn compile(conn: &Connection) -> Result<Vec<String>> {
    let artifact = compile_project_rules(conn, PROJECT, config(3))?;
    Ok(artifact.rules.iter().map(|r| r.rule_id.clone()).collect())
}

#[test]
fn eligible_preference_compiles_with_warn_default() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-eligible");
    let conn = db::open_db()?;
    insert_pref(&conn, &PrefSpec::default())?;

    let artifact = compile_project_rules(&conn, PROJECT, config(3))?;
    assert_eq!(artifact.rules.len(), 1);
    let rule = &artifact.rules[0];
    assert_eq!(rule.rule_id, "pref-1-1");
    assert_eq!(rule.source_memory_id, 1);
    assert_eq!(rule.reinforcement_count, 3);
    assert_eq!(rule.action, RuleAction::Warn);
    assert!(!rule.override_state.disabled);
    assert!(rule.override_state.action_override.is_none());
    assert!(matches!(rule.predicate, RulePredicate::CommandRegex { .. }));
    Ok(())
}

#[test]
fn below_threshold_preference_is_not_compiled() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-below-threshold");
    let conn = db::open_db()?;
    insert_pref(
        &conn,
        &PrefSpec {
            reinforcement: 2,
            ..Default::default()
        },
    )?;
    assert!(compile(&conn)?.is_empty());
    Ok(())
}

#[test]
fn inactive_preference_is_not_compiled() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-inactive");
    let conn = db::open_db()?;
    insert_pref(
        &conn,
        &PrefSpec {
            status: "stale",
            ..Default::default()
        },
    )?;
    assert!(compile(&conn)?.is_empty());
    Ok(())
}

#[test]
fn expired_preference_is_not_compiled() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-expired");
    let conn = db::open_db()?;
    insert_pref(
        &conn,
        &PrefSpec {
            expires_at: Some(1),
            ..Default::default()
        },
    )?;
    assert!(compile(&conn)?.is_empty());
    Ok(())
}

#[test]
fn unresolved_owner_scope_is_not_compiled() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-no-owner");
    let conn = db::open_db()?;
    insert_pref(
        &conn,
        &PrefSpec {
            owner_scope: None,
            ..Default::default()
        },
    )?;
    assert!(compile(&conn)?.is_empty());
    Ok(())
}

#[test]
fn ambiguous_preference_is_not_compiled() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-ambiguous");
    let conn = db::open_db()?;
    insert_pref(
        &conn,
        &PrefSpec {
            content: "I like clean code",
            machine_checkable: 0,
            ..Default::default()
        },
    )?;
    assert!(compile(&conn)?.is_empty());
    Ok(())
}

#[test]
fn suppressed_source_removes_rule() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-suppressed");
    let conn = db::open_db()?;
    insert_pref(&conn, &PrefSpec::default())?;
    assert_eq!(compile(&conn)?.len(), 1);

    conn.execute(
        "INSERT INTO memory_suppressions
         (target_kind, target_id, reason, actor, status, created_at_epoch, updated_at_epoch)
         VALUES ('memory', 1, 'noisy', 'user', 'active', 1, 1)",
        [],
    )?;
    assert!(compile(&conn)?.is_empty());
    Ok(())
}

#[test]
fn deleted_source_removes_rule() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-deleted");
    let conn = db::open_db()?;
    insert_pref(&conn, &PrefSpec::default())?;
    assert_eq!(compile(&conn)?.len(), 1);

    // ON DELETE CASCADE removes the reinforcement row with the memory.
    conn.execute("DELETE FROM memories WHERE id = 1", [])?;
    assert!(compile(&conn)?.is_empty());
    Ok(())
}

#[test]
fn override_merge_applies_disable_and_action() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-override");
    let conn = db::open_db()?;
    insert_pref(&conn, &PrefSpec::default())?;
    conn.execute(
        "INSERT INTO preference_rule_overrides
         (project, rule_id, disabled, action_override, updated_at_epoch)
         VALUES (?1, 'pref-1-1', 1, 'block', 1)",
        params![PROJECT],
    )?;

    let artifact = compile_project_rules(&conn, PROJECT, config(3))?;
    assert_eq!(artifact.rules.len(), 1);
    let rule = &artifact.rules[0];
    // Compiled action stays warn; user override carries disabled + block.
    assert_eq!(rule.action, RuleAction::Warn);
    assert!(rule.override_state.disabled);
    assert_eq!(rule.override_state.action_override, Some(RuleAction::Block));
    Ok(())
}

#[test]
fn conflicting_predicates_keep_newest_source() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-conflict");
    let conn = db::open_db()?;
    // Two package-manager preferences (same conflict key) reinforced enough;
    // memory 2 is newer, so it wins and memory 1 is dropped.
    insert_pref(
        &conn,
        &PrefSpec {
            id: 1,
            content: "Use bun, not npm",
            updated_at: 100,
            ..Default::default()
        },
    )?;
    insert_pref(
        &conn,
        &PrefSpec {
            id: 2,
            content: "Prefer pnpm over npm",
            updated_at: 200,
            ..Default::default()
        },
    )?;

    let ids = compile(&conn)?;
    assert_eq!(ids, vec!["pref-2-1".to_string()]);
    Ok(())
}

#[test]
fn pure_compile_does_not_write_artifact() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-no-write");
    let conn = db::open_db()?;
    insert_pref(&conn, &PrefSpec::default())?;

    compile_project_rules(&conn, PROJECT, config(3))?;

    let data_dir = db::absolute_data_dir()?;
    let path = artifact_path_for_project(&data_dir, PROJECT);
    assert!(
        !path.exists(),
        "pure compile must not write the artifact file"
    );
    Ok(())
}

#[test]
fn worker_job_writes_artifact_and_records_diagnostic() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-worker-write");
    crate::runtime_config::init_config()?;
    crate::runtime_config::set_config_value("rule_compilation.enabled", "true")?;
    let conn = db::open_db()?;
    insert_pref(&conn, &PrefSpec::default())?;
    drop(conn);

    let outcome = run_compile_rules_job(PROJECT)?.expect("compilation should run when enabled");
    assert_eq!(outcome.rule_count, 1);

    let loaded = load_artifact_fail_open(&outcome.artifact_path);
    match loaded {
        ArtifactLoad::Loaded(artifact) => assert_eq!(artifact.rules.len(), 1),
        other => panic!("expected loaded artifact, got {other:?}"),
    }

    let conn = db::open_db()?;
    let (status, rule_count): (String, i64) = conn.query_row(
        "SELECT status, rule_count FROM preference_rule_diagnostics
         WHERE project = ?1 AND event_kind = 'compile'
         ORDER BY id DESC LIMIT 1",
        params![PROJECT],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(status, "ok");
    assert_eq!(rule_count, 1);
    Ok(())
}

#[test]
fn disabled_config_skips_compilation() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-disabled");
    crate::runtime_config::init_config()?;
    // Default config is disabled.
    let conn = db::open_db()?;
    insert_pref(&conn, &PrefSpec::default())?;
    drop(conn);

    assert!(run_compile_rules_job(PROJECT)?.is_none());
    let data_dir = db::absolute_data_dir()?;
    assert!(!artifact_path_for_project(&data_dir, PROJECT).exists());
    Ok(())
}
