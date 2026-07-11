use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

use crate::db::{self, test_support::ScopedTestDataDir};

fn run_summary_git(repo: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").args(args).current_dir(repo).output()?;
    anyhow::ensure!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn init_summary_repo(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)?;
    run_summary_git(path, &["init"])?;
    run_summary_git(
        path,
        &["config", "user.email", "remem-test@example.invalid"],
    )?;
    run_summary_git(path, &["config", "user.name", "Remem Test"])?;
    Ok(())
}

fn commit_summary_file(path: &Path, contents: &str, message: &str) -> Result<String> {
    std::fs::write(path.join("summary-evidence.txt"), contents)?;
    run_summary_git(path, &["add", "summary-evidence.txt"])?;
    run_summary_git(path, &["commit", "-m", message])?;
    run_summary_git(path, &["rev-parse", "HEAD"])
}

#[test]
fn replayed_stop_spill_uses_spilled_commit_evidence_when_head_moves() -> Result<()> {
    let test_dir = ScopedTestDataDir::new("stop-spill-git-snapshot");
    let repo = test_dir.path.join("repo");
    init_summary_repo(&repo)?;
    let sha_a = commit_summary_file(&repo, "commit-a", "commit a")?;
    let repo_str = repo.to_string_lossy();
    let metadata_a = crate::git_util::detect_commit_metadata(&repo_str)?
        .context("commit A metadata should be detectable")?;
    let evidence_a = crate::git_util::GitCommitEvidence {
        kind: crate::git_util::GitEvidenceKind::ObservedCommit,
        metadata: metadata_a,
        locator: Some("codex_call:test".to_string()),
    };
    let input = serde_json::json!({
        "session_id": "session-stop-spill-git",
        "cwd": repo,
        "last_assistant_message": "Finished the work"
    })
    .to_string();
    super::spill::spill_summary_hook_payload_with_git_evidence(
        &input,
        Some("codex-cli"),
        None,
        Some(&repo_str),
        &[evidence_a],
        &anyhow::anyhow!("database unavailable"),
    )?;
    let sha_b = commit_summary_file(&repo, "commit-b", "commit b")?;
    assert_ne!(sha_a, sha_b);

    let mut conn = db::open_db()?;
    let replayed = super::spill::replay_spilled_summary_hook_payloads(&conn, |conn, record| {
        super::hook::enqueue_summary_payload_with_git_evidence(
            conn,
            &record.input,
            record.host.as_deref(),
            record.profile.as_deref(),
            super::replay::SummaryPayloadOrigin::Replay,
            &record.git_evidence,
        )
    })?;
    assert_eq!(replayed, 1);
    let stored_sha: String = conn.query_row(
        "SELECT evidence.sha
         FROM captured_event_commits evidence
         JOIN captured_events events ON events.id = evidence.event_row_id
         WHERE events.session_id = 'session-stop-spill-git'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(stored_sha, sha_a);

    let task = db::claim_next_extraction_task(&mut conn, "worker-stop-link", 60)?
        .context("replayed Stop should enqueue SessionRollup")?;
    assert_eq!(task.task_kind, db::ExtractionTaskKind::SessionRollup);
    let linked = crate::captured_git::link_task_range(&mut conn, &task)?;
    assert_eq!(linked.len(), 1);
    assert_eq!(linked[0].sha, sha_a);
    let linked_sha: String = conn.query_row(
        "SELECT commits.sha
         FROM git_commit_sessions links
         JOIN git_commits commits ON commits.id = links.commit_id",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(linked_sha, sha_a);
    Ok(())
}
