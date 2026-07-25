use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use super::*;
use crate::db::{CaptureEventInput, ExtractionTaskKind};

const PROJECT: &str = "/tmp/remem-commit-link";
const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn setup_conn() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    crate::migrate::run_migrations(&conn)?;
    Ok(conn)
}

fn metadata(sha: &str, message: &str) -> crate::git_util::GitCommitMetadata {
    crate::git_util::GitCommitMetadata {
        repo_path: PROJECT.to_string(),
        sha: sha.to_string(),
        short_sha: sha.chars().take(7).collect(),
        branch: Some("main".to_string()),
        message: Some(message.to_string()),
        authored_at_epoch: Some(1_700_000_000),
        changed_files: vec!["src/lib.rs".to_string()],
    }
}

fn evidence(sha: &str, message: &str) -> crate::git_util::GitCommitEvidence {
    crate::git_util::GitCommitEvidence {
        kind: crate::git_util::GitEvidenceKind::ObservedCommit,
        metadata: metadata(sha, message),
        locator: Some("test".to_string()),
    }
}

fn capture_with_metadata(
    conn: &Connection,
    host: &str,
    session_id: &str,
    event_id: &str,
    sha: &str,
) -> Result<i64> {
    let evidence = evidence(sha, event_id);
    let outcome = crate::db::record_captured_event_with_id_and_reference_time_and_git_evidence(
        conn,
        &CaptureEventInput {
            host,
            session_id,
            project: PROJECT,
            cwd: Some(PROJECT),
            event_type: "tool_result",
            role: None,
            tool_name: Some("Bash"),
            content: "git commit completed successfully",
            task_kind: Some(ExtractionTaskKind::ObservationExtract),
        },
        Some(event_id),
        Some(1_700_000_000),
        &[evidence],
    )?;
    outcome
        .extraction_task_id
        .context("captured event should enqueue ObservationExtract")
}

fn capture_without_metadata(conn: &Connection, session_id: &str, event_id: &str) -> Result<i64> {
    let outcome = crate::db::record_captured_event_with_id_and_reference_time_and_git_evidence(
        conn,
        &CaptureEventInput {
            host: "codex-cli",
            session_id,
            project: PROJECT,
            cwd: Some(PROJECT),
            event_type: "tool_result",
            role: None,
            tool_name: Some("Bash"),
            content: "non-git evidence",
            task_kind: Some(ExtractionTaskKind::ObservationExtract),
        },
        Some(event_id),
        Some(1_700_000_000),
        &[],
    )?;
    outcome
        .extraction_task_id
        .context("captured event should enqueue ObservationExtract")
}

fn claim(conn: &mut Connection, owner: &str) -> Result<db::ExtractionTask> {
    db::claim_next_extraction_task(conn, owner, 60)?
        .context("expected claimable ObservationExtract task")
}

fn no_observations() -> String {
    serde_json::json!({
        "no_observations": {"reason": "commit link is deterministic"}
    })
    .to_string()
}

fn one_observation() -> String {
    serde_json::json!({
        "observations": [{
            "type": "change",
            "title": "Commits captured",
            "subtitle": null,
            "narrative": "Captured commits were linked to the session.",
            "facts": [],
            "concepts": [],
            "files_read": [],
            "files_modified": ["src/lib.rs"],
            "confidence": 0.99
        }]
    })
    .to_string()
}

