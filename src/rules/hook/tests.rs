use anyhow::Result;
use serde_json::json;

use super::*;
use crate::rules::test_support::{package_manager_rule, test_dir};
use crate::rules::{write_artifact_atomic, CompiledRulesArtifact, RuleAction, RuleOverrideState};

fn hook_input(project: &Path, session_id: &str, command: &str) -> String {
    json!({
        "session_id": session_id,
        "cwd": project,
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": command}
    })
    .to_string()
}

fn write_rule(data_dir: &Path, project: &Path, action: RuleAction) -> Result<()> {
    let project = crate::db::project_from_cwd(&project.to_string_lossy());
    let mut rule = package_manager_rule(action);
    rule.override_state = RuleOverrideState {
        disabled: false,
        action_override: (action == RuleAction::Block).then_some(RuleAction::Block),
    };
    let artifact = CompiledRulesArtifact::new(123, vec![rule]);
    write_artifact_atomic(artifact_path_for_project(data_dir, &project), &artifact)
}

#[test]
fn claude_warn_is_visible_without_overriding_host_permissions() -> Result<()> {
    let data_dir = test_dir("hook-warn-data");
    let project = test_dir("hook-warn-project");
    std::fs::create_dir_all(&project)?;
    write_rule(&data_dir, &project, RuleAction::Warn)?;

    let evaluated = evaluate_pre_tool_use(
        &hook_input(&project, "session-warn", "npm install"),
        Some("claude-code"),
        &data_dir,
        true,
    )?;
    let output = evaluated.output.context("warning output")?;

    assert!(evaluated.diagnostics.is_empty());
    assert!(output["systemMessage"].as_str().is_some_and(|message| {
        message.contains("warning") && message.contains("source memory(s) 123")
    }));
    assert_eq!(output["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    assert!(output["hookSpecificOutput"]
        .get("permissionDecision")
        .is_none());
    Ok(())
}

#[test]
fn claude_block_denies_before_command_execution() -> Result<()> {
    let data_dir = test_dir("hook-block-data");
    let project = test_dir("hook-block-project");
    std::fs::create_dir_all(&project)?;
    write_rule(&data_dir, &project, RuleAction::Block)?;

    let evaluated = evaluate_pre_tool_use(
        &hook_input(&project, "session-block", "npm install"),
        Some("claude-code"),
        &data_dir,
        true,
    )?;
    let output = evaluated.output.context("block output")?;

    assert_eq!(output["hookSpecificOutput"]["permissionDecision"], "deny");
    assert!(output["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .is_some_and(|message| message.contains("blocked")));
    Ok(())
}

#[test]
fn missing_artifact_fails_open_with_diagnostic() -> Result<()> {
    let data_dir = test_dir("hook-missing-data");
    let project = test_dir("hook-missing-project");
    std::fs::create_dir_all(&project)?;

    let evaluated = evaluate_pre_tool_use(
        &hook_input(&project, "session-missing", "npm install"),
        Some("claude-code"),
        &data_dir,
        true,
    )?;

    assert!(evaluated.output.is_none());
    assert_eq!(evaluated.diagnostics.len(), 1);
    assert!(evaluated.diagnostics[0].contains("artifact missing"));
    Ok(())
}

#[test]
fn unsupported_host_does_not_claim_command_enforcement() {
    let error = evaluate_pre_tool_use("{}", Some("codex-cli"), Path::new("/tmp"), true)
        .expect_err("Codex has no pre-execution Bash enforcement hook");
    assert!(error
        .to_string()
        .contains("unsupported for host 'codex-cli'"));
}

#[test]
fn disabled_rollout_ignores_existing_artifact() -> Result<()> {
    let data_dir = test_dir("hook-disabled-data");
    let project = test_dir("hook-disabled-project");
    std::fs::create_dir_all(&project)?;
    write_rule(&data_dir, &project, RuleAction::Block)?;

    let evaluated = evaluate_pre_tool_use(
        &hook_input(&project, "session-disabled", "npm install"),
        Some("claude-code"),
        &data_dir,
        false,
    )?;

    assert!(evaluated.output.is_none());
    assert!(evaluated.diagnostics.is_empty());
    Ok(())
}

#[test]
fn evaluation_error_marker_is_hashed_and_once_per_session() -> Result<()> {
    let data_dir = test_dir("hook-diagnostic-dedup");
    log_evaluation_error_once(&data_dir, Some("../../private-session"), "first error");
    log_evaluation_error_once(&data_dir, Some("../../private-session"), "second error");

    let marker_dir = data_dir
        .join("compiled_rules")
        .join(".evaluation-error-sessions");
    let markers = std::fs::read_dir(marker_dir)?.collect::<std::io::Result<Vec<_>>>()?;
    assert_eq!(markers.len(), 1);
    let name = markers[0].file_name().to_string_lossy().to_string();
    assert_eq!(name.len(), 64);
    assert!(!name.contains("private-session"));
    Ok(())
}

#[test]
fn evaluation_error_without_session_is_not_globally_suppressed() {
    let data_dir = test_dir("hook-diagnostic-no-session");

    log_evaluation_error_once(&data_dir, None, "malformed input");

    assert!(!data_dir
        .join("compiled_rules")
        .join(".evaluation-error-sessions")
        .exists());
}
