use super::parse::parse_native_memory_frontmatter;
use super::path::extract_project_from_memory_path;

use crate::adapter::{EventSummary, ParsedHookEvent};
use crate::db::{self, test_support::ScopedTestDataDir};
use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

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

fn init_git_repo(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)?;
    run_git(path, &["init", "-b", "main"])?;
    run_git(
        path,
        &["config", "user.email", "remem-test@example.invalid"],
    )?;
    run_git(path, &["config", "user.name", "Remem Test"])?;
    Ok(())
}

fn commit_file(path: &Path, contents: &str, message: &str) -> Result<String> {
    std::fs::write(path.join("evidence.txt"), contents)?;
    run_git(path, &["add", "evidence.txt"])?;
    run_git(path, &["commit", "-m", message])?;
    run_git(path, &["rev-parse", "HEAD"])
}

fn observed_event(repo: &Path, session_id: &str) -> ParsedHookEvent {
    let repo = repo.to_string_lossy().to_string();
    ParsedHookEvent {
        session_id: session_id.to_string(),
        cwd: Some(repo.clone()),
        project: repo,
        reference_time_epoch: Some(1_700_000_000),
        tool_name: "Edit".to_string(),
        tool_input: Some(serde_json::json!({"file_path": "evidence.txt"})),
        tool_response: Some(serde_json::json!({"ok": true})),
    }
}

fn observed_summary() -> EventSummary {
    EventSummary {
        event_type: "file_edit".to_string(),
        summary: "Edited evidence.txt".to_string(),
        detail: None,
        files_json: Some("[\"evidence.txt\"]".to_string()),
        exit_code: None,
    }
}

#[test]
fn parse_frontmatter_full() {
    let content =
        "---\nname: my memory\ndescription: test\ntype: feedback\n---\nBody content here.";
    let (title, memory_type, body) = parse_native_memory_frontmatter(content);
    assert_eq!(title, "my memory");
    assert_eq!(memory_type, "preference");
    assert_eq!(body.trim(), "Body content here.");
}

#[test]
fn parse_frontmatter_missing() {
    let content = "Just plain text, no frontmatter.";
    let (title, memory_type, body) = parse_native_memory_frontmatter(content);
    assert_eq!(title, "Untitled memory");
    assert_eq!(memory_type, "discovery");
    assert_eq!(body, content);
}

#[test]
fn parse_frontmatter_project_type() {
    let content = "---\nname: deploy notes\ntype: project\n---\nContent.";
    let (_, memory_type, _) = parse_native_memory_frontmatter(content);
    assert_eq!(memory_type, "discovery");
}

#[test]
fn extract_project_from_path() {
    let path = "/Users/lifcc/.claude/projects/-Users-lifcc-Desktop-code-AI-tools-remem/memory/feedback_quality.md";
    let project = extract_project_from_memory_path(path);
    assert_eq!(project, "/Users/lifcc/Desktop/code/AI/tools/remem");
}

#[test]
fn extract_project_short_slug() {
    let path = "/Users/x/.claude/projects/-myproject/memory/foo.md";
    let project = extract_project_from_memory_path(path);
    assert_eq!(project, "/myproject");
}

#[tokio::test]
async fn successful_explicit_commit_persists_full_git_evidence() -> Result<()> {
    let test_dir = ScopedTestDataDir::new("observe-git-snapshot");
    let repo = test_dir.path.join("repo");
    init_git_repo(&repo)?;
    let sha = commit_file(&repo, "commit-a", "commit a")?;
    db::open_db()?;
    let input = serde_json::json!({
        "session_id": "session-git-snapshot",
        "cwd": repo,
        "hook_event_name": "PostToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "git commit -m 'commit a'"},
        "tool_response": {
            "stdout": format!("[main (root-commit) {}] commit a\n", &sha[..7])
        }
    })
    .to_string();

    super::hook::observe_input(&input, Some("claude-code")).await?;

    let conn = db::open_db()?;
    let (stored_sha, raw_metadata): (String, String) = conn.query_row(
        "SELECT evidence.sha, evidence.metadata_json
         FROM captured_event_commits evidence
         JOIN captured_events events ON events.id = evidence.event_row_id
         WHERE events.session_id = 'session-git-snapshot'
           AND evidence.evidence_kind = 'observed_commit'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let metadata: crate::git_util::GitCommitMetadata = serde_json::from_str(&raw_metadata)?;
    assert_eq!(stored_sha, sha);
    assert_eq!(metadata.sha, sha);
    assert_eq!(metadata.message.as_deref(), Some("commit a"));
    Ok(())
}

#[tokio::test]
async fn failure_hook_overrides_contradictory_zero_status_and_preserves_capture() -> Result<()> {
    let test_dir = ScopedTestDataDir::new("observe-unknown-git-status");
    let repo = test_dir.path.join("repo");
    init_git_repo(&repo)?;
    let spoofed_sha = commit_file(&repo, "baseline", "baseline")?;
    db::open_db()?;
    let input = serde_json::json!({
        "session_id": "session-unknown-git-status",
        "cwd": repo,
        "hook_event_name": "PostToolUseFailure",
        "tool_name": "Bash",
        "tool_input": {"command": "git commit -m failed"},
        "tool_response": {
            "exitCode": 0,
            "stdout": format!("[main {}] callback", &spoofed_sha[..7])
        }
    })
    .to_string();

    super::hook::observe_input(&input, Some("claude-code")).await?;

    let conn = db::open_db()?;
    let captured: i64 = conn.query_row(
        "SELECT COUNT(*) FROM captured_events
         WHERE session_id = 'session-unknown-git-status'",
        [],
        |row| row.get(0),
    )?;
    let evidence: i64 =
        conn.query_row("SELECT COUNT(*) FROM captured_event_commits", [], |row| {
            row.get(0)
        })?;
    assert_eq!(captured, 1);
    assert_eq!(evidence, 0);
    Ok(())
}

#[tokio::test]
async fn ordinary_edit_does_not_link_baseline_head() -> Result<()> {
    let test_dir = ScopedTestDataDir::new("observe-baseline-not-evidence");
    let repo = test_dir.path.join("repo");
    init_git_repo(&repo)?;
    commit_file(&repo, "baseline", "baseline")?;
    db::open_db()?;
    let input = serde_json::json!({
        "session_id": "session-baseline-not-evidence",
        "cwd": repo,
        "tool_name": "Edit",
        "tool_input": {"file_path": "evidence.txt"},
        "tool_response": {"ok": true}
    })
    .to_string();

    super::hook::observe_input(&input, Some("claude-code")).await?;

    let conn = db::open_db()?;
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM captured_event_commits", [], |row| {
        row.get(0)
    })?;
    assert_eq!(count, 0);
    let branch: Option<String> = conn.query_row(
        "SELECT json_extract(content_text, '$.git_branch')
         FROM captured_events
         WHERE session_id = 'session-baseline-not-evidence'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(branch.as_deref(), Some("main"));
    Ok(())
}

