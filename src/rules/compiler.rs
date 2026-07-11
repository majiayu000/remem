//! Worker-side preference rule compiler (SP671-T3).
//!
//! Canonical SQLite state is compiled only by the background worker. Hooks
//! read the derived artifact and never perform DB, network, or LLM work.

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::io::ErrorKind;

use crate::rules::artifact::{CompiledRule, CompiledRulesArtifact, RuleAction, RulePredicate};
use crate::rules::store::{
    artifact_path_for_project, load_artifact_fail_open, write_artifact_atomic, ArtifactLoad,
};
use crate::runtime_config::{rule_compilation_config, RuleCompilationConfig};

mod classify;

pub use classify::{
    classify_preference_predicate, classify_preference_predicates, PreferenceClassification,
    PreferencePredicate,
};

const PACKAGE_MANAGER_MESSAGE: &str = "Command violates a compiled package-manager preference";
const COMMIT_TRAILER_MESSAGE: &str = "Commit message violates a compiled trailer preference";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileOutcome {
    pub project: String,
    pub rule_count: usize,
    pub artifact_path: std::path::PathBuf,
    pub artifact_changed: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompileSweepOutcome {
    pub projects_seen: usize,
    pub artifacts_changed: usize,
    pub failures: usize,
}

/// Rebuild artifacts for every known project on a bounded cadence. One bad
/// project is isolated so it cannot stop unrelated memory work.
pub fn run_compile_rules_sweep() -> Result<CompileSweepOutcome> {
    let config = rule_compilation_config()?;
    if !config.enabled {
        return Ok(CompileSweepOutcome::default());
    }

    let conn = crate::db::open_db()?;
    let projects = load_rule_compilation_projects(&conn)?;
    drop(conn);

    let mut outcome = CompileSweepOutcome {
        projects_seen: projects.len(),
        ..Default::default()
    };
    for project in &projects {
        match run_compile_rules_job(project) {
            Ok(Some(project_outcome)) => {
                outcome.artifacts_changed += usize::from(project_outcome.artifact_changed);
            }
            Ok(None) => {}
            Err(error) => {
                outcome.failures += 1;
                crate::log::error(
                    "rules",
                    &format!("rule compilation sweep failed for project {project}: {error}"),
                );
            }
        }
    }
    Ok(outcome)
}

/// Worker-only entry point. Failures are durably recorded and propagated;
/// unchanged successful artifacts and diagnostics are not rewritten.
pub fn run_compile_rules_job(project: &str) -> Result<Option<CompileOutcome>> {
    let config = rule_compilation_config()?;
    if !config.enabled {
        return Ok(None);
    }
    let conn = crate::db::open_db()?;
    let data_dir = crate::db::absolute_data_dir()?;
    let artifact_path = artifact_path_for_project(&data_dir, project);

    let mut conflict_messages = Vec::new();
    let artifact = match compile_project_rules_with_conflicts(
        &conn,
        project,
        config,
        &mut conflict_messages,
    ) {
        Ok(artifact) => artifact,
        Err(error) => {
            if let Err(diagnostic_error) =
                record_diagnostic(&conn, project, "error", &error.to_string(), None, None)
            {
                crate::log::error(
                    "rules",
                    &format!(
                        "compile and diagnostic persistence failed for project {project}: {diagnostic_error}"
                    ),
                );
                return Err(error.context(format!(
                    "failed to persist compile diagnostic: {diagnostic_error}"
                )));
            }
            crate::log::error(
                "rules",
                &format!("compile failed for project {project}: {error}"),
            );
            return Err(error);
        }
    };

    let rule_count = artifact.rules.len();
    if matches!(
        load_artifact_fail_open(&artifact_path),
        ArtifactLoad::Loaded(existing) if existing.rules == artifact.rules
    ) {
        record_compile_success(
            &conn,
            project,
            rule_count,
            &artifact_path,
            false,
            &conflict_messages,
        )?;
        return Ok(Some(CompileOutcome {
            project: project.to_string(),
            rule_count,
            artifact_path,
            artifact_changed: false,
        }));
    }

    let previous_artifact = match snapshot_artifact(&artifact_path) {
        Ok(previous) => previous,
        Err(error) => {
            let message = format!("artifact snapshot failed: {error}");
            if let Err(diagnostic_error) =
                record_diagnostic(&conn, project, "error", &message, None, None)
            {
                return Err(error.context(format!(
                    "failed to persist artifact snapshot diagnostic: {diagnostic_error}"
                )));
            }
            return Err(error);
        }
    };

    if let Err(error) = write_artifact_atomic(&artifact_path, &artifact) {
        let message = format!("artifact write failed: {error}");
        crate::log::error(
            "rules",
            &format!("compile artifact write failed for project {project}: {error}"),
        );
        if let Err(diagnostic_error) =
            record_diagnostic(&conn, project, "error", &message, None, None)
        {
            return Err(error.context(format!(
                "failed to persist artifact write diagnostic: {diagnostic_error}"
            )));
        }
        return Err(error);
    }

    if let Err(diagnostic_error) = record_compile_success(
        &conn,
        project,
        rule_count,
        &artifact_path,
        true,
        &conflict_messages,
    ) {
        crate::log::error(
            "rules",
            &format!(
                "compile success diagnostic failed for project {project}; restoring previous artifact: {diagnostic_error}"
            ),
        );
        return match restore_artifact(&artifact_path, previous_artifact) {
            Ok(()) => Err(diagnostic_error
                .context("compile success diagnostic failed; previous artifact restored")),
            Err(restore_error) => Err(diagnostic_error.context(format!(
                "compile success diagnostic failed and previous artifact restoration failed: {restore_error}"
            ))),
        };
    }
    Ok(Some(CompileOutcome {
        project: project.to_string(),
        rule_count,
        artifact_path,
        artifact_changed: true,
    }))
}

fn snapshot_artifact(path: &std::path::Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error)
            .with_context(|| format!("read previous compiled rules artifact {}", path.display())),
    }
}

