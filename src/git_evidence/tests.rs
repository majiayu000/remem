use std::path::Path;
use std::process::Command;

use super::*;

fn run_git(repo: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").args(args).current_dir(repo).output()?;
    anyhow::ensure!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn init_repo(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)?;
    run_git(path, &["init"])?;
    run_git(
        path,
        &["config", "user.email", "remem-test@example.invalid"],
    )?;
    run_git(path, &["config", "user.name", "Remem Test"])?;
    Ok(())
}

fn commit(repo: &Path, file: &str, content: &str, message: &str) -> Result<String> {
    std::fs::write(repo.join(file), content)?;
    run_git(repo, &["add", file])?;
    run_git(repo, &["commit", "-m", message])?;
    run_git(repo, &["rev-parse", "HEAD"])
}

fn codex_call(call_id: &str, command: &str, workdir: &Path) -> String {
    serde_json::json!({
        "type": "response_item",
        "payload": {
            "type": "function_call",
            "name": "exec_command",
            "call_id": call_id,
            "arguments": serde_json::json!({
                "cmd": command,
                "workdir": workdir,
            }).to_string(),
        }
    })
    .to_string()
}

fn codex_output(call_id: &str, exit_code: i32, output: &str) -> String {
    serde_json::json!({
        "type": "response_item",
        "payload": {
            "type": "function_call_output",
            "call_id": call_id,
            "output": format!(
                "Chunk ID: test\nWall time: 0.1 seconds\nProcess exited with code {exit_code}\nFinal output:\n{output}"
            ),
        }
    })
    .to_string()
}

#[test]
fn commit_command_requires_commit_as_last_success_chained_segment() -> Result<()> {
    assert!(is_supported_commit_command("git commit -m done")?);
    assert!(is_supported_commit_command(
        "git add src/lib.rs && git -c user.name=test commit -m done"
    )?);
    assert!(!is_supported_commit_command("echo git commit -m fake")?);
    assert!(!is_supported_commit_command("git commit -m done; true")?);
    assert!(!is_supported_commit_command("git commit -m done || true")?);
    assert!(!is_supported_commit_command(
        "git commit -m done | tee log"
    )?);
    Ok(())
}

#[test]
fn parses_standard_git_commit_output_candidate() -> Result<()> {
    assert_eq!(
        commit_candidate_from_output("[main (root-commit) a1b2c3d] first\n")?.as_deref(),
        Some("a1b2c3d")
    );
    assert_eq!(
        commit_candidate_from_output("[detached HEAD deadbeef] amend\n")?.as_deref(),
        Some("deadbeef")
    );
    assert!(commit_candidate_from_output("[main abcd] too short\n")?.is_none());
    Ok(())
}

