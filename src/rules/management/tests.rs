use super::*;
use crate::db::{self, test_support::ScopedTestDataDir};
use crate::rules::{
    compile_project_rules, write_artifact_atomic, RuleOverrideState, RulePredicate,
};
use crate::runtime_config::RuleCompilationConfig;

const PROJECT: &str = "/tmp/remem";

fn insert_cli_rule_fixture(conn: &Connection) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_candidates
         (id, scope, memory_type, topic_key, text, evidence_event_ids,
          confidence, risk_class, review_status, created_at_epoch, updated_at_epoch,
          source_trust_class)
         VALUES (1, 'project', 'preference', 'package-manager',
                 'Use bun, not npm', '[1,2,3]', 0.95, 'low', 'approved', 1, 3,
                 'user_prompt')",
        [],
    )?;
    conn.execute(
        "INSERT INTO memories
         (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch,
          status, scope, owner_scope, owner_key, source_candidate_id, source_trust_class)
         VALUES (1, ?1, 'pref', 'Use bun, not npm', 'preference', 1, 3,
                 'active', 'project', 'repo', ?1, 1, 'user_prompt')",
        [PROJECT],
    )?;
    conn.execute(
        "INSERT INTO memory_preference_reinforcements
         (memory_id, reinforcement_count, source_evidence,
          last_reinforced_at_epoch, created_at_epoch, updated_at_epoch,
          machine_checkable, risk_class)
         VALUES (1, 3, '[1,2,3]', 3, 1, 3, 1, 'low')",
        [],
    )?;
    Ok(())
}

fn compile_and_write(conn: &Connection, data_dir: &Path) -> Result<ProjectRules> {
    let artifact = compile_project_rules(
        conn,
        PROJECT,
        RuleCompilationConfig {
            enabled: true,
            min_reinforcement: 3,
        },
    )?;
    write_artifact_atomic(artifact_path_for_project(data_dir, PROJECT), &artifact)?;
    list_project_rules(data_dir, PROJECT)
}

fn worker_rebuild(data_dir: &Path) -> Result<ProjectRules> {
    crate::rules::run_compile_rules_job(PROJECT)?.ok_or_else(|| {
        anyhow::anyhow!("rule compilation unexpectedly disabled during worker rebuild")
    })?;
    list_project_rules(data_dir, PROJECT)
}

#[test]
fn list_exposes_compiled_rule_provenance() -> Result<()> {
    let scoped = ScopedTestDataDir::new("rules-cli-list");
    let conn = db::open_db()?;
    insert_cli_rule_fixture(&conn)?;

    let listed = compile_and_write(&conn, &scoped.path)?;

    assert_eq!(listed.project, PROJECT);
    assert!(listed.compiled_at_epoch > 0);
    assert_eq!(listed.rules.len(), 1);
    let rule = &listed.rules[0];
    assert_eq!(rule.rule_id, "pref-1-1");
    assert_eq!(rule.source_memory_id, 1);
    assert_eq!(rule.reinforcement_count, 3);
    assert_eq!(rule.effective_action(), RuleAction::Warn);
    assert!(!rule.override_state.disabled);
    assert!(matches!(rule.predicate, RulePredicate::CommandRegex { .. }));
    Ok(())
}

