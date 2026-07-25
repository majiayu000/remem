use super::*;

#[test]
fn direct_git_commit_help_cannot_inject_viewer_output() -> Result<()> {
    let test_dir =
        crate::db::test_support::ScopedTestDataDir::new("git-evidence-commit-help-spoof");
    let repo = test_dir.path.join("repo");
    init_repo(&repo)?;
    let spoofed_sha = commit(&repo, "a.txt", "a", "commit a")?;
    let spoofed_output = format!("[main {}] spoof", &spoofed_sha[..7]);
    let command = format!(
        "GIT_MAN_VIEWER=spoof git -c 'man.spoof.cmd=printf \\\"{spoofed_output}\\\\n\\\"' commit --help"
    );
    let event = ParsedHookEvent {
        session_id: "commit-help-spoof".to_string(),
        cwd: Some(repo.to_string_lossy().into_owned()),
        project: repo.to_string_lossy().into_owned(),
        reference_time_epoch: None,
        tool_name: "Bash".to_string(),
        tool_input: Some(serde_json::json!({"command": command})),
        tool_response: Some(serde_json::json!({"stdout": spoofed_output})),
    };
    let summary = EventSummary {
        event_type: "bash".to_string(),
        summary: "crafted commit help".to_string(),
        detail: None,
        files_json: None,
        exit_code: Some(0),
    };

    assert!(!is_supported_commit_command(
        event
            .tool_input
            .as_ref()
            .and_then(|value| value.get("command"))
            .and_then(Value::as_str)
            .expect("test command should exist")
    ));
    assert!(from_observed_event(&event, &summary)?.is_empty());
    Ok(())
}

#[test]
fn leading_git_add_help_cannot_inject_viewer_output() -> Result<()> {
    let test_dir = crate::db::test_support::ScopedTestDataDir::new("git-evidence-add-help-spoof");
    let repo = test_dir.path.join("repo");
    init_repo(&repo)?;
    let spoofed_sha = commit(&repo, "a.txt", "a", "commit a")?;
    let actual_sha = commit(&repo, "b.txt", "b", "commit b")?;
    assert_ne!(spoofed_sha, actual_sha);
    let spoofed_output = format!("[main {}] spoof", &spoofed_sha[..7]);
    let command = format!(
        "GIT_MAN_VIEWER=spoof git -c 'man.spoof.cmd=printf \\\"{spoofed_output}\\\\n\\\"' add --help && git commit -q -m done"
    );
    let event = ParsedHookEvent {
        session_id: "add-help-spoof".to_string(),
        cwd: Some(repo.to_string_lossy().into_owned()),
        project: repo.to_string_lossy().into_owned(),
        reference_time_epoch: None,
        tool_name: "Bash".to_string(),
        tool_input: Some(serde_json::json!({"command": command})),
        tool_response: Some(serde_json::json!({"stdout": spoofed_output})),
    };
    let summary = EventSummary {
        event_type: "bash".to_string(),
        summary: "crafted add help before quiet commit".to_string(),
        detail: None,
        files_json: None,
        exit_code: Some(0),
    };

    assert!(!is_supported_commit_command(
        event
            .tool_input
            .as_ref()
            .and_then(|value| value.get("command"))
            .and_then(Value::as_str)
            .expect("test command should exist")
    ));
    assert!(from_observed_event(&event, &summary)?.is_empty());
    Ok(())
}

#[test]
fn quiet_commit_callback_output_is_not_commit_proof() -> Result<()> {
    let test_dir = crate::db::test_support::ScopedTestDataDir::new("git-evidence-quiet-callback");
    let repo = test_dir.path.join("repo");
    init_repo(&repo)?;
    let spoofed_sha = commit(&repo, "a.txt", "a", "commit a")?;
    let spoofed_output = format!("[main {}] callback", &spoofed_sha[..7]);
    let command = "git commit -q -m done";
    assert!(is_supported_commit_command(command));
    let event = ParsedHookEvent {
        session_id: "quiet-callback-spoof".to_string(),
        cwd: Some(repo.to_string_lossy().into_owned()),
        project: repo.to_string_lossy().into_owned(),
        reference_time_epoch: None,
        tool_name: "Bash".to_string(),
        tool_input: Some(serde_json::json!({"command": command})),
        tool_response: Some(serde_json::json!({"stdout": spoofed_output})),
    };
    let summary = EventSummary {
        event_type: "bash".to_string(),
        summary: "quiet commit callback output".to_string(),
        detail: None,
        files_json: None,
        exit_code: Some(0),
    };

    assert!(from_observed_event(&event, &summary)?.is_empty());
    Ok(())
}

#[test]
fn redirected_commit_summary_cannot_leave_callback_as_proof() -> Result<()> {
    let test_dir =
        crate::db::test_support::ScopedTestDataDir::new("git-evidence-redirected-summary");
    let repo = test_dir.path.join("repo");
    init_repo(&repo)?;
    let spoofed_sha = commit(&repo, "a.txt", "a", "commit a")?;
    let spoofed_output = format!("[main {}] callback", &spoofed_sha[..7]);
    let command = "git commit -m done > /dev/null";
    assert!(!is_supported_commit_command(command));
    let event = ParsedHookEvent {
        session_id: "redirected-summary-spoof".to_string(),
        cwd: Some(repo.to_string_lossy().into_owned()),
        project: repo.to_string_lossy().into_owned(),
        reference_time_epoch: None,
        tool_name: "Bash".to_string(),
        tool_input: Some(serde_json::json!({"command": command})),
        tool_response: Some(serde_json::json!({"stdout": spoofed_output})),
    };
    let summary = EventSummary {
        event_type: "bash".to_string(),
        summary: "redirected commit summary".to_string(),
        detail: None,
        files_json: None,
        exit_code: Some(0),
    };

    assert!(from_observed_event(&event, &summary)?.is_empty());
    Ok(())
}

#[test]
fn shell_comment_cannot_expose_an_unexecuted_commit() -> Result<()> {
    let test_dir = crate::db::test_support::ScopedTestDataDir::new("git-evidence-shell-comment");
    let repo = test_dir.path.join("repo");
    init_repo(&repo)?;
    let spoofed_sha = commit(&repo, "a.txt", "a", "commit a")?;
    let spoofed_output = format!("[main {}] filter", &spoofed_sha[..7]);
    let command = "git add tracked.txt # && git commit -m done";
    let event = ParsedHookEvent {
        session_id: "commented-out-commit".to_string(),
        cwd: Some(repo.to_string_lossy().into_owned()),
        project: repo.to_string_lossy().into_owned(),
        reference_time_epoch: None,
        tool_name: "Bash".to_string(),
        tool_input: Some(serde_json::json!({"command": command})),
        tool_response: Some(serde_json::json!({"stdout": spoofed_output})),
    };
    let summary = EventSummary {
        event_type: "bash".to_string(),
        summary: "commented-out commit".to_string(),
        detail: None,
        files_json: None,
        exit_code: Some(0),
    };

    assert!(from_observed_event(&event, &summary)?.is_empty());
    Ok(())
}
