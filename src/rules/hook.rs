use std::fs::OpenOptions;
use std::path::Path;

use anyhow::{bail, Context, Result};
use fs2::FileExt;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::{
    artifact_path_for_project, evaluate_artifact_file_with_codes, EvaluationDiagnosticCode,
    EvaluationInput, EvaluationVerdict, RuleMatch,
};

#[derive(Debug, Deserialize)]
struct PreToolUsePayload {
    session_id: Option<String>,
    cwd: String,
    hook_event_name: String,
    tool_name: String,
    tool_input: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuleHookEvaluation {
    pub session_id: Option<String>,
    pub output: Option<Value>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DetailedRuleHookEvaluation {
    pub evaluation: RuleHookEvaluation,
    pub project: Option<String>,
    pub diagnostic_codes: Vec<EvaluationDiagnosticCode>,
}

pub fn session_id_hint(raw: &str) -> Option<String> {
    serde_json::from_str::<Value>(raw)
        .ok()?
        .get("session_id")?
        .as_str()
        .map(str::to_string)
}

pub(crate) fn project_hint(raw: &str) -> Option<String> {
    let cwd = serde_json::from_str::<Value>(raw)
        .ok()?
        .get("cwd")?
        .as_str()?
        .to_string();
    Some(crate::db::project_from_cwd(&cwd))
}

pub fn evaluate_pre_tool_use(
    raw: &str,
    host: Option<&str>,
    data_dir: &Path,
    enabled: bool,
) -> Result<RuleHookEvaluation> {
    evaluate_pre_tool_use_with_diagnostics(raw, host, data_dir, enabled)
        .map(|detailed| detailed.evaluation)
}

pub(crate) fn evaluate_pre_tool_use_with_diagnostics(
    raw: &str,
    host: Option<&str>,
    data_dir: &Path,
    enabled: bool,
) -> Result<DetailedRuleHookEvaluation> {
    let host = host
        .map(crate::runtime_config::normalize_host)
        .unwrap_or_else(|| "unknown".to_string());
    if host != crate::runtime_config::CLAUDE_HOST {
        bail!("compiled command-rule enforcement is unsupported for host '{host}'");
    }

    if !enabled {
        return Ok(DetailedRuleHookEvaluation {
            evaluation: RuleHookEvaluation {
                session_id: session_id_hint(raw),
                output: None,
                diagnostics: Vec::new(),
            },
            project: None,
            diagnostic_codes: Vec::new(),
        });
    }

    let payload: PreToolUsePayload =
        serde_json::from_str(raw).context("parse Claude PreToolUse hook input")?;
    if payload.hook_event_name != "PreToolUse" {
        bail!(
            "rules eval expected hook_event_name=PreToolUse, got '{}'",
            payload.hook_event_name
        );
    }
    if payload.tool_name != "Bash" {
        bail!(
            "rules eval expected tool_name=Bash, got '{}'",
            payload.tool_name
        );
    }
    let command = payload
        .tool_input
        .get("command")
        .and_then(Value::as_str)
        .filter(|command| !command.trim().is_empty())
        .context("Claude PreToolUse Bash input is missing tool_input.command")?;
    let project = crate::db::project_from_cwd(&payload.cwd);
    let coded_outcome = evaluate_artifact_file_with_codes(
        artifact_path_for_project(data_dir, &project),
        &EvaluationInput {
            command: command.to_string(),
        },
    );
    let diagnostic_codes = coded_outcome.diagnostic_codes;
    let outcome = coded_outcome.outcome;
    let diagnostics = outcome
        .diagnostics
        .into_iter()
        .map(|diagnostic| sanitize_diagnostic(&diagnostic.message))
        .collect::<Vec<_>>();
    if !diagnostics.is_empty() {
        return Ok(DetailedRuleHookEvaluation {
            evaluation: RuleHookEvaluation {
                session_id: payload.session_id,
                output: None,
                diagnostics,
            },
            project: Some(project),
            diagnostic_codes,
        });
    }

    Ok(DetailedRuleHookEvaluation {
        evaluation: RuleHookEvaluation {
            session_id: payload.session_id,
            output: render_hook_output(outcome.verdict, &outcome.matches),
            diagnostics,
        },
        project: Some(project),
        diagnostic_codes,
    })
}

fn render_hook_output(verdict: EvaluationVerdict, matches: &[RuleMatch]) -> Option<Value> {
    match verdict {
        EvaluationVerdict::Allow => None,
        EvaluationVerdict::Warn => {
            let message = static_match_message("warning", matches);
            Some(json!({
                "systemMessage": message,
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "additionalContext": message,
                }
            }))
        }
        EvaluationVerdict::Block => {
            let message = static_match_message("blocked", matches);
            Some(json!({
                "systemMessage": message,
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "deny",
                    "permissionDecisionReason": message,
                }
            }))
        }
    }
}

