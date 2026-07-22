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
const FORBIDDEN_COMMAND_MESSAGE: &str = "Command violates a compiled forbidden-command preference";
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
                PreferencePredicate::GitPushForceForbidden { .. } => {
                    RulePredicate::GitPushForceForbidden {
                        message: FORBIDDEN_COMMAND_MESSAGE.to_string(),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClosedValue {
    Allowed,
    Denied,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EligibilityScope {
    Project,
    Global,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuleEligibilityDecision {
    Eligible,
    Rejected(RejectReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RejectReason {
    Type,
    Lifecycle,
    Expiry,
    Scope,
    Owner,
    Trust,
    MachineCheckable,
    Threshold,
    ReinforcementRisk,
    CandidateRisk,
    Review,
    Policy,
    Suppressed,
}

const KNOWN_TRUST: &[&str] = &[
    "local_tool_output",
    "repo_file",
    "user_prompt",
    "external_content",
    "pack",
];
const KNOWN_RISK: &[&str] = &["low", "medium", "high", "unknown"];
const KNOWN_REVIEW: &[&str] = &[
    "pending_review",
    "quarantined",
    "auto_promoted",
    "approved",
    "edited",
    "rejected",
    "discarded",
    "deferred",
];

#[derive(Clone, Copy)]
struct RuleEligibilityInput<'a> {
    memory_type: ClosedValue,
    lifecycle: ClosedValue,
    expires_at: Option<i64>,
    scope: Option<EligibilityScope>,
    owner_scope: Option<&'a str>,
    owner_key: Option<&'a str>,
    target_project: Option<&'a str>,
    legacy_project: &'a str,
    current_project: &'a str,
    trust: ClosedValue,
    machine_checkable: i64,
    reinforcement_count: i64,
    min_reinforcement: i64,
    reinforcement_risk: ClosedValue,
    candidate_risk: ClosedValue,
    review: ClosedValue,
    policy: ClosedValue,
    now: i64,
}

fn closed_value(value: &str, known: &[&str], allowed: &[&str]) -> ClosedValue {
    if allowed.contains(&value) {
        ClosedValue::Allowed
    } else if known.contains(&value) {
        ClosedValue::Denied
    } else {
        ClosedValue::Unknown
    }
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.is_empty())
}

fn eligibility_decision(input: &RuleEligibilityInput<'_>) -> RuleEligibilityDecision {
    let reject = |reason| RuleEligibilityDecision::Rejected(reason);
    if input.memory_type != ClosedValue::Allowed {
        return reject(RejectReason::Type);
    }
    if input.lifecycle != ClosedValue::Allowed {
        return reject(RejectReason::Lifecycle);
    }
    if input.expires_at.is_some_and(|expiry| expiry <= input.now) {
        return reject(RejectReason::Expiry);
    }
    let owner_matches = match input.scope {
        Some(EligibilityScope::Project) => {
            let authority = non_empty(input.target_project)
                .or_else(|| non_empty(input.owner_key))
                .unwrap_or(input.legacy_project);
            input.owner_scope == Some("repo") && authority == input.current_project
        }
        Some(EligibilityScope::Global) => {
            input.owner_scope == Some("user")
                && input.owner_key == Some("user:default")
                && non_empty(input.target_project).is_none()
        }
        None => return reject(RejectReason::Scope),
    };
    if !owner_matches {
        return reject(RejectReason::Owner);
    }
    for (value, reason) in [
        (input.trust, RejectReason::Trust),
        (input.reinforcement_risk, RejectReason::ReinforcementRisk),
        (input.candidate_risk, RejectReason::CandidateRisk),
        (input.review, RejectReason::Review),
    ] {
        if value != ClosedValue::Allowed {
            return reject(reason);
        }
    }
    if input.machine_checkable != 1 {
        return reject(RejectReason::MachineCheckable);
    }
    if input.reinforcement_count < input.min_reinforcement {
        return reject(RejectReason::Threshold);
    }
    match input.policy {
        ClosedValue::Allowed => {}
        ClosedValue::Denied => return reject(RejectReason::Suppressed),
        ClosedValue::Unknown => return reject(RejectReason::Policy),
    }
    RuleEligibilityDecision::Eligible
}

fn select_eligible_preferences(
    conn: &Connection,
    project: &str,
    min_reinforcement: i64,
    now: i64,
) -> Result<Vec<EligiblePreference>> {
    let policy_filter = crate::memory::suppression::memory_policy_filter_sql("m");
    let sql = format!(
        "SELECT m.id, m.content, m.memory_type, m.status, m.expires_at_epoch,
                m.scope, m.owner_scope, m.owner_key, m.target_project, m.project,
                m.source_trust_class, r.machine_checkable, r.reinforcement_count,
                r.risk_class, c.risk_class, c.review_status,
                CASE WHEN EXISTS (
                    SELECT 1 FROM memory_suppressions malformed
                    WHERE malformed.status NOT IN ('active', 'revoked')
                       OR (malformed.status = 'active' AND COALESCE((
                        (malformed.target_kind IN ('memory', 'user_claim', 'user_candidate')
                         AND malformed.target_id > 0 AND malformed.target_value IS NULL)
                        OR (malformed.target_kind IN ('topic_key', 'entity', 'pattern')
                            AND malformed.target_id IS NULL
                            AND length(trim(malformed.target_value)) > 0)
                        OR (malformed.target_kind = 'summary'
                            AND (malformed.target_id IS NULL OR malformed.target_id > 0)
                            AND (malformed.target_id > 0
                                 OR length(trim(malformed.target_value)) > 0))), 0) = 0))
                     THEN -1 WHEN {policy_filter} THEN 1 ELSE 0 END
         FROM memories m
         JOIN memory_preference_reinforcements r ON r.memory_id = m.id
         JOIN memory_candidates c ON c.id = m.source_candidate_id
         WHERE m.project = ?1 OR m.target_project = ?1 OR m.owner_key = ?1
            OR m.scope = 'global'
         ORDER BY CASE WHEN m.scope = 'project' THEN 0 ELSE 1 END,
                  m.updated_at_epoch DESC, m.id DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![project], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<i64>>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, Option<String>>(7)?,
            row.get::<_, Option<String>>(8)?,
            row.get::<_, String>(9)?,
            row.get::<_, String>(10)?,
            row.get::<_, i64>(11)?,
            row.get::<_, i64>(12)?,
            row.get::<_, String>(13)?,
            row.get::<_, String>(14)?,
            row.get::<_, String>(15)?,
            row.get::<_, i64>(16)?,
        ))
    })?;
    let rows = crate::db::query::collect_rows(rows)
        .context("load rule eligibility candidates for compilation")?;
    let mut eligible = Vec::new();
    for row in rows {
        let memory_type = match crate::memory::types::MemoryType::parse(&row.2) {
            Some(crate::memory::types::MemoryType::Preference) => ClosedValue::Allowed,
            Some(_) => ClosedValue::Denied,
            None => ClosedValue::Unknown,
        };
        let input = RuleEligibilityInput {
            memory_type,
            lifecycle: closed_value(&row.3, &["active", "stale", "archived"], &["active"]),
            expires_at: row.4,
            scope: row.5.as_deref().and_then(|scope| match scope {
                "project" => Some(EligibilityScope::Project),
                "global" => Some(EligibilityScope::Global),
                _ => None,
            }),
            owner_scope: row.6.as_deref(),
            owner_key: row.7.as_deref(),
            target_project: row.8.as_deref(),
            legacy_project: &row.9,
            current_project: project,
            trust: closed_value(&row.10, KNOWN_TRUST, &KNOWN_TRUST[..3]),
            machine_checkable: row.11,
            reinforcement_count: row.12,
            min_reinforcement,
            reinforcement_risk: closed_value(&row.13, KNOWN_RISK, &["low"]),
            candidate_risk: closed_value(&row.14, KNOWN_RISK, &["low"]),
            review: closed_value(
                &row.15,
                KNOWN_REVIEW,
                &["approved", "edited", "auto_promoted"],
            ),
            policy: match row.16 {
                1 => ClosedValue::Allowed,
                0 => ClosedValue::Denied,
                _ => ClosedValue::Unknown,
            },
            now,
        };
        match eligibility_decision(&input) {
            RuleEligibilityDecision::Eligible => eligible.push(EligiblePreference {
                memory_id: row.0,
                content: row.1,
                reinforcement_count: row.12,
            }),
            RuleEligibilityDecision::Rejected(reason)
                if matches!(memory_type, ClosedValue::Unknown)
                    || [
                        input.lifecycle,
                        input.trust,
                        input.reinforcement_risk,
                        input.candidate_risk,
                        input.review,
                    ]
                    .contains(&ClosedValue::Unknown)
                    || matches!(
                        reason,
                        RejectReason::Scope | RejectReason::Owner | RejectReason::Policy
                    ) =>
            {
                crate::log::error(
                    "rules",
                    &format!("rule eligibility rejected memory {}: {reason:?}", row.0),
                )
            }
            RuleEligibilityDecision::Rejected(_) => {}
        }
    }
    Ok(eligible)
}

fn load_rule_compilation_projects(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT project
         FROM (
             SELECT DISTINCT project FROM memories
             UNION
             SELECT DISTINCT CASE
                        WHEN COALESCE(m.scope, 'project') = 'global'
                         AND m.owner_scope = 'user'
                         AND m.owner_key = 'user:default'
                         AND COALESCE(NULLIF(m.target_project, ''), '') = ''
                        THEN m.project
                        ELSE COALESCE(
                            NULLIF(m.target_project, ''),
                            CASE WHEN m.owner_scope = 'repo' THEN NULLIF(m.owner_key, '') END,
                            m.project
                        )
                    END AS project
             FROM memories m
             JOIN memory_preference_reinforcements r ON r.memory_id = m.id
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
mod eligibility_tests;
#[cfg(test)]
mod fixture_tests;
#[cfg(test)]
mod sweep_tests;
#[cfg(test)]
mod tests;