#[test]
fn codex_transcript_captures_multiple_successful_commits_within_boundary() -> Result<()> {
    let test_dir = crate::db::test_support::ScopedTestDataDir::new("codex-commit-evidence");
    let repo = test_dir.path.join("repo");
    init_repo(&repo)?;
    let sha_a = commit(&repo, "a.txt", "a", "commit a")?;
    let sha_b = commit(&repo, "b.txt", "b", "commit b")?;
    let lines = [
        codex_call("call-a", "git commit -m 'commit a'", &repo),
        codex_output(
            "call-a",
            0,
            &format!("[main (root-commit) {}] commit a", &sha_a[..7]),
        ),
        codex_call("call-b", "git add b.txt && git commit -m 'commit b'", &repo),
        codex_output("call-b", 0, &format!("[main {}] commit b", &sha_b[..7])),
    ];
    let transcript = test_dir.path.join("rollout.jsonl");
    let bounded = format!("{}\n", lines.join("\n"));
    std::fs::write(&transcript, &bounded)?;
    let boundary = bounded.len() as u64;
    std::fs::write(
        &transcript,
        format!(
            "{bounded}{}\n{}\n",
            codex_call("after", "git commit -m after", &repo),
            codex_output("after", 0, "[main deadbeef] after")
        ),
    )?;

    let evidence = from_codex_transcript(
        transcript.to_string_lossy().as_ref(),
        boundary,
        repo.to_string_lossy().as_ref(),
    )?;

    let actual = evidence
        .iter()
        .map(|item| item.metadata.sha.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let expected = [sha_a.as_str(), sha_b.as_str()]
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(actual, expected);
    assert_eq!(
        evidence
            .iter()
            .find(|item| item.metadata.sha == sha_a)
            .and_then(|item| item.metadata.branch.as_deref()),
        None,
        "historical evidence must not inherit the branch of the current HEAD"
    );
    assert!(evidence
        .iter()
        .find(|item| item.metadata.sha == sha_b)
        .and_then(|item| item.metadata.branch.as_deref())
        .is_some());
    Ok(())
}

#[test]
fn failed_explicit_commit_does_not_create_evidence() -> Result<()> {
    let event = ParsedHookEvent {
        session_id: "failed-commit".to_string(),
        cwd: Some("/tmp".to_string()),
        project: "/tmp".to_string(),
        reference_time_epoch: None,
        tool_name: "Bash".to_string(),
        tool_input: Some(serde_json::json!({"command": "git commit -m failed"})),
        tool_response: Some(serde_json::json!({
            "exitCode": 1,
            "stdout": "[main deadbeef] fake"
        })),
    };
    let summary = EventSummary {
        event_type: "bash".to_string(),
        summary: "failed".to_string(),
        detail: None,
        files_json: None,
        exit_code: Some(1),
    };
    assert!(from_observed_event(&event, &summary)?.is_empty());
    Ok(())
}

#[test]
fn successful_commit_without_sha_proof_is_not_an_extraction_error() -> Result<()> {
    let event = ParsedHookEvent {
        session_id: "quiet-commit".to_string(),
        cwd: Some("/tmp".to_string()),
        project: "/tmp".to_string(),
        reference_time_epoch: None,
        tool_name: "Bash".to_string(),
        tool_input: Some(serde_json::json!({"command": "git commit -q -m done"})),
        tool_response: Some(serde_json::json!({"stdout": ""})),
    };
    let summary = EventSummary {
        event_type: "bash".to_string(),
        summary: "quiet commit".to_string(),
        detail: None,
        files_json: None,
        exit_code: Some(0),
    };

    assert!(from_observed_event(&event, &summary)?.is_empty());
    Ok(())
}

#[test]
fn leading_command_output_cannot_spoof_quiet_commit_evidence() -> Result<()> {
    let test_dir = crate::db::test_support::ScopedTestDataDir::new("git-evidence-spoof");
    let repo = test_dir.path.join("repo");
    init_repo(&repo)?;
    let spoofed_sha = commit(&repo, "a.txt", "a", "commit a")?;
    let actual_sha = commit(&repo, "b.txt", "b", "commit b")?;
    assert_ne!(spoofed_sha, actual_sha);
    let spoofed_output = format!("[main {}] spoof", &spoofed_sha[..7]);
    let event = ParsedHookEvent {
        session_id: "spoofed-quiet-commit".to_string(),
        cwd: Some(repo.to_string_lossy().into_owned()),
        project: repo.to_string_lossy().into_owned(),
        reference_time_epoch: None,
        tool_name: "Bash".to_string(),
        tool_input: Some(serde_json::json!({
            "command": format!("printf '{spoofed_output}\\n' && git commit -q -m done")
        })),
        tool_response: Some(serde_json::json!({"stdout": spoofed_output})),
    };
    let summary = EventSummary {
        event_type: "bash".to_string(),
        summary: "quiet commit with leading output".to_string(),
        detail: None,
        files_json: None,
        exit_code: Some(0),
    };

    assert!(from_observed_event(&event, &summary)?.is_empty());
    Ok(())
}

#[test]
fn unsupported_shell_syntax_is_not_a_commit_extraction_error() -> Result<()> {
    let event = ParsedHookEvent {
        session_id: "ordinary-shell".to_string(),
        cwd: Some("/tmp".to_string()),
        project: "/tmp".to_string(),
        reference_time_epoch: None,
        tool_name: "Bash".to_string(),
        tool_input: Some(serde_json::json!({"command": "echo ok;"})),
        tool_response: Some(serde_json::json!({"stdout": "ok"})),
    };
    let summary = EventSummary {
        event_type: "bash".to_string(),
        summary: "ordinary shell".to_string(),
        detail: None,
        files_json: None,
        exit_code: Some(0),
    };

    assert!(from_observed_event(&event, &summary)?.is_empty());
    Ok(())
}

#[test]
fn observed_commit_resolves_git_dash_c_effective_workdir() -> Result<()> {
    let test_dir = crate::db::test_support::ScopedTestDataDir::new("git-evidence-dash-c");
    let repo = test_dir.path.join("repo");
    init_repo(&repo)?;
    let sha = commit(&repo, "a.txt", "a", "commit a")?;
    let event = ParsedHookEvent {
        session_id: "dash-c".to_string(),
        cwd: Some(test_dir.path.to_string_lossy().into_owned()),
        project: test_dir.path.to_string_lossy().into_owned(),
        reference_time_epoch: None,
        tool_name: "Bash".to_string(),
        tool_input: Some(serde_json::json!({
            "command": format!("git -C {} commit -m done", repo.display())
        })),
        tool_response: Some(serde_json::json!({
            "stdout": format!("[main {}] commit a", &sha[..7])
        })),
    };
    let summary = EventSummary {
        event_type: "bash".to_string(),
        summary: "commit".to_string(),
        detail: None,
        files_json: None,
        exit_code: Some(0),
    };

    let evidence = from_observed_event(&event, &summary)?;
    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].metadata.sha, sha);
    assert_eq!(
        Path::new(&evidence[0].metadata.repo_path),
        repo.canonicalize()?
    );
    Ok(())
}

#[test]
fn observed_commit_resolves_chained_cd_effective_workdir() -> Result<()> {
    let test_dir = crate::db::test_support::ScopedTestDataDir::new("git-evidence-chained-cd");
    let repo = test_dir.path.join("repo");
    init_repo(&repo)?;
    let sha = commit(&repo, "a.txt", "a", "commit a")?;
    let event = ParsedHookEvent {
        session_id: "chained-cd".to_string(),
        cwd: Some(test_dir.path.to_string_lossy().into_owned()),
        project: test_dir.path.to_string_lossy().into_owned(),
        reference_time_epoch: None,
        tool_name: "Bash".to_string(),
        tool_input: Some(serde_json::json!({"command": "cd repo && git commit -m done"})),
        tool_response: Some(serde_json::json!({
            "stdout": format!("[main {}] commit a", &sha[..7])
        })),
    };
    let summary = EventSummary {
        event_type: "bash".to_string(),
        summary: "commit".to_string(),
        detail: None,
        files_json: None,
        exit_code: Some(0),
    };

    let evidence = from_observed_event(&event, &summary)?;
    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].metadata.sha, sha);
    assert_eq!(
        Path::new(&evidence[0].metadata.repo_path),
        repo.canonicalize()?
    );
    Ok(())
}
