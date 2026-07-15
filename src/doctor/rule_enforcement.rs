use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use super::types::{Check, Status};

pub(super) fn check_compiled_rules(conn: Option<&Connection>) -> Check {
    let config = match crate::runtime_config::rule_compilation_config() {
        Ok(config) => config,
        Err(error) => {
            return Check::new(
                "Compiled rules",
                Status::Fail,
                format!("rule compilation config is invalid: {error}"),
            )
        }
    };
    let data_dir = match crate::db::absolute_data_dir() {
        Ok(data_dir) => data_dir,
        Err(error) => {
            return Check::new(
                "Compiled rules",
                Status::Fail,
                format!("rule data directory is unavailable: {error}"),
            )
        }
    };
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(error) => {
            return Check::new(
                "Compiled rules",
                Status::Fail,
                format!("current project directory is unavailable: {error}"),
            )
        }
    };
    let project = crate::db::project_from_cwd(&cwd.to_string_lossy());
    check_compiled_rules_for(conn, &data_dir, &project, config.enabled)
}

pub(super) fn check_rule_enforcement_capabilities() -> Vec<Check> {
    vec![
        Check::new(
            "Rule enforcement (claude-code)",
            Status::Ok,
            "pre_execution=true warn=supported block=supported",
        ),
        Check::new(
            "Rule enforcement (codex-cli)",
            Status::Ok,
            "pre_execution=false warn=unsupported block=unsupported reason=no_pre_execution_bash_hook",
        ),
    ]
}

fn check_compiled_rules_for(
    conn: Option<&Connection>,
    data_dir: &Path,
    project: &str,
    enabled: bool,
) -> Check {
    let artifact_path = crate::rules::artifact_path_for_project(data_dir, project);
    let (artifact_present, artifact_valid, rule_count, last_compile_epoch, artifact_error) =
        match crate::rules::load_artifact_fail_open(&artifact_path) {
            crate::rules::ArtifactLoad::Loaded(artifact) => (
                true,
                true,
                Some(artifact.rules.len()),
                Some(artifact.compiled_at_epoch),
                None,
            ),
            crate::rules::ArtifactLoad::FailOpen { kind, .. } => (
                kind != crate::rules::ArtifactLoadErrorKind::Missing,
                false,
                None,
                None,
                Some(artifact_error_label(kind)),
            ),
        };

    let compile = conn.map(|conn| load_compile_diagnostics_for(conn, project));
    let evaluation = crate::rules::load_evaluation_error(data_dir, project);
    let mut degraded = enabled && (!artifact_present || !artifact_valid);
    let compile_detail = match compile {
        Some(Ok(snapshot)) => {
            degraded |= enabled
                && snapshot
                    .latest_status
                    .as_deref()
                    .is_some_and(|status| status != "ok");
            format!(
                "latest_compile_status={} latest_compile_event_epoch={} last_compile_error={}",
                optional_text(snapshot.latest_status.as_deref()),
                optional_epoch(snapshot.latest_epoch),
                optional_error(snapshot.last_error_epoch),
            )
        }
        Some(Err(_)) => {
            degraded |= enabled;
            "latest_compile_status=unavailable latest_compile_event_epoch=unavailable last_compile_error=unavailable".to_string()
        }
        None => {
            degraded |= enabled;
            "latest_compile_status=unavailable latest_compile_event_epoch=unavailable last_compile_error=unavailable".to_string()
        }
    };
    let evaluation_detail = match evaluation {
        Ok(Some(record)) => {
            degraded |= enabled;
            let codes = record
                .codes
                .iter()
                .map(|code| code.as_str())
                .collect::<Vec<_>>()
                .join(",");
            format!("last_evaluation_error={codes}@{}", record.occurred_at_epoch)
        }
        Ok(None) => "last_evaluation_error=none".to_string(),
        Err(_) => {
            degraded = true;
            "last_evaluation_error=unavailable".to_string()
        }
    };

    let status = if degraded { Status::Warn } else { Status::Ok };
    Check::new(
        "Compiled rules",
        status,
        format!(
            "enabled={enabled} artifact_present={artifact_present} artifact_valid={artifact_valid} artifact_error={} rule_count={} last_compile_epoch={} {compile_detail} {evaluation_detail}",
            artifact_error.unwrap_or("none"),
            optional_count(rule_count),
            optional_epoch(last_compile_epoch),
        ),
    )
}

struct CompileDiagnosticSnapshot {
    latest_status: Option<String>,
    latest_epoch: Option<i64>,
    last_error_epoch: Option<i64>,
}

