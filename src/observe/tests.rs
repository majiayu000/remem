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

// ---- GH-823 Cursor observe capture (B-007, B-011, B-014, B-016) ----

mod cursor_observe {
    use crate::db::{self, test_support::ScopedTestDataDir};
    use serde_json::json;

    const EMAIL_SENTINEL: &str = "gh823.sentinel+cursor@example.invalid";

    fn success_payload(tool_name: &str, tool_use_id: &str) -> Vec<u8> {
        json!({
            "conversation_id": "sess-cursor-obs",
            "generation_id": "gen-1",
            "model": "auto",
            "tool_name": tool_name,
            "tool_input": {"file_path": "README.md"},
            "tool_output": "file contents",
            "duration": 12,
            "tool_use_id": tool_use_id,
            "session_id": "sess-cursor-obs",
            "hook_event_name": "postToolUse",
            "cursor_version": "3.12.17",
            "workspace_roots": ["/tmp/remem-cursor"],
            "user_email": EMAIL_SENTINEL,
            "transcript_path": "/tmp/transcript.jsonl"
        })
        .to_string()
        .into_bytes()
    }

    fn failure_payload(tool_use_id: &str) -> Vec<u8> {
        json!({
            "conversation_id": "sess-cursor-obs",
            "generation_id": "gen-1",
            "model": "auto",
            "tool_name": "Read",
            "tool_input": {"file_path": "missing.md"},
            "error_message": "file not found",
            "failure_type": "error",
            "duration": 8,
            "tool_use_id": tool_use_id,
            "is_interrupt": false,
            "session_id": "sess-cursor-obs",
            "hook_event_name": "postToolUseFailure",
            "cursor_version": "3.12.17",
            "workspace_roots": ["/tmp/remem-cursor"],
            "user_email": EMAIL_SENTINEL,
            "transcript_path": "/tmp/transcript.jsonl"
        })
        .to_string()
        .into_bytes()
    }

    fn captured_events(conn: &rusqlite::Connection) -> Vec<(String, String, String)> {
        let mut statement = conn
            .prepare(
                "SELECT h.name, ce.event_id, ce.event_type
                 FROM captured_events ce JOIN hosts h ON h.id = ce.host_id
                 WHERE ce.session_id = 'sess-cursor-obs'
                 ORDER BY ce.id",
            )
            .expect("prepare");
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .expect("query");
        rows.collect::<Result<Vec<_>, _>>().expect("collect")
    }