#[tokio::test]
async fn rollup_first_priority_still_reaches_captured_commit_link() -> Result<()> {
    let mut conn = setup_conn()?;
    capture_with_metadata(
        &conn,
        "codex-cli",
        "session-rollup-first",
        "event-observation",
        SHA_A,
    )?;
    crate::db::record_captured_event(
        &conn,
        &CaptureEventInput {
            host: "codex-cli",
            session_id: "session-rollup-first",
            project: PROJECT,
            cwd: Some(PROJECT),
            event_type: "session_stop",
            role: None,
            tool_name: None,
            content: r#"{"session_id":"session-rollup-first"}"#,
            task_kind: Some(ExtractionTaskKind::SessionRollup),
        },
    )?;

    let rollup = claim(&mut conn, "worker-rollup")?;
    assert_eq!(rollup.task_kind, ExtractionTaskKind::SessionRollup);
    db::mark_extraction_task_done(
        &conn,
        rollup.id,
        "worker-rollup",
        rollup.high_watermark_event_id,
    )?;
    let observation = claim(&mut conn, "worker-observation")?;
    assert_eq!(
        observation.task_kind,
        ExtractionTaskKind::ObservationExtract
    );
    assert_eq!(observation.session_row_id, rollup.session_row_id);

    process_with_extractor(&mut conn, &observation, |_| async { Ok(no_observations()) }).await?;

    let (session_row_id, memory_session_id): (i64, String) = conn.query_row(
        "SELECT session_row_id, memory_session_id FROM git_commit_sessions",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(Some(session_row_id), observation.session_row_id);
    assert_eq!(
        memory_session_id,
        crate::session_rollup::rollup_memory_session_id(session_row_id)
    );
    Ok(())
}

#[tokio::test]
async fn captured_commit_links_even_when_model_returns_no_observations() -> Result<()> {
    let mut conn = setup_conn()?;
    capture_with_metadata(
        &conn,
        "codex-cli",
        "session-no-observations",
        "event-a",
        SHA_A,
    )?;
    let task = claim(&mut conn, "worker-a")?;

    let result =
        process_with_extractor(&mut conn, &task, |_| async { Ok(no_observations()) }).await?;

    assert_eq!(result, ObservationExtractResult::NoObservations);
    let observations: i64 =
        conn.query_row("SELECT COUNT(*) FROM observations", [], |row| row.get(0))?;
    let links: i64 = conn.query_row("SELECT COUNT(*) FROM git_commit_sessions", [], |row| {
        row.get(0)
    })?;
    assert_eq!(observations, 0);
    assert_eq!(links, 1);
    Ok(())
}

#[tokio::test]
async fn captured_commit_link_survives_ai_failure_and_retry() -> Result<()> {
    let mut conn = setup_conn()?;
    capture_with_metadata(&conn, "codex-cli", "session-ai-failure", "event-a", SHA_A)?;
    let task = claim(&mut conn, "worker-a")?;

    let error = process_with_extractor(&mut conn, &task, |_| async {
        Err(anyhow::anyhow!("simulated model timeout"))
    })
    .await
    .expect_err("model failure should remain visible");
    assert!(error.to_string().contains("simulated model timeout"));
    let linked_after_failure: i64 =
        conn.query_row("SELECT COUNT(*) FROM git_commit_sessions", [], |row| {
            row.get(0)
        })?;
    assert_eq!(linked_after_failure, 1);

    let retry =
        process_with_extractor(&mut conn, &task, |_| async { Ok(no_observations()) }).await?;
    assert_eq!(retry, ObservationExtractResult::NoObservations);
    let linked_after_retry: i64 =
        conn.query_row("SELECT COUNT(*) FROM git_commit_sessions", [], |row| {
            row.get(0)
        })?;
    assert_eq!(linked_after_retry, 1);
    Ok(())
}

#[tokio::test]
async fn captured_commit_range_links_all_commits_without_fabricating_observation_sha() -> Result<()>
{
    let mut conn = setup_conn()?;
    capture_with_metadata(
        &conn,
        "codex-cli",
        "session-multi-commit-range",
        "event-a",
        SHA_A,
    )?;
    capture_with_metadata(
        &conn,
        "codex-cli",
        "session-multi-commit-range",
        "event-b",
        SHA_B,
    )?;
    let task = claim(&mut conn, "worker-a")?;

    process_with_extractor(&mut conn, &task, |_| async { Ok(one_observation()) }).await?;

    let commits: i64 = conn.query_row("SELECT COUNT(*) FROM git_commits", [], |row| row.get(0))?;
    let links: i64 = conn.query_row("SELECT COUNT(*) FROM git_commit_sessions", [], |row| {
        row.get(0)
    })?;
    let observation_sha: Option<String> = conn.query_row(
        "SELECT commit_sha FROM observations WHERE session_row_id = ?1",
        [task.session_row_id],
        |row| row.get(0),
    )?;
    assert_eq!(commits, 2);
    assert_eq!(links, 2);
    assert_eq!(observation_sha, None);

    process_with_extractor(&mut conn, &task, |_| async { Ok(one_observation()) }).await?;
    let replay_links: i64 =
        conn.query_row("SELECT COUNT(*) FROM git_commit_sessions", [], |row| {
            row.get(0)
        })?;
    assert_eq!(replay_links, 2);
    Ok(())
}

#[tokio::test]
async fn captured_commit_link_is_bounded_to_each_claimed_range() -> Result<()> {
    let mut conn = setup_conn()?;
    let task_id = capture_with_metadata(
        &conn,
        "codex-cli",
        "session-range-boundary",
        "event-a",
        SHA_A,
    )?;
    let first = claim(&mut conn, "worker-a")?;
    capture_with_metadata(
        &conn,
        "codex-cli",
        "session-range-boundary",
        "event-b",
        SHA_B,
    )?;

    process_with_extractor(&mut conn, &first, |_| async { Ok(no_observations()) }).await?;
    let first_shas = conn
        .prepare("SELECT sha FROM git_commits ORDER BY sha")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(first_shas, vec![SHA_A.to_string()]);
    db::mark_extraction_task_done(&conn, task_id, "worker-a", first.high_watermark_event_id)?;

    let second = claim(&mut conn, "worker-b")?;
    process_with_extractor(&mut conn, &second, |_| async { Ok(no_observations()) }).await?;
    let all_shas = conn
        .prepare("SELECT sha FROM git_commits ORDER BY sha")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(all_shas, vec![SHA_A.to_string(), SHA_B.to_string()]);
    Ok(())
}

#[tokio::test]
async fn captured_commit_link_failure_is_diagnosable_and_retryable() -> Result<()> {
    let mut conn = setup_conn()?;
    capture_with_metadata(&conn, "codex-cli", "session-link-failure", "event-a", SHA_A)?;
    let task = claim(&mut conn, "worker-a")?;
    let session_row_id = task.session_row_id.context("session row should exist")?;
    conn.execute_batch(
        "CREATE TRIGGER fail_captured_commit_link
         BEFORE INSERT ON git_commit_sessions
         BEGIN
           SELECT RAISE(FAIL, 'forced captured commit link failure');
         END;",
    )?;

    let error = process_with_extractor(&mut conn, &task, |_| async { Ok(no_observations()) })
        .await
        .expect_err("link failure must not become a successful no-op");
    let error = format!("{error:#}");
    assert!(error.contains("captured commit link failed"));
    assert!(error.contains(&format!("session_row_id={session_row_id}")));
    assert!(error.contains("range=1..1"));
    assert!(error.contains(SHA_A));
    let commits_after_failure: i64 =
        conn.query_row("SELECT COUNT(*) FROM git_commits", [], |row| row.get(0))?;
    assert_eq!(
        commits_after_failure, 0,
        "link batch should roll back atomically"
    );

    conn.execute_batch("DROP TRIGGER fail_captured_commit_link;")?;
    process_with_extractor(&mut conn, &task, |_| async { Ok(no_observations()) }).await?;
    let links_after_retry: i64 =
        conn.query_row("SELECT COUNT(*) FROM git_commit_sessions", [], |row| {
            row.get(0)
        })?;
    assert_eq!(links_after_retry, 1);
    Ok(())
}

#[tokio::test]
async fn equal_raw_session_ids_from_different_hosts_keep_distinct_links() -> Result<()> {
    let mut conn = setup_conn()?;
    capture_with_metadata(
        &conn,
        "claude-code",
        "shared-raw-session",
        "event-claude",
        SHA_A,
    )?;
    capture_with_metadata(
        &conn,
        "codex-cli",
        "shared-raw-session",
        "event-codex",
        SHA_A,
    )?;

    let first = claim(&mut conn, "worker-a")?;
    process_with_extractor(&mut conn, &first, |_| async { Ok(no_observations()) }).await?;
    let second = claim(&mut conn, "worker-b")?;
    process_with_extractor(&mut conn, &second, |_| async { Ok(no_observations()) }).await?;

    let rows = conn
        .prepare(
            "SELECT session_row_id, memory_session_id
             FROM git_commit_sessions
             ORDER BY session_row_id",
        )?
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(rows.len(), 2);
    assert_ne!(rows[0].0, rows[1].0);
    assert_ne!(rows[0].1, rows[1].1);
    Ok(())
}

#[tokio::test]
async fn commit_lookup_selects_one_latest_rollup_summary() -> Result<()> {
    let mut conn = setup_conn()?;
    capture_with_metadata(
        &conn,
        "codex-cli",
        "session-latest-summary",
        "event-a",
        SHA_A,
    )?;
    let task = claim(&mut conn, "worker-a")?;
    process_with_extractor(&mut conn, &task, |_| async { Ok(no_observations()) }).await?;
    let session_row_id = task.session_row_id.context("session row should exist")?;
    let memory_session_id = crate::session_rollup::rollup_memory_session_id(session_row_id);
    for (request, covered_to, created_at) in [("older", 1, 10), ("latest", 2, 20)] {
        conn.execute(
            "INSERT INTO session_summaries
             (memory_session_id, project, request, created_at, created_at_epoch,
              session_row_id, covered_from_event_id, covered_to_event_id)
             VALUES (?1, ?2, ?3, '2026-01-01T00:00:00Z', ?4, ?5, 1, ?6)",
            params![
                memory_session_id,
                PROJECT,
                request,
                created_at,
                session_row_id,
                covered_to
            ],
        )?;
    }

    let lookup = crate::git_trace::lookup_commit(&conn, Some(PROJECT), SHA_A)?;
    assert_eq!(lookup.len(), 1);
    assert_eq!(lookup[0].sessions.len(), 1);
    assert_eq!(
        lookup[0].sessions[0]
            .summary
            .as_ref()
            .and_then(|summary| summary.request.as_deref()),
        Some("latest")
    );
    let reverse =
        crate::git_trace::commits_for_session(&conn, Some(PROJECT), "session-latest-summary", 10)?;
    assert_eq!(reverse.len(), 1);
    Ok(())
}

#[tokio::test]
async fn capture_without_commit_evidence_never_creates_a_link() -> Result<()> {
    let mut conn = setup_conn()?;
    capture_without_metadata(&conn, "session-no-evidence", "event-none")?;
    let task = claim(&mut conn, "worker-a")?;

    process_with_extractor(&mut conn, &task, |_| async { Ok(no_observations()) }).await?;

    let commits: i64 = conn.query_row("SELECT COUNT(*) FROM git_commits", [], |row| row.get(0))?;
    let links: i64 = conn.query_row("SELECT COUNT(*) FROM git_commit_sessions", [], |row| {
        row.get(0)
    })?;
    assert_eq!(commits, 0);
    assert_eq!(links, 0);
    Ok(())
}
