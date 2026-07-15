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

    let evaluated = evaluate_pre_tool_use_with_diagnostics(
        &hook_input(&project, "session-missing", "npm install"),
        Some("claude-code"),
        &data_dir,
        true,
    )?;

    assert!(evaluated.evaluation.output.is_none());
    assert_eq!(evaluated.evaluation.diagnostics.len(), 1);
    assert!(evaluated.evaluation.diagnostics[0].contains("artifact missing"));
    log_evaluation_error_once_with_diagnostic(
        &data_dir,
        evaluated.evaluation.session_id.as_deref(),
        evaluated.project.as_deref(),
        &evaluated.diagnostic_codes,
        &evaluated.evaluation.diagnostics.join("; "),
    );
    let project_key = crate::db::project_from_cwd(&project.to_string_lossy());
    let record = crate::rules::load_evaluation_error(&data_dir, &project_key)?
        .latest
        .context("evaluation diagnostic marker")?;
    assert_eq!(
        record.codes,
        vec![crate::rules::EvaluationDiagnosticCode::ArtifactMissing]
    );
    Ok(())
}

#[test]
fn successful_evaluation_does_not_add_or_rewrite_diagnostic_marker() -> Result<()> {
    let data_dir = test_dir("hook-recovery-data");
    let project = test_dir("hook-recovery-project");
    std::fs::create_dir_all(&project)?;
    let input = hook_input(&project, "session-recovery", "npm install");

    let failed =
        evaluate_pre_tool_use_with_diagnostics(&input, Some("claude-code"), &data_dir, true)?;
    log_evaluation_error_once_with_diagnostic(
        &data_dir,
        failed.evaluation.session_id.as_deref(),
        failed.project.as_deref(),
        &failed.diagnostic_codes,
        &failed.evaluation.diagnostics.join("; "),
    );
    let marker_dir = crate::rules::evaluation_marker_dir(&data_dir);
    let mut markers = std::fs::read_dir(&marker_dir)?.collect::<std::io::Result<Vec<_>>>()?;
    markers.retain(|entry| !entry.file_name().to_string_lossy().starts_with('.'));
    let marker = markers.first().context("evaluation marker")?.path();
    let before = std::fs::read(&marker)?;

    write_rule(&data_dir, &project, RuleAction::Warn)?;
    let recovered =
        evaluate_pre_tool_use_with_diagnostics(&input, Some("claude-code"), &data_dir, true)?;

    assert!(recovered.evaluation.diagnostics.is_empty());
    assert_eq!(std::fs::read(marker)?, before);
    let visible_markers = std::fs::read_dir(marker_dir)?
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
        .count();
    assert_eq!(visible_markers, 1);
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
fn invalid_hook_input_can_record_a_project_scoped_closed_code() -> Result<()> {
    let data_dir = test_dir("hook-invalid-input-data");
    let project = test_dir("hook-invalid-input-project");
    let raw = json!({
        "session_id": "session-invalid-input",
        "cwd": project,
        "hook_event_name": "PreToolUse",
        "tool_name": "Read",
        "tool_input": {}
    })
    .to_string();
    let error = evaluate_pre_tool_use(&raw, Some("claude-code"), &data_dir, true)
        .expect_err("non-Bash hook input should fail open through the CLI");
    let project = project_hint(&raw).context("project hint")?;

    log_evaluation_error_once_with_diagnostic(
        &data_dir,
        session_id_hint(&raw).as_deref(),
        Some(&project),
        &[EvaluationDiagnosticCode::HookInput],
        &error.to_string(),
    );

    let record = crate::rules::load_evaluation_error(&data_dir, &project)?
        .latest
        .context("project-scoped evaluation diagnostic")?;
    assert_eq!(record.codes, vec![EvaluationDiagnosticCode::HookInput]);
    Ok(())
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
    let markers = std::fs::read_dir(marker_dir)?
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
        .collect::<Vec<_>>();
    assert_eq!(markers.len(), 1);
    let name = markers[0].file_name().to_string_lossy().to_string();
    assert_eq!(name.len(), 64);
    assert!(!name.contains("private-session"));
    Ok(())
}

#[test]
fn concurrent_errors_log_and_publish_once_per_session() -> Result<()> {
    let scoped = crate::db::test_support::ScopedTestDataDir::new("hook-diagnostic-concurrent");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let mut handles = Vec::new();
    for index in 0..8 {
        let data_dir = scoped.path.clone();
        let barrier = std::sync::Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            log_evaluation_error_once_with_diagnostic(
                &data_dir,
                Some("concurrent-session"),
                Some("/repo/concurrent"),
                &[EvaluationDiagnosticCode::ArtifactMissing],
                &format!("concurrent-marker-{index}"),
            );
        }));
    }
    for handle in handles {
        handle.join().expect("diagnostic writer should not panic");
    }

    let marker_dir = crate::rules::evaluation_marker_dir(&scoped.path);
    let visible_markers = std::fs::read_dir(marker_dir)?
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
        .count();
    assert_eq!(visible_markers, 1);
    let log = std::fs::read_to_string(scoped.path.join("remem.log"))?;
    assert_eq!(
        log.lines()
            .filter(|line| line.contains("concurrent-marker-"))
            .count(),
        1
    );
    Ok(())
}

#[test]
fn contended_diagnostic_claim_never_blocks_hook_return() -> Result<()> {
    let data_dir = test_dir("hook-diagnostic-contention");
    let marker_dir = crate::rules::evaluation_marker_dir(&data_dir);
    std::fs::create_dir_all(&marker_dir)?;
    let digest = Sha256::digest(b"contended-session");
    let claim = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(marker_dir.join(format!(".{digest:x}.claim")))?;
    FileExt::lock_exclusive(&claim)?;

    let (done, returned) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        log_evaluation_error_once_with_diagnostic(
            &data_dir,
            Some("contended-session"),
            Some("/repo"),
            &[EvaluationDiagnosticCode::ArtifactMissing],
            "contended marker",
        );
        done.send(()).expect("contention result receiver");
    });
    returned
        .recv_timeout(std::time::Duration::from_secs(1))
        .context("diagnostic publication blocked on its claim")?;
    handle.join().expect("diagnostic writer should not panic");
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