    fn assert_sentinel_absent(conn: &rusqlite::Connection) {
        let leaked: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM captured_events WHERE content_text LIKE ?1",
                [format!("%{EMAIL_SENTINEL}%")],
                |row| row.get(0),
            )
            .expect("sentinel scan");
        assert_eq!(leaked, 0, "user_email sentinel reached captured_events");
    }

    #[tokio::test]
    async fn cursor_generic_success_captures_once_with_canonical_key() -> anyhow::Result<()> {
        let _dir = ScopedTestDataDir::new("cursor-observe-success");
        drop(db::open_db()?);

        crate::observe::observe_cursor_bytes(&success_payload("Read", "tu-1")).await?;
        // Replay of the same call maps to itself (idempotent).
        crate::observe::observe_cursor_bytes(&success_payload("Read", "tu-1")).await?;

        let conn = db::open_db()?;
        let events = captured_events(&conn);
        assert_eq!(
            events.len(),
            1,
            "canonical per-call capture is exactly once"
        );
        assert_eq!(events[0].0, "cursor", "B-011 canonical host value");
        assert_eq!(events[0].1, "cursor-tool:tu-1");
        assert_eq!(events[0].2, "tool_result");
        let legacy: i64 = conn.query_row(
            "SELECT COUNT(*) FROM events WHERE session_id = 'sess-cursor-obs'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(legacy, 1, "idempotent replay writes one legacy event");
        assert_sentinel_absent(&conn);
        Ok(())
    }

    #[tokio::test]
    async fn cursor_same_tool_calls_keep_distinct_canonical_keys() -> anyhow::Result<()> {
        let _dir = ScopedTestDataDir::new("cursor-observe-distinct");
        drop(db::open_db()?);

        crate::observe::observe_cursor_bytes(&success_payload("Read", "tu-a")).await?;
        crate::observe::observe_cursor_bytes(&success_payload("Read", "tu-b")).await?;

        let conn = db::open_db()?;
        let events = captured_events(&conn);
        assert_eq!(events.len(), 2, "two same-tool calls stay distinct");
        assert_eq!(events[0].1, "cursor-tool:tu-a");
        assert_eq!(events[1].1, "cursor-tool:tu-b");
        Ok(())
    }

    #[tokio::test]
    async fn cursor_failed_read_stores_existing_schema_failure_discriminator() -> anyhow::Result<()>
    {
        let _dir = ScopedTestDataDir::new("cursor-observe-failure");
        drop(db::open_db()?);

        crate::observe::observe_cursor_bytes(&failure_payload("tu-fail")).await?;

        let conn = db::open_db()?;
        let events = captured_events(&conn);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].2, "cursor_tool_failure");
        assert_eq!(events[0].1, "cursor-tool:tu-fail");
        assert_sentinel_absent(&conn);
        Ok(())
    }

    #[tokio::test]
    async fn cursor_dual_delivery_keeps_failure_precedence_both_orders() -> anyhow::Result<()> {
        let _dir = ScopedTestDataDir::new("cursor-observe-precedence");
        drop(db::open_db()?);

        // success then failure: promoted to the failure discriminator.
        crate::observe::observe_cursor_bytes(&success_payload("Read", "tu-x")).await?;
        crate::observe::observe_cursor_bytes(&failure_payload("tu-x")).await?;
        // failure then success: never downgraded.
        crate::observe::observe_cursor_bytes(&failure_payload("tu-y")).await?;
        {
            let mut payload = success_payload("Read", "tu-y");
            crate::observe::observe_cursor_bytes(&std::mem::take(&mut payload)).await?;
        }

        let conn = db::open_db()?;
        let events = captured_events(&conn);
        assert_eq!(events.len(), 2, "each call persists exactly once");
        for (_, event_id, event_type) in &events {
            assert_eq!(
                event_type, "cursor_tool_failure",
                "failure precedence lost for {event_id}"
            );
        }
        let projections: Vec<(String, String)> = conn
            .prepare(
                "SELECT ce.event_id, e.event_type
                 FROM captured_events ce
                 JOIN events e ON e.captured_event_id = ce.id
                 WHERE ce.session_id = 'sess-cursor-obs'
                 ORDER BY ce.event_id",
            )?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(
            projections,
            vec![
                (
                    "cursor-tool:tu-x".to_string(),
                    "cursor_tool_failure".to_string()
                ),
                (
                    "cursor-tool:tu-y".to_string(),
                    "cursor_tool_failure".to_string()
                ),
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn cursor_failure_projection_error_rolls_back_canonical_promotion() -> anyhow::Result<()>
    {
        let _dir = ScopedTestDataDir::new("cursor-observe-promotion-rollback");
        drop(db::open_db()?);
        crate::observe::observe_cursor_bytes(&success_payload("Read", "tu-rollback")).await?;

        let conn = db::open_db()?;
        conn.execute_batch(
            "CREATE TRIGGER fail_cursor_projection_update
             BEFORE UPDATE OF event_type ON events
             BEGIN
                 SELECT RAISE(FAIL, 'cursor projection blocked');
             END;",
        )?;
        drop(conn);

        let error = crate::observe::observe_cursor_bytes(&failure_payload("tu-rollback"))
            .await
            .expect_err("projection failure must fail the canonical promotion");
        assert!(error.to_string().contains("cursor projection blocked"));
        let conn = db::open_db()?;
        let before_retry: (String, String, i64) = conn.query_row(
            "SELECT ce.event_type, e.event_type,
                    (SELECT COUNT(*) FROM events WHERE captured_event_id = ce.id)
             FROM captured_events ce
             JOIN events e ON e.captured_event_id = ce.id
             WHERE ce.event_id = 'cursor-tool:tu-rollback'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_eq!(
            before_retry,
            ("tool_result".to_string(), "tool_result".to_string(), 1)
        );
        conn.execute_batch("DROP TRIGGER fail_cursor_projection_update")?;
        drop(conn);

        crate::observe::observe_cursor_bytes(&failure_payload("tu-rollback")).await?;
        let conn = db::open_db()?;
        let after_retry: (String, String, i64) = conn.query_row(
            "SELECT ce.event_type, e.event_type,
                    (SELECT COUNT(*) FROM events WHERE captured_event_id = ce.id)
             FROM captured_events ce
             JOIN events e ON e.captured_event_id = ce.id
             WHERE ce.event_id = 'cursor-tool:tu-rollback'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_eq!(
            after_retry,
            (
                "cursor_tool_failure".to_string(),
                "cursor_tool_failure".to_string(),
                1,
            )
        );
        Ok(())
    }

    #[tokio::test]
    async fn cursor_unknown_tool_name_uses_verbatim_generic_capture() -> anyhow::Result<()> {
        let _dir = ScopedTestDataDir::new("cursor-observe-unknown-tool");
        drop(db::open_db()?);

        crate::observe::observe_cursor_bytes(&success_payload("SomethingNew", "tu-new")).await?;
        crate::observe::observe_cursor_bytes(&success_payload("MCP:browser_tabs", "tu-mcp"))
            .await?;

        let conn = db::open_db()?;
        let tools: Vec<String> = conn
            .prepare(
                "SELECT tool_name FROM captured_events WHERE session_id = 'sess-cursor-obs' ORDER BY id",
            )?
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(
            tools,
            vec!["SomethingNew".to_string(), "MCP:browser_tabs".to_string()]
        );
        Ok(())
    }

    #[tokio::test]
    async fn cursor_invalid_payloads_produce_zero_writes() -> anyhow::Result<()> {
        let _dir = ScopedTestDataDir::new("cursor-observe-zero-writes");
        drop(db::open_db()?);

        let mut cases: Vec<Vec<u8>> = vec![
            b"not-json".to_vec(),
            success_payload("Task", "tu-task"),
            success_payload("Write", "tu-write"),
        ];
        // MCP-specific events stay unregistered under generic ownership.
        let mut mcp_specific = json!({
            "conversation_id": "sess-cursor-obs",
            "generation_id": "gen-1",
            "tool_name": "browser_tabs",
            "tool_input": "{}",
            "result_json": "{}",
            "duration": 5,
            "mcp_server_name": "cursor-ide-browser",
            "session_id": "sess-cursor-obs",
            "hook_event_name": "afterMCPExecution",
            "cursor_version": "3.12.17",
            "workspace_roots": ["/tmp/remem-cursor"],
            "user_email": EMAIL_SENTINEL,
            "transcript_path": "/tmp/transcript.jsonl"
        });
        cases.push(mcp_specific.to_string().into_bytes());
        mcp_specific["hook_event_name"] = json!("beforeMCPExecution");
        cases.push(mcp_specific.to_string().into_bytes());
        // Identity mismatch, blank tool_use_id, multi-root.
        for (field, value) in [
            ("conversation_id", json!("other-session")),
            ("tool_use_id", json!("")),
            ("workspace_roots", json!(["/a", "/b"])),
        ] {
            let mut payload: serde_json::Value =
                serde_json::from_slice(&success_payload("Read", "tu-z"))?;
            payload[field] = value;
            cases.push(payload.to_string().into_bytes());
        }

        for case in &cases {
            let error = crate::observe::observe_cursor_bytes(case).await;
            assert!(error.is_err(), "case must fail closed");
        }

        let conn = db::open_db()?;
        let captured: i64 =
            conn.query_row("SELECT COUNT(*) FROM captured_events", [], |row| row.get(0))?;
        let legacy: i64 = conn.query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))?;
        assert_eq!(
            (captured, legacy),
            (0, 0),
            "fail-closed paths must not write"
        );
        assert!(
            !crate::db::data_dir().join("capture-spill.jsonl").exists(),
            "fail-closed paths must not spill"
        );
        Ok(())
    }

    #[tokio::test]
    async fn cursor_db_open_failure_spills_sanitized_event_and_replays_failure_type(
    ) -> anyhow::Result<()> {
        let dir = ScopedTestDataDir::new("cursor-observe-spill-replay");
        std::fs::create_dir_all(&dir.path)?;
        // A stale schema makes hook DB open fail closed.
        let stale = rusqlite::Connection::open(dir.db_path())?;
        stale.execute("CREATE TABLE marker (id INTEGER PRIMARY KEY)", [])?;
        drop(stale);

        let error = crate::observe::observe_cursor_bytes(&failure_payload("tu-spill"))
            .await
            .expect_err("stale hook database should fail closed");
        assert!(error.to_string().contains("hook database open requires"));
        let spill_path = crate::db::data_dir().join("capture-spill.jsonl");
        assert!(spill_path.exists(), "failure event must spill");
        let spill = std::fs::read_to_string(&spill_path)?;
        assert!(
            !spill.contains(EMAIL_SENTINEL),
            "spill record must be built from the sanitized event"
        );
        assert!(spill.contains("cursor_tool_failure"));

        // Migrate the database, then replay through the normal path.
        std::fs::remove_file(dir.db_path())?;
        drop(db::open_db()?);
        crate::observe::observe_cursor_bytes(&success_payload("Read", "tu-after")).await?;

        let conn = db::open_db()?;
        let replayed: String = conn.query_row(
            "SELECT event_type FROM captured_events WHERE event_id = 'cursor-tool:tu-spill'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(
            replayed, "cursor_tool_failure",
            "failure discriminator must survive spill replay"
        );
        assert_sentinel_absent(&conn);
        Ok(())
    }
}
