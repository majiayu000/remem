//! Worker-side preference rule compiler (SP671-T3).
//!
//! The compiler reads canonical SQLite state (active preference memories +
//! their persisted reinforcement counts), keeps only eligible machine-checkable
//! preferences, merges user overrides, drops rules whose source memory is no
//! longer authoritative (superseded / suppressed / expired / deleted),
//! resolves contradictory predicates in favour of the newest source memory,
//! and writes the derived artifact. Artifact writes happen ONLY from the
//! background worker via [`run_compile_rules_job`]; hooks never compile.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::rules::artifact::{CompiledRule, CompiledRulesArtifact, RuleAction, RulePredicate};
use crate::rules::store::{artifact_path_for_project, write_artifact_atomic};
use crate::runtime_config::{rule_compilation_config, RuleCompilationConfig};

mod classify;

pub use classify::{classify_preference_predicate, PreferenceClassification, PreferencePredicate};

/// Outcome of a worker compile pass for one project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileOutcome {
    pub project: String,
    pub rule_count: usize,
    pub artifact_path: std::path::PathBuf,
}

/// Worker-only entry point: gate on config, compile canonical state into the
/// derived artifact, and persist it atomically. Returns `Ok(None)` when rule
/// compilation is disabled by config (disabled-by-default).
///
/// U-29: a compile or write failure is recorded at error level in the
/// `preference_rule_diagnostics` table and propagated, never swallowed.
pub fn run_compile_rules_job(project: &str) -> Result<Option<CompileOutcome>> {
    let config = rule_compilation_config()?;
    if !config.enabled {
        return Ok(None);
    }
    let conn = crate::db::open_db()?;
    let data_dir = crate::db::absolute_data_dir()?;
    let artifact_path = artifact_path_for_project(&data_dir, project);

    let artifact = match compile_project_rules(&conn, project, config) {
        Ok(artifact) => artifact,
        Err(error) => {
            record_diagnostic(&conn, project, "error", &error.to_string(), None, None);
            crate::log::error(
                "rules",
                &format!("compile failed for project {project}: {error}"),
            );
            return Err(error);
        }
    };

    if let Err(error) = write_artifact_atomic(&artifact_path, &artifact) {
        let message = format!("artifact write failed: {error}");
        record_diagnostic(&conn, project, "error", &message, None, None);
        crate::log::error(
            "rules",
            &format!("compile artifact write failed for project {project}: {error}"),
        );
        return Err(error);
    }

    let rule_count = artifact.rules.len();
    record_diagnostic(
        &conn,
        project,
        "ok",
        &format!("compiled {rule_count} rule(s)"),
        Some(rule_count),
        Some(&artifact_path.to_string_lossy()),
    );
    Ok(Some(CompileOutcome {
        project: project.to_string(),
        rule_count,
        artifact_path,
    }))
}

/// Pure compile pass: build the artifact from canonical state without writing
/// anything. Used by the worker entry point and by tests; never writes files.
pub fn compile_project_rules(
    conn: &Connection,
    project: &str,
    config: RuleCompilationConfig,
) -> Result<CompiledRulesArtifact> {
    let now = chrono::Utc::now().timestamp();
    let eligible = select_eligible_preferences(conn, project, config.min_reinforcement, now)?;
    let overrides = load_overrides(conn, project)?;

    // Rows arrive newest-source-first (updated_at DESC, id DESC). Resolve
    // conflicts by keeping the first (newest) rule per conflict key.
    let mut seen_conflicts: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut rules = Vec::new();
    for pref in eligible {
        let Some(classification) = classify_preference_predicate(&pref.content) else {
            // machine_checkable was set at apply time; a classification that no
            // longer resolves is a state drift we skip rather than compile.
            continue;
        };
        let conflict_key = classification.predicate.conflict_key();
        if !seen_conflicts.insert(conflict_key.clone()) {
            record_diagnostic(
                conn,
                project,
                "warn",
                &format!(
                    "dropped preference #{} superseded by newer conflicting rule ({conflict_key})",
                    pref.memory_id
                ),
                None,
                None,
            );
            continue;
        }

        let rule_id = format!("pref-{}-1", pref.memory_id);
        let message = format!("Preference #{}: {}", pref.memory_id, classification.summary);
        let predicate = match classification.predicate {
            PreferencePredicate::CommandRegex { pattern, .. } => {
                RulePredicate::CommandRegex { pattern, message }
            }
            PreferencePredicate::CommitTrailerForbidden { trailer, .. } => {
                RulePredicate::CommitTrailerForbidden { trailer, message }
            }
        };
        let override_state =
            overrides
                .get(&rule_id)
                .cloned()
                .unwrap_or(crate::rules::RuleOverrideState {
                    disabled: false,
                    action_override: None,
                });
        rules.push(CompiledRule {
            rule_id,
            source_memory_id: pref.memory_id,
            reinforcement_count: pref.reinforcement_count,
            action: RuleAction::Warn,
            override_state,
            predicate,
        });
    }

    // Stable ordering for deterministic artifacts.
    rules.sort_by(|a, b| a.rule_id.cmp(&b.rule_id));
    Ok(CompiledRulesArtifact::new(now, rules))
}

