use anyhow::{bail, ensure, Context, Result};
use rusqlite::{params, Connection};
use std::path::Path;

use super::{
    artifact_path_for_project, load_artifact_fail_open, ArtifactLoad, CompiledRule, RuleAction,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRules {
    pub project: String,
    pub compiled_at_epoch: i64,
    pub rules: Vec<CompiledRule>,
}

pub fn list_project_rules(data_dir: &Path, project: &str) -> Result<ProjectRules> {
    let path = artifact_path_for_project(data_dir, project);
    let artifact = match load_artifact_fail_open(&path) {
        ArtifactLoad::Loaded(artifact) => artifact,
        ArtifactLoad::FailOpen { message, .. } => bail!(message),
    };
    Ok(ProjectRules {
        project: project.to_string(),
        compiled_at_epoch: artifact.compiled_at_epoch,
        rules: artifact.rules,
    })
}

pub fn set_rule_disabled(
    conn: &Connection,
    data_dir: &Path,
    project: &str,
    rule_id: &str,
    disabled: bool,
) -> Result<()> {
    update_rule_override(
        conn,
        data_dir,
        project,
        rule_id,
        RuleOverrideUpdate::Disabled(disabled),
    )
}

pub fn set_rule_action(
    conn: &Connection,
    data_dir: &Path,
    project: &str,
    rule_id: &str,
    action: RuleAction,
) -> Result<()> {
    if action == RuleAction::Block {
        bail!(
            "block action is unavailable until remem installs a supported pre-execution enforcement hook; use 'warn'"
        );
    }
    update_rule_override(
        conn,
        data_dir,
        project,
        rule_id,
        RuleOverrideUpdate::Action(action),
    )
}

enum RuleOverrideUpdate {
    Disabled(bool),
    Action(RuleAction),
}

fn update_rule_override(
    conn: &Connection,
    data_dir: &Path,
    project: &str,
    rule_id: &str,
    update: RuleOverrideUpdate,
) -> Result<()> {
    let project_rules = list_project_rules(data_dir, project)?;
    let artifact_rule = project_rules
        .rules
        .iter()
        .find(|rule| rule.rule_id == rule_id)
        .with_context(|| format!("compiled rule '{rule_id}' not found for project '{project}'"))?;
    let config = crate::runtime_config::rule_compilation_config()
        .context("read rule compilation config before rule override")?;
    ensure!(
        config.enabled,
        "rule compilation is disabled; enable rule_compilation.enabled before changing compiled rule overrides"
    );

    let tx = conn.unchecked_transaction()?;
    let current_rules = crate::rules::compile_project_rules(&tx, project, config)
        .context("validate current rule eligibility before override")?;
    let current_rule = current_rules
        .rules
        .iter()
        .find(|rule| rule.rule_id == rule_id)
        .with_context(|| {
            format!(
                "compiled rule '{rule_id}' is stale and no longer eligible for project '{project}'"
            )
        })?;
    ensure!(
        artifact_rule.source_memory_id == current_rule.source_memory_id
            && artifact_rule.predicate == current_rule.predicate,
        "compiled rule '{rule_id}' is stale; wait for the pending worker rebuild before changing it"
    );

    execute_override_upsert(&tx, project, current_rule, update)?;
    crate::memory::preference::compilation::enqueue_project_required(&tx, project)?;
    tx.commit()?;
    Ok(())
}

fn execute_override_upsert(
    conn: &Connection,
    project: &str,
    rule: &CompiledRule,
    update: RuleOverrideUpdate,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    match update {
        RuleOverrideUpdate::Disabled(disabled) => conn.execute(
            "INSERT INTO preference_rule_overrides
                 (project, rule_id, source_memory_id, disabled, action_override,
                  reason, updated_by, updated_at_epoch)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'remem rules CLI', 'user', ?6)
                 ON CONFLICT(project, rule_id) DO UPDATE SET
                   source_memory_id = excluded.source_memory_id,
                   disabled = excluded.disabled,
                   reason = excluded.reason,
                   updated_by = excluded.updated_by,
                   updated_at_epoch = excluded.updated_at_epoch",
            params![
                project,
                rule.rule_id,
                rule.source_memory_id,
                i64::from(disabled),
                rule.override_state
                    .action_override
                    .map(rule_action_db_value),
                now
            ],
        ),
        RuleOverrideUpdate::Action(action) => conn.execute(
            "INSERT INTO preference_rule_overrides
                 (project, rule_id, source_memory_id, disabled, action_override,
                  reason, updated_by, updated_at_epoch)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'remem rules CLI', 'user', ?6)
                 ON CONFLICT(project, rule_id) DO UPDATE SET
                   source_memory_id = excluded.source_memory_id,
                   action_override = excluded.action_override,
                   reason = excluded.reason,
                   updated_by = excluded.updated_by,
                   updated_at_epoch = excluded.updated_at_epoch",
            params![
                project,
                rule.rule_id,
                rule.source_memory_id,
                i64::from(rule.override_state.disabled),
                rule_action_db_value(action),
                now
            ],
        ),
    }
    .with_context(|| format!("persist override for compiled rule '{}'", rule.rule_id))?;
    Ok(())
}

fn rule_action_db_value(action: RuleAction) -> &'static str {
    match action {
        RuleAction::Warn => "warn",
        RuleAction::Block => "block",
    }
}

#[cfg(test)]
mod tests;