fn static_match_message(disposition: &str, matches: &[RuleMatch]) -> String {
    let mut source_ids = matches
        .iter()
        .map(|matched| matched.source_memory_id)
        .collect::<Vec<_>>();
    source_ids.sort_unstable();
    source_ids.dedup();
    let sources = source_ids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "remem compiled preference rule {disposition} for source memory(s) {sources}; inspect with `remem rules list`."
    )
}

fn sanitize_diagnostic(message: &str) -> String {
    let single_line = message
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>();
    crate::db::truncate_str(&single_line, 1000).to_string()
}

pub fn log_evaluation_error_once(data_dir: &Path, session_id: Option<&str>, message: &str) {
    log_evaluation_error_once_with_diagnostic(
        data_dir,
        session_id,
        None,
        &[EvaluationDiagnosticCode::HookInput],
        message,
    );
}

pub(crate) fn log_evaluation_error_once_with_diagnostic(
    data_dir: &Path,
    session_id: Option<&str>,
    project: Option<&str>,
    codes: &[EvaluationDiagnosticCode],
    message: &str,
) {
    let Some(session_key) = session_id.filter(|session_id| !session_id.trim().is_empty()) else {
        crate::log::error("rules-eval", &sanitize_diagnostic(message));
        return;
    };
    let marker_dir = super::evaluation_marker_dir(data_dir);
    if let Err(error) = std::fs::create_dir_all(&marker_dir) {
        crate::log::error(
            "rules-eval",
            &format!(
                "could not create evaluation diagnostic marker directory: {error}; {}",
                sanitize_diagnostic(message)
            ),
        );
        return;
    }
    let digest = Sha256::digest(session_key.as_bytes());
    let marker = marker_dir.join(format!("{digest:x}"));
    let claim = marker_dir.join(format!(".{digest:x}.claim"));
    let claim_file = match OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&claim)
    {
        Ok(file) => file,
        Err(error) => {
            crate::log::error(
                "rules-eval",
                &format!(
                    "could not open evaluation diagnostic claim: {error}; {}",
                    sanitize_diagnostic(message)
                ),
            );
            return;
        }
    };
    crate::log::set_private_permissions(&claim);
    if let Err(error) = claim_file.lock_exclusive() {
        crate::log::error(
            "rules-eval",
            &format!(
                "could not lock evaluation diagnostic claim: {error}; {}",
                sanitize_diagnostic(message)
            ),
        );
        return;
    }
    match super::upsert_evaluation_error_record(&marker, data_dir, project, codes) {
        Ok(true) => crate::log::error("rules-eval", &sanitize_diagnostic(message)),
        Ok(false) => {}
        Err(error) => crate::log::error(
            "rules-eval",
            &format!(
                "could not publish evaluation diagnostic marker: {error:#}; {}",
                sanitize_diagnostic(message)
            ),
        ),
    }
}

#[cfg(test)]
mod tests;