#[test]
fn overrides_round_trip_through_artifact_deletion_and_recompile() -> Result<()> {
    let scoped = ScopedTestDataDir::new("rules-cli-round-trip");
    crate::runtime_config::init_config()?;
    crate::runtime_config::set_config_value("rule_compilation.enabled", "true")?;
    let conn = db::open_db()?;
    insert_cli_rule_fixture(&conn)?;
    let initial = compile_and_write(&conn, &scoped.path)?;
    assert_eq!(
        initial.rules[0].override_state,
        RuleOverrideState {
            disabled: false,
            action_override: None,
        }
    );

    set_rule_disabled(&conn, &scoped.path, PROJECT, "pref-1-1", true)?;
    set_rule_action(&conn, &scoped.path, PROJECT, "pref-1-1", RuleAction::Warn)?;
    let stored: (i64, String) = conn.query_row(
        "SELECT disabled, action_override FROM preference_rule_overrides
         WHERE project = ?1 AND rule_id = 'pref-1-1'",
        [PROJECT],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(stored, (1, "warn".to_string()));
    let pending: i64 = conn.query_row(
        "SELECT COUNT(*) FROM jobs
         WHERE project = ?1 AND job_type = 'compile_rules' AND state = 'pending'",
        [PROJECT],
        |row| row.get(0),
    )?;
    assert_eq!(pending, 1);

    std::fs::remove_file(artifact_path_for_project(&scoped.path, PROJECT))?;
    let regenerated = worker_rebuild(&scoped.path)?;
    assert!(regenerated.rules[0].override_state.disabled);
    assert_eq!(regenerated.rules[0].effective_action(), RuleAction::Warn);

    set_rule_disabled(&conn, &scoped.path, PROJECT, "pref-1-1", false)?;
    let enabled = worker_rebuild(&scoped.path)?;
    assert!(!enabled.rules[0].override_state.disabled);
    assert_eq!(enabled.rules[0].effective_action(), RuleAction::Warn);
    Ok(())
}

#[test]
fn block_action_is_rejected_before_override_or_compile_job() -> Result<()> {
    let scoped = ScopedTestDataDir::new("rules-cli-block-unsupported");
    let conn = db::open_db()?;

    let error = match set_rule_action(&conn, &scoped.path, PROJECT, "pref-1-1", RuleAction::Block) {
        Ok(()) => panic!("block must fail closed without a pre-execution enforcement hook"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("supported pre-execution enforcement hook"),
        "{error:#}"
    );

    let override_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM preference_rule_overrides",
        [],
        |row| row.get(0),
    )?;
    let job_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM jobs WHERE job_type = 'compile_rules'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!((override_count, job_count), (0, 0));
    Ok(())
}

#[test]
fn independent_stale_updates_preserve_each_owned_override_column() -> Result<()> {
    let scoped = ScopedTestDataDir::new("rules-cli-independent-updates");
    crate::runtime_config::init_config()?;
    crate::runtime_config::set_config_value("rule_compilation.enabled", "true")?;
    let conn = db::open_db()?;
    insert_cli_rule_fixture(&conn)?;
    let stale_rule = compile_and_write(&conn, &scoped.path)?.rules.remove(0);

    execute_override_upsert(
        &conn,
        PROJECT,
        &stale_rule,
        RuleOverrideUpdate::Disabled(true),
    )?;
    execute_override_upsert(
        &conn,
        PROJECT,
        &stale_rule,
        RuleOverrideUpdate::Action(RuleAction::Block),
    )?;

    let state: (i64, String) = conn.query_row(
        "SELECT disabled, action_override FROM preference_rule_overrides
         WHERE project = ?1 AND rule_id = 'pref-1-1'",
        [PROJECT],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(state, (1, "block".to_string()));
    Ok(())
}

#[test]
fn stale_artifact_cannot_recreate_override_for_changed_predicate() -> Result<()> {
    let scoped = ScopedTestDataDir::new("rules-cli-stale-artifact");
    crate::runtime_config::init_config()?;
    crate::runtime_config::set_config_value("rule_compilation.enabled", "true")?;
    let conn = db::open_db()?;
    insert_cli_rule_fixture(&conn)?;
    compile_and_write(&conn, &scoped.path)?;
    conn.execute(
        "UPDATE memories SET content = 'Use npm, not bun', updated_at_epoch = 4 WHERE id = 1",
        [],
    )?;

    let error = set_rule_disabled(&conn, &scoped.path, PROJECT, "pref-1-1", true)
        .expect_err("stale predicate must not recreate an override");
    assert!(error.to_string().contains("is stale"), "{error:#}");
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM preference_rule_overrides",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(count, 0);
    Ok(())
}

#[test]
fn disabled_compilation_rejects_inert_override() -> Result<()> {
    let scoped = ScopedTestDataDir::new("rules-cli-disabled-compilation");
    crate::runtime_config::init_config()?;
    let conn = db::open_db()?;
    insert_cli_rule_fixture(&conn)?;
    compile_and_write(&conn, &scoped.path)?;

    let error = set_rule_disabled(&conn, &scoped.path, PROJECT, "pref-1-1", true)
        .expect_err("disabled compilation must reject an inert override");
    assert!(
        error.to_string().contains("rule compilation is disabled"),
        "{error:#}"
    );
    let state: (i64, i64) = conn.query_row(
        "SELECT
           (SELECT COUNT(*) FROM preference_rule_overrides),
           (SELECT COUNT(*) FROM jobs WHERE job_type = 'compile_rules')",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(state, (0, 0));
    Ok(())
}

#[test]
fn missing_or_unknown_rule_does_not_create_override_or_job() -> Result<()> {
    let scoped = ScopedTestDataDir::new("rules-cli-missing");
    crate::runtime_config::init_config()?;
    crate::runtime_config::set_config_value("rule_compilation.enabled", "true")?;
    let conn = db::open_db()?;
    insert_cli_rule_fixture(&conn)?;

    let missing = set_rule_disabled(&conn, &scoped.path, PROJECT, "pref-1-1", true)
        .expect_err("missing artifact must be visible to the user");
    assert!(
        missing.to_string().contains("artifact missing"),
        "{missing:#}"
    );

    let artifact_path = artifact_path_for_project(&scoped.path, PROJECT);
    std::fs::create_dir_all(
        artifact_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("compiled rule artifact path has no parent"))?,
    )?;
    std::fs::write(&artifact_path, "{not-json")?;
    let corrupt = list_project_rules(&scoped.path, PROJECT)
        .expect_err("corrupt artifact must be visible to the user");
    assert!(corrupt
        .to_string()
        .contains("parse compiled rules artifact"));

    compile_and_write(&conn, &scoped.path)?;
    let unknown = set_rule_disabled(&conn, &scoped.path, PROJECT, "pref-999-1", true)
        .expect_err("unknown rule must not create an override");
    assert!(unknown.to_string().contains("not found"), "{unknown:#}");
    let override_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM preference_rule_overrides",
        [],
        |row| row.get(0),
    )?;
    let job_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM jobs WHERE job_type = 'compile_rules'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!((override_count, job_count), (0, 0));
    Ok(())
}

#[test]
fn enqueue_failure_rolls_back_override() -> Result<()> {
    let scoped = ScopedTestDataDir::new("rules-cli-enqueue-rollback");
    let conn = db::open_db()?;
    insert_cli_rule_fixture(&conn)?;
    compile_and_write(&conn, &scoped.path)?;
    std::fs::write(
        crate::runtime_config::config_path(),
        "[rule_compilation]\nenabled = 'yes'\n",
    )?;

    let error = set_rule_disabled(&conn, &scoped.path, PROJECT, "pref-1-1", true)
        .expect_err("invalid enqueue config must roll back the override");
    assert!(
        error
            .to_string()
            .contains("read rule compilation config before rule override"),
        "{error:#}"
    );
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM preference_rule_overrides",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(count, 0);
    Ok(())
}