fn load_compile_diagnostics_for(
    conn: &Connection,
    project: &str,
) -> rusqlite::Result<CompileDiagnosticSnapshot> {
    let latest = conn
        .query_row(
            "SELECT status, occurred_at_epoch
             FROM preference_rule_diagnostics
             WHERE project = ?1 AND event_kind = 'compile'
             ORDER BY id DESC LIMIT 1",
            params![project],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    let last_error_epoch = conn
        .query_row(
            "SELECT occurred_at_epoch
             FROM preference_rule_diagnostics
             WHERE project = ?1 AND event_kind = 'compile' AND status = 'error'
             ORDER BY id DESC LIMIT 1",
            params![project],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    Ok(CompileDiagnosticSnapshot {
        latest_status: latest.as_ref().map(|(status, _)| status.clone()),
        latest_epoch: latest.map(|(_, epoch)| epoch),
        last_error_epoch,
    })
}

fn artifact_error_label(kind: crate::rules::ArtifactLoadErrorKind) -> &'static str {
    match kind {
        crate::rules::ArtifactLoadErrorKind::Missing => "missing",
        crate::rules::ArtifactLoadErrorKind::Read => "read",
        crate::rules::ArtifactLoadErrorKind::Parse => "parse",
        crate::rules::ArtifactLoadErrorKind::Validate => "validate",
    }
}

fn optional_text(value: Option<&str>) -> &str {
    value.unwrap_or("none")
}

fn optional_epoch(value: Option<i64>) -> String {
    value.map_or_else(|| "none".to_string(), |value| value.to_string())
}

fn optional_count(value: Option<usize>) -> String {
    value.map_or_else(|| "none".to_string(), |value| value.to_string())
}

fn optional_error(value: Option<i64>) -> String {
    value.map_or_else(|| "none".to_string(), |value| format!("present@{value}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::ScopedTestDataDir;
    use crate::rules::{
        write_artifact_atomic, CompiledRule, CompiledRulesArtifact, EvaluationDiagnosticCode,
        RuleAction, RuleOverrideState, RulePredicate,
    };

    #[test]
    fn reports_artifact_and_errors_without_rule_payload_secrets() -> anyhow::Result<()> {
        let scoped = ScopedTestDataDir::new("doctor-compiled-rules");
        let conn = crate::db::open_db()?;
        let project = "/repo/private";
        let artifact = CompiledRulesArtifact::new(
            1234,
            vec![CompiledRule {
                rule_id: "pref-1-1".to_string(),
                source_memory_id: 1,
                reinforcement_count: 3,
                action: RuleAction::Warn,
                override_state: RuleOverrideState {
                    disabled: false,
                    action_override: None,
                },
                predicate: RulePredicate::CommandRegex {
                    pattern: "TOP_SECRET_PATTERN".to_string(),
                    message: "TOP_SECRET_MESSAGE".to_string(),
                },
            }],
        );
        write_artifact_atomic(
            crate::rules::artifact_path_for_project(&scoped.path, project),
            &artifact,
        )?;
        conn.execute(
            "INSERT INTO preference_rule_diagnostics
             (project, event_kind, status, message, occurred_at_epoch)
             VALUES (?1, 'compile', 'error', 'TOP_SECRET_DIAGNOSTIC', 1240)",
            [project],
        )?;
        crate::rules::log_evaluation_error_once_with_diagnostic(
            &scoped.path,
            Some("doctor-session"),
            Some(project),
            &[EvaluationDiagnosticCode::RuleEvaluation],
            "TOP_SECRET_EVALUATION",
        );

        let check = check_compiled_rules_for(Some(&conn), &scoped.path, project, true);

        assert!(matches!(check.status, Status::Warn));
        assert!(check.detail.contains("artifact_present=true"));
        assert!(check.detail.contains("artifact_valid=true"));
        assert!(check.detail.contains("rule_count=1"));
        assert!(check.detail.contains("last_compile_epoch=1234"));
        assert!(check.detail.contains("last_compile_error=present@1240"));
        assert!(check
            .detail
            .contains("last_evaluation_error=rule_evaluation@"));
        assert!(!check.detail.contains("TOP_SECRET"), "{}", check.detail);
        Ok(())
    }

    #[test]
    fn disabled_rollout_reports_missing_artifact_without_warning() {
        let data_dir = crate::rules::test_support::test_dir("doctor-rules-disabled");

        let check = check_compiled_rules_for(None, &data_dir, "/repo", false);

        assert!(matches!(check.status, Status::Ok));
        assert!(check.detail.contains("enabled=false"));
        assert!(check.detail.contains("artifact_present=false"));
    }

    #[test]
    fn host_capabilities_are_explicit_and_honest() {
        let checks = check_rule_enforcement_capabilities();

        assert_eq!(checks.len(), 2);
        assert!(checks[0].detail.contains("block=supported"));
        assert!(checks[1].detail.contains("block=unsupported"));
        assert!(checks[1].detail.contains("pre_execution=false"));
    }
}