fn restore_artifact(path: &std::path::Path, previous: Option<Vec<u8>>) -> Result<()> {
    match previous {
        Some(contents) => crate::atomic_file::write_atomic(path, contents)
            .with_context(|| format!("restore compiled rules artifact {}", path.display())),
        None => match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error)
                .with_context(|| format!("remove unpublished artifact {}", path.display())),
        },
    }
}

/// Pure compile pass used by tests and the worker wrapper.
pub fn compile_project_rules(
    conn: &Connection,
    project: &str,
    config: RuleCompilationConfig,
) -> Result<CompiledRulesArtifact> {
    compile_project_rules_with_conflicts(conn, project, config, &mut Vec::new())
}

fn compile_project_rules_with_conflicts(
    conn: &Connection,
    project: &str,
    config: RuleCompilationConfig,
    conflict_messages: &mut Vec<String>,
) -> Result<CompiledRulesArtifact> {
    let now = chrono::Utc::now().timestamp();
    let eligible = select_eligible_preferences(conn, project, config.min_reinforcement, now)?;
    let overrides = load_overrides(conn, project)?;

    // Rows are project-before-global and newest-first. One source may emit
    // several distinct trailer rules, but a later source cannot replace a
    // conflict family already claimed by the authoritative earlier source.
    let mut conflict_sources = std::collections::HashMap::<String, i64>::new();
    let mut rules = Vec::new();
    for pref in eligible {
        let classifications = classify_preference_predicates(&pref.content);
        if classifications.is_empty() {
            bail!(
                "preference memory {} is marked machine_checkable but no safe v1 predicate can be derived",
                pref.memory_id
            );
        }

        for (index, classification) in classifications.into_iter().enumerate() {
            let conflict_key = classification.predicate.conflict_key();
            match conflict_sources.get(&conflict_key) {
                Some(source_memory_id) if *source_memory_id != pref.memory_id => {
                    conflict_messages.push(format!(
                        "dropped preference #{} behind authoritative conflicting rule ({conflict_key})",
                        pref.memory_id
                    ));
                    continue;
                }
                Some(_) => {}
                None => {
                    conflict_sources.insert(conflict_key, pref.memory_id);
                }
            }

            let rule_id = format!("pref-{}-{}", pref.memory_id, index + 1);
            let predicate = match classification.predicate {
                PreferencePredicate::CommandRegex { pattern, .. } => RulePredicate::CommandRegex {
                    pattern,
                    message: PACKAGE_MANAGER_MESSAGE.to_string(),
                },
                PreferencePredicate::CommitTrailerForbidden { trailer, .. } => {
                    RulePredicate::CommitTrailerForbidden {
                        trailer,
                        message: COMMIT_TRAILER_MESSAGE.to_string(),
                    }
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
    }

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
         JOIN memory_candidates c ON c.id = m.source_candidate_id
         WHERE m.memory_type = 'preference'
           AND m.status = 'active'
           AND (m.expires_at_epoch IS NULL OR m.expires_at_epoch > ?1)
           AND (
               (COALESCE(m.scope, 'project') = 'project'
                AND m.owner_scope = 'repo'
                AND COALESCE(
                    NULLIF(m.target_project, ''),
                    NULLIF(m.owner_key, ''),
                    m.project
                ) = ?3)
               OR
               (COALESCE(m.scope, 'project') = 'global'
                AND m.owner_scope IS NOT NULL)
           )
           AND m.source_trust_class IN ('local_tool_output', 'repo_file', 'user_prompt')
           AND r.machine_checkable = 1
           AND r.risk_class = 'low'
           AND r.reinforcement_count >= ?2
           AND c.risk_class = 'low'
           AND c.review_status IN ('approved', 'edited', 'auto_promoted')
           AND {policy_filter}
         ORDER BY CASE
                    WHEN COALESCE(m.scope, 'project') = 'project' THEN 0
                    ELSE 1
                  END,
                  m.updated_at_epoch DESC,
                  m.id DESC"
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

fn load_rule_compilation_projects(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT project
         FROM (
             SELECT DISTINCT project FROM memories
             UNION
             SELECT DISTINCT project FROM preference_rule_overrides
             UNION
             SELECT DISTINCT project FROM preference_rule_diagnostics
             UNION
             SELECT DISTINCT project FROM jobs WHERE job_type = 'compile_rules'
             UNION
             SELECT DISTINCT project_path AS project FROM projects
         )
         WHERE project IS NOT NULL AND TRIM(project) <> ''
         ORDER BY project",
    )?;
    let rows = stmt.query_map([], |row| row.get(0))?;
    crate::db::query::collect_rows(rows).context("load projects for rule compilation sweep")
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
            Some(other) => bail!("invalid action_override '{other}' for rule {rule_id}"),
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
) -> Result<()> {
    if status != "ok"
        && latest_compile_diagnostic(conn, project)?.is_some_and(
            |(latest_status, latest_message)| latest_status == status && latest_message == message,
        )
    {
        return Ok(());
    }
    let now = chrono::Utc::now().timestamp();
    conn.execute(
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
    )
    .with_context(|| format!("persist compile diagnostic for {project}"))?;
    Ok(())
}

fn record_compile_success(
    conn: &Connection,
    project: &str,
    rule_count: usize,
    artifact_path: &std::path::Path,
    artifact_changed: bool,
    conflict_messages: &[String],
) -> Result<()> {
    if !conflict_messages.is_empty() {
        let mut conflicts = conflict_messages.to_vec();
        conflicts.sort();
        conflicts.dedup();
        return record_diagnostic(
            conn,
            project,
            "warn",
            &format!(
                "compiled {rule_count} rule(s) with conflicts: {}",
                conflicts.join("; ")
            ),
            Some(rule_count),
            Some(&artifact_path.display().to_string()),
        );
    }

    let latest = latest_compile_diagnostic(conn, project)
        .with_context(|| format!("load latest compile diagnostic for {project}"))?;
    if artifact_changed
        || latest
            .as_ref()
            .is_none_or(|(latest_status, _)| latest_status != "ok")
    {
        record_diagnostic(
            conn,
            project,
            "ok",
            &format!("compiled {rule_count} rule(s)"),
            Some(rule_count),
            Some(&artifact_path.display().to_string()),
        )?;
    }
    Ok(())
}

fn latest_compile_diagnostic(
    conn: &Connection,
    project: &str,
) -> rusqlite::Result<Option<(String, String)>> {
    conn.query_row(
        "SELECT status, COALESCE(message, '')
         FROM preference_rule_diagnostics
         WHERE project = ?1
           AND event_kind = 'compile'
         ORDER BY id DESC
         LIMIT 1",
        params![project],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
}

#[cfg(test)]
mod sweep_tests;
#[cfg(test)]
mod tests;