#[tokio::test]
async fn unresolvable_commit_evidence_preserves_observed_capture() -> Result<()> {
    let test_dir = ScopedTestDataDir::new("observe-unresolvable-git-evidence");
    let missing_repo = test_dir.path.join("missing-repo");
    db::open_db()?;
    let input = serde_json::json!({
        "session_id": "session-unresolvable-git-evidence",
        "cwd": missing_repo,
        "tool_name": "Bash",
        "tool_input": {"command": "git commit -m done"},
        "tool_response": {"stdout": "[main deadbeef] done"}
    })
    .to_string();

    super::hook::observe_input(&input, Some("claude-code")).await?;

    let conn = db::open_db()?;
    let captured: i64 = conn.query_row(
        "SELECT COUNT(*) FROM captured_events
         WHERE session_id = 'session-unresolvable-git-evidence'",
        [],
        |row| row.get(0),
    )?;
    let evidence: i64 =
        conn.query_row("SELECT COUNT(*) FROM captured_event_commits", [], |row| {
            row.get(0)
        })?;
    assert_eq!(captured, 1);
    assert_eq!(evidence, 0);
    Ok(())
}

#[test]
fn replayed_observe_spill_preserves_commit_snapshot_when_head_moves() -> Result<()> {
    let test_dir = ScopedTestDataDir::new("observe-spill-git-snapshot");
    let repo = test_dir.path.join("repo");
    init_git_repo(&repo)?;
    let sha_a = commit_file(&repo, "commit-a", "commit a")?;
    let event = observed_event(&repo, "session-spill-git-snapshot");
    let summary = observed_summary();
    let repo_str = repo.to_string_lossy();
    let metadata_a = crate::git_util::detect_commit_metadata(&repo_str)?
        .context("commit A metadata should be detectable")?;
    let evidence_a = crate::git_util::GitCommitEvidence {
        kind: crate::git_util::GitEvidenceKind::ObservedCommit,
        metadata: metadata_a,
        locator: Some("test_spill".to_string()),
    };
    super::spill::spill_capture_event_with_git_evidence(
        "claude-code",
        "tool_result-spill-git-snapshot",
        &event,
        &summary,
        &[evidence_a],
        super::spill::SPILL_REASON_DB_OPEN_FAILED,
        &anyhow::anyhow!("database unavailable"),
    )?;
    let sha_b = commit_file(&repo, "commit-b", "commit b")?;
    assert_ne!(sha_a, sha_b);

    let conn = db::open_db()?;
    assert_eq!(super::spill::replay_spilled_capture_events(&conn)?, 1);
    let stored_sha: String = conn.query_row(
        "SELECT evidence.sha
         FROM captured_event_commits evidence
         JOIN captured_events events ON events.id = evidence.event_row_id
         WHERE events.session_id = 'session-spill-git-snapshot'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(stored_sha, sha_a);
    assert_eq!(super::spill::replay_spilled_capture_events(&conn)?, 0);
    Ok(())
}

#[test]
fn replayed_observe_spill_without_snapshot_does_not_adopt_later_head() -> Result<()> {
    let test_dir = ScopedTestDataDir::new("observe-spill-no-git-snapshot");
    let repo = test_dir.path.join("repo");
    init_git_repo(&repo)?;
    let event = observed_event(&repo, "session-spill-no-git-snapshot");
    let summary = observed_summary();
    super::spill::spill_capture_event_with_git_evidence(
        "claude-code",
        "tool_result-spill-no-git-snapshot",
        &event,
        &summary,
        &[],
        super::spill::SPILL_REASON_DB_OPEN_FAILED,
        &anyhow::anyhow!("database unavailable"),
    )?;
    commit_file(&repo, "commit-after-spill", "later commit")?;

    let conn = db::open_db()?;
    assert_eq!(super::spill::replay_spilled_capture_events(&conn)?, 1);
    let evidence_count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM captured_event_commits evidence
         JOIN captured_events events ON events.id = evidence.event_row_id
         WHERE events.session_id = 'session-spill-no-git-snapshot'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(evidence_count, 0);
    Ok(())
}
