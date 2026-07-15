use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

use super::{
    artifact_path_for_project, load_artifact_fail_open, ArtifactLoad, CompiledRule, RuleAction,
    RuleOverrideState,
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
    let rule = project_rules
        .rules
        .iter()
        .find(|rule| rule.rule_id == rule_id)
        .with_context(|| format!("compiled rule '{rule_id}' not found for project '{project}'"))?;
    let stored = conn
        .query_row(
            "SELECT disabled, action_override
             FROM preference_rule_overrides
             WHERE project = ?1 AND rule_id = ?2",
            params![project, rule_id],
            |row| {
                let disabled: i64 = row.get(0)?;
                let action: Option<String> = row.get(1)?;
                Ok((disabled != 0, action))
            },
        )
        .optional()?;
    let mut state = match stored {
        Some((disabled, action)) => RuleOverrideState {
            disabled,
            action_override: parse_rule_action(action.as_deref(), rule_id)?,
        },
        None => rule.override_state.clone(),
    };
    match update {
        RuleOverrideUpdate::Disabled(value) => state.disabled = value,
        RuleOverrideUpdate::Action(value) => state.action_override = Some(value),
    }

    let action_override = state.action_override.map(rule_action_db_value);
    let now = chrono::Utc::now().timestamp();
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO preference_rule_overrides
         (project, rule_id, source_memory_id, disabled, action_override,
          reason, updated_by, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, 'remem rules CLI', 'user', ?6)
         ON CONFLICT(project, rule_id) DO UPDATE SET
           source_memory_id = excluded.source_memory_id,
           disabled = excluded.disabled,
           action_override = excluded.action_override,
           reason = excluded.reason,
           updated_by = excluded.updated_by,
           updated_at_epoch = excluded.updated_at_epoch",
        params![
            project,
            rule.rule_id,
            rule.source_memory_id,
            i64::from(state.disabled),
            action_override,
            now
        ],
    )
    .with_context(|| format!("persist override for compiled rule '{rule_id}'"))?;
    crate::memory::preference::compilation::enqueue_project(&tx, project)?;
    tx.commit()?;
    Ok(())
}

fn parse_rule_action(value: Option<&str>, rule_id: &str) -> Result<Option<RuleAction>> {
    match value {
        Some("warn") => Ok(Some(RuleAction::Warn)),
        Some("block") => Ok(Some(RuleAction::Block)),
        Some(other) => bail!("invalid action_override '{other}' for rule {rule_id}"),
        None => Ok(None),
    }
}

fn rule_action_db_value(action: RuleAction) -> &'static str {
    match action {
        RuleAction::Warn => "warn",
        RuleAction::Block => "block",
    }
}

#[cfg(test)]
mod tests;