struct EligiblePreference {
    memory_id: i64,
    content: String,
    reinforcement_count: i64,
}

fn select_eligible_preferences(
    conn: &Connection,
    project: &str,
    min_reinforcement: i64,
    now: i64,
) -> Result<Vec<EligiblePreference>> {
    let policy_filter = crate::memory::suppression::memory_policy_filter_sql("m");
    let sql = format!(
        "SELECT m.id, m.content, r.reinforcement_count
         FROM memories m
         JOIN memory_preference_reinforcements r ON r.memory_id = m.id
         WHERE m.memory_type = 'preference'
           AND m.status = 'active'
           AND (m.expires_at_epoch IS NULL OR m.expires_at_epoch > ?1)
           AND m.owner_scope IS NOT NULL
           AND r.machine_checkable = 1
           AND r.reinforcement_count >= ?2
           AND (m.project = ?3 OR COALESCE(m.scope, 'project') = 'global')
           AND {policy_filter}
         ORDER BY m.updated_at_epoch DESC, m.id DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![now, min_reinforcement, project], |row| {
        Ok(EligiblePreference {
            memory_id: row.get(0)?,
            content: row.get(1)?,
            reinforcement_count: row.get(2)?,
        })
    })?;
    crate::db::query::collect_rows(rows).context("load eligible preferences for rule compilation")
}

fn load_overrides(
    conn: &Connection,
    project: &str,
) -> Result<std::collections::HashMap<String, crate::rules::RuleOverrideState>> {
    let mut stmt = conn.prepare(
        "SELECT rule_id, disabled, action_override
         FROM preference_rule_overrides
         WHERE project = ?1",
    )?;
    let rows = stmt.query_map(params![project], |row| {
        let rule_id: String = row.get(0)?;
        let disabled: i64 = row.get(1)?;
        let action_override: Option<String> = row.get(2)?;
        Ok((rule_id, disabled != 0, action_override))
    })?;
    let mut map = std::collections::HashMap::new();
    for row in rows {
        let (rule_id, disabled, action_override) = row?;
        let action_override = match action_override.as_deref() {
            Some("warn") => Some(RuleAction::Warn),
            Some("block") => Some(RuleAction::Block),
            Some(other) => {
                anyhow::bail!("invalid action_override '{other}' for rule {rule_id}");
            }
            None => None,
        };
        map.insert(
            rule_id,
            crate::rules::RuleOverrideState {
                disabled,
                action_override,
            },
        );
    }
    Ok(map)
}

fn record_diagnostic(
    conn: &Connection,
    project: &str,
    status: &str,
    message: &str,
    rule_count: Option<usize>,
    artifact_path: Option<&str>,
) {
    let now = chrono::Utc::now().timestamp();
    let result = conn.execute(
        "INSERT INTO preference_rule_diagnostics
         (project, event_kind, status, message, rule_id, artifact_path, rule_count, occurred_at_epoch)
         VALUES (?1, 'compile', ?2, ?3, NULL, ?4, ?5, ?6)",
        params![
            project,
            status,
            message,
            artifact_path,
            rule_count.map(|count| count as i64),
            now
        ],
    );
    if let Err(error) = result {
        crate::log::error(
            "rules",
            &format!("failed to record compile diagnostic for {project}: {error}"),
        );
    }
}

#[cfg(test)]
mod tests;
