use rusqlite::{params, Connection};

use crate::db::test_support::{
    reset_runtime_connection_open_count, runtime_connection_open_count, ScopedTestDataDir,
};
use crate::{db, memory};

use super::database::{
    check_capture_drops, check_database, check_pending_queue, check_raw_archive_ingest,
    check_temporal_facts, check_worker_daemon,
};
use super::health_action::{queue_actions, render_action_block};
use super::report::{run_doctor_with_writer, DoctorOptions};
use super::schema::{check_key_format, check_schema_migration};

struct ScopedCipherKeyEnv {
    previous: Option<std::ffi::OsString>,
}

impl ScopedCipherKeyEnv {
    fn remove() -> Self {
        let previous = std::env::var_os("REMEM_CIPHER_KEY");
        std::env::remove_var("REMEM_CIPHER_KEY");
        Self { previous }
    }

    fn set(value: String) -> Self {
        let previous = std::env::var_os("REMEM_CIPHER_KEY");
        std::env::set_var("REMEM_CIPHER_KEY", value);
        Self { previous }
    }
}

impl Drop for ScopedCipherKeyEnv {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.as_ref() {
            std::env::set_var("REMEM_CIPHER_KEY", previous);
        } else {
            std::env::remove_var("REMEM_CIPHER_KEY");
        }
    }
}

fn insert_tool_capture(
    conn: &Connection,
    session_id: &str,
    task_kind: Option<db::ExtractionTaskKind>,
) -> anyhow::Result<db::CaptureEventOutcome> {
    db::record_captured_event(
        conn,
        &db::CaptureEventInput {
            host: "codex-cli",
            session_id,
            project: "proj-a",
            cwd: None,
            event_type: "tool_result",
            role: None,
            tool_name: Some("Bash"),
            content: r#"{"tool_name":"test"}"#,
            task_kind,
        },
    )
}
#[test]
fn check_database_reports_shared_active_memory_count() {
    let _test_dir = ScopedTestDataDir::new("doctor-db");
    let conn = db::open_db().expect("db should open");
    memory::insert_memory(
        &conn,
        Some("session-1"),
        "proj-a",
        None,
        "active",
        "kept",
        "decision",
        None,
    )
    .expect("active memory insert should succeed");
    let archived_id = memory::insert_memory(
        &conn,
        Some("session-2"),
        "proj-a",
        None,
        "archived",
        "hidden",
        "decision",
        None,
    )
    .expect("archived memory insert should succeed");
    conn.execute(
        "UPDATE memories SET status = 'archived' WHERE id = ?1",
        params![archived_id],
    )
    .expect("archive update should succeed");

    let stats = db::query_system_stats(&conn).expect("system stats should load");
    let check = check_database(Some(&conn), None);
    assert_eq!(check.icon(), "ok");
    assert!(check
        .detail
        .contains(&format!("{} memories", stats.active_memories)));
}

#[test]
fn check_key_format_warns_for_legacy_key() -> anyhow::Result<()> {
    let test_dir = ScopedTestDataDir::new("doctor-key-format-legacy");
    let _env = ScopedCipherKeyEnv::remove();
    std::fs::create_dir_all(&test_dir.path)?;
    std::fs::write(test_dir.path.join(".key"), "3".repeat(64))?;

    let check = check_key_format();
    assert_eq!(check.icon(), "WARN");
    assert!(
        check.detail.contains("remem encrypt --rekey-raw"),
        "{}",
        check.detail
    );
    Ok(())
}

#[test]
fn check_key_format_accepts_raw_key() -> anyhow::Result<()> {
    let test_dir = ScopedTestDataDir::new("doctor-key-format-raw");
    let _env = ScopedCipherKeyEnv::remove();
    std::fs::create_dir_all(&test_dir.path)?;
    std::fs::write(test_dir.path.join(".key"), format!("v2:{}", "4".repeat(64)))?;

    let check = check_key_format();
    assert_eq!(check.icon(), "ok");
    assert!(check.detail.contains("raw-key"), "{}", check.detail);
    Ok(())
}

#[test]
fn check_key_format_reports_effective_env_key() -> anyhow::Result<()> {
    let test_dir = ScopedTestDataDir::new("doctor-key-format-env");
    let _env = ScopedCipherKeyEnv::set(format!("v2:{}", "6".repeat(64)));
    std::fs::create_dir_all(&test_dir.path)?;
    std::fs::write(test_dir.path.join(".key"), "3".repeat(64))?;

    let check = check_key_format();
    assert_eq!(check.icon(), "ok");
    assert!(
        check.detail.contains("REMEM_CIPHER_KEY"),
        "{}",
        check.detail
    );
    Ok(())
}

#[test]
fn health_action_queue_actions_are_empty_when_runtime_is_clear() {
    let actions = queue_actions(0, 0, 0, 0, 0, 0);
    assert!(actions.is_empty());
    assert!(render_action_block(&actions).is_empty());
}

#[test]
fn health_action_queue_actions_render_copy_paste_commands() {
    let actions = queue_actions(43, 1, 5, 2, 3, 4);
    let text = render_action_block(&actions);

    assert!(text.contains("Needs attention:"));
    assert!(text.contains("43 failed pending observations"));
    assert!(text.contains("inspect: remem pending list-failed --limit 20"));
    assert!(text.contains("preview retry: remem pending retry-failed --dry-run"));
    assert!(text.contains("1 expired processing pending observation"));
    assert!(text.contains("5 expired processing extraction tasks"));
    assert!(text.contains("2 failed jobs"));
    assert!(text.contains("3 stuck jobs"));
    assert!(text.contains("4 failed extraction tasks"));
    assert!(text.contains("inspect counts: remem status --json"));
    assert!(text.contains("recover: remem worker --once"));
}

#[test]
fn check_pending_queue_reports_shared_counts() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("doctor-pending");
    let conn = db::open_db().expect("db should open");
    db::enqueue_pending(
        &conn,
        "codex-cli",
        "session-1",
        "proj-a",
        "tool",
        None,
        None,
        None,
    )
    .expect("pending row insert should succeed");
    let failed_id = db::enqueue_pending(
        &conn,
        "codex-cli",
        "session-2",
        "proj-a",
        "tool",
        None,
        None,
        None,
    )
    .expect("failed row insert should succeed");
    conn.execute(
        "UPDATE pending_observations SET status = 'failed' WHERE id = ?1",
        params![failed_id],
    )
    .expect("failed status update should succeed");

    let job_id = db::enqueue_job(
        &conn,
        "codex-cli",
        db::JobType::Observation,
        "proj-a",
        Some("session-3"),
        "{}",
        1,
    )
    .expect("job insert should succeed");
    conn.execute(
        "UPDATE jobs SET state = 'processing', lease_expires_epoch = ?2 WHERE id = ?1",
        params![job_id, chrono::Utc::now().timestamp() - 1],
    )
    .expect("job update should succeed");
    let failed_job_id = db::enqueue_job(
        &conn,
        "codex-cli",
        db::JobType::Summary,
        "proj-a",
        Some("session-4"),
        "{}",
        1,
    )?;
    conn.execute(
        "UPDATE jobs SET state = 'failed' WHERE id = ?1",
        params![failed_job_id],
    )?;
    let capture = insert_tool_capture(
        &conn,
        "session-5",
        Some(db::ExtractionTaskKind::ObservationExtract),
    )?;
    let extraction_task_id = capture
        .extraction_task_id
        .ok_or_else(|| anyhow::anyhow!("capture should enqueue extraction task"))?;
    conn.execute(
        "UPDATE extraction_tasks SET status = 'failed' WHERE id = ?1",
        params![extraction_task_id],
    )?;

    let stats = db::query_system_stats(&conn).expect("system stats should load");
    let check = check_pending_queue(Some(&conn));
    assert_eq!(check.icon(), "WARN");
    let expected_counts = format!(
        "{} ready, {} delayed, {} processing ({} expired), {} failed pending; {} extraction tasks pending, {} processing ({} expired), {} failed; {} jobs pending, {} processing, {} failed, {} stuck",
        stats.ready_pending_observations,
        stats.delayed_pending_observations,
        stats.processing_pending_observations,
        stats.expired_processing_pending_observations,
        stats.failed_pending_observations,
        stats.pending_extraction_tasks,
        stats.processing_extraction_tasks,
        stats.expired_processing_extraction_tasks,
        stats.failed_extraction_tasks,
        stats.pending_jobs,
        stats.processing_jobs,
        stats.failed_jobs,
        stats.stuck_jobs,
    );
    assert!(check.detail.contains(&expected_counts), "{}", check.detail);
    assert!(
        check.detail.contains("will auto-recover"),
        "{}",
        check.detail
    );
    assert!(
        check
            .detail
            .contains("inspect: `remem pending list-failed --limit 20`"),
        "{}",
        check.detail
    );
    assert!(
        check
            .detail
            .contains("preview retry: `remem pending retry-failed --dry-run`"),
        "{}",
        check.detail
    );
    assert!(
        check
            .detail
            .contains("inspect counts: `remem status --json`"),
        "{}",
        check.detail
    );
    assert!(
        check.detail.contains("extraction tasks pending"),
        "{}",
        check.detail
    );
    assert!(
        check.detail.contains("recover: `remem worker --once`"),
        "{}",
        check.detail
    );
    Ok(())
}

#[test]
fn check_raw_archive_ingest_warns_on_recorded_failures() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("doctor-raw-ingest");
    let conn = db::open_db()?;
    conn.execute(
        "INSERT INTO raw_ingest_failures
         (project, session_id, source, transcript_path, error_kind, error_message,
          inserted, duplicates, parse_errors, insert_errors, created_at_epoch)
         VALUES ('proj-a', 'session-a', 'transcript', '/bad/path.jsonl',
                 'read_error', 'read failed', 0, 0, 0, 0, ?1)",
        params![chrono::Utc::now().timestamp()],
    )?;
    let check = check_raw_archive_ingest(Some(&conn));
    assert_eq!(check.icon(), "WARN");
    assert!(check.detail.contains("1 failure"), "{}", check.detail);
    assert!(check.detail.contains("read_error"), "{}", check.detail);
    assert!(check.detail.contains("/bad/path.jsonl"), "{}", check.detail);
    Ok(())
}

#[test]
fn check_capture_drops_is_ok_for_expected_hook_skips() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("doctor-capture-drops-expected");
    let conn = db::open_db()?;
    db::record_capture_drop(
        &conn,
        &db::CaptureDropInput {
            host: Some("codex-cli"),
            session_id: Some("session-a"),
            project: Some("proj-a"),
            tool_name: Some("Bash"),
            reason: "codex_bash_disabled",
            detail: Some("Codex Bash capture disabled"),
            spill_path: None,
            recovered_event_id: None,
        },
    )?;

    let check = check_capture_drops(Some(&conn));

    assert_eq!(check.icon(), "ok");
    assert!(
        check.detail.contains("1 expected hook skip/drop event"),
        "{}",
        check.detail
    );
    Ok(())
}

#[test]
fn check_capture_drops_is_ok_for_recovered_persistence_spills() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("doctor-capture-drops-recovered-persistence");
    let conn = db::open_db()?;
    let recovered_capture = insert_tool_capture(&conn, "session-a", None)?;
    db::record_capture_drop(
        &conn,
        &db::CaptureDropInput {
            host: Some("codex-cli"),
            session_id: Some("session-a"),
            project: Some("proj-a"),
            tool_name: Some("Edit"),
            reason: "capture_persistence_failed",
            detail: Some("events insert failed"),
            spill_path: Some("/tmp/capture-spill.jsonl"),
            recovered_event_id: Some(recovered_capture.event_row_id),
        },
    )?;

    let check = check_capture_drops(Some(&conn));

    assert_eq!(check.icon(), "ok");
    assert!(
        check.detail.contains("no actionable capture drops"),
        "{}",
        check.detail
    );
    Ok(())
}

#[test]
fn check_capture_drops_warns_on_actionable_drops() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("doctor-capture-drops-actionable");
    let conn = db::open_db()?;
    db::record_capture_drop(
        &conn,
        &db::CaptureDropInput {
            host: Some("codex-cli"),
            session_id: None,
            project: None,
            tool_name: None,
            reason: "adapter_mismatch",
            detail: Some("no capture adapter matched hook input"),
            spill_path: None,
            recovered_event_id: None,
        },
    )?;

    let check = check_capture_drops(Some(&conn));

    assert_eq!(check.icon(), "WARN");
    assert!(
        check.detail.contains("1 actionable capture drop"),
        "{}",
        check.detail
    );
    assert!(
        check.detail.contains("latest reason=adapter_mismatch"),
        "{}",
        check.detail
    );
    Ok(())
}

#[test]
fn check_temporal_facts_warns_when_fact_table_is_empty() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("doctor-temporal-facts-empty");
    let conn = db::open_db()?;
    memory::insert_memory(
        &conn,
        Some("session-1"),
        "proj-a",
        None,
        "source memory",
        "A source memory exists without temporal facts.",
        "decision",
        None,
    )?;
    let stats = db::query_memory_facts_stats(&conn)?;
    assert!(stats.table_exists);
    assert_eq!(stats.total, 0);
    assert_eq!(stats.retrieval_eligible, 0);

    let check = check_temporal_facts(Some(&conn));
    assert_eq!(check.icon(), "WARN");
    assert!(
        check
            .detail
            .contains("temporal retrieval can read memory_facts"),
        "{}",
        check.detail
    );
    assert!(
        check
            .detail
            .contains("production fact extraction is not populating"),
        "{}",
        check.detail
    );
    Ok(())
}

#[test]
fn check_temporal_facts_is_ok_when_store_has_no_source_data() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("doctor-temporal-facts-empty-store");
    let conn = db::open_db()?;
    let stats = db::query_memory_facts_stats(&conn)?;
    assert!(stats.table_exists);
    assert_eq!(stats.total, 0);
    assert_eq!(stats.retrieval_eligible, 0);
    assert_eq!(stats.active_memories, 0);
    assert_eq!(stats.captured_events, 0);

    let check = check_temporal_facts(Some(&conn));
    assert_eq!(check.icon(), "ok");
    assert!(
        check.detail.contains("no memories or captured events yet"),
        "{}",
        check.detail
    );
    Ok(())
}

#[test]
fn check_temporal_facts_warns_when_rows_are_not_retrievable() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("doctor-temporal-facts-unlinked");
    let conn = db::open_db()?;
    memory::insert_memory(
        &conn,
        Some("session-1"),
        "proj-a",
        None,
        "source memory",
        "A source memory exists with an unlinked temporal fact.",
        "decision",
        None,
    )?;
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO memory_facts
         (project, subject, predicate, object, valid_from_epoch, valid_to_epoch,
          learned_at_epoch, source_memory_id, source_event_ids, confidence, status,
          created_at_epoch, updated_at_epoch)
         VALUES ('proj-a', 'deploy', 'affects_project', 'prod', ?1, NULL,
                 ?1, NULL, '[]', 0.9, 'active', ?1, ?1)",
        params![now],
    )?;
    let stats = db::query_memory_facts_stats(&conn)?;
    assert_eq!(stats.total, 1);
    assert_eq!(stats.retrieval_eligible, 0);

    let check = check_temporal_facts(Some(&conn));
    assert_eq!(check.icon(), "WARN");
    assert!(
        check.detail.contains("0 of 1 fact row(s)"),
        "{}",
        check.detail
    );
    Ok(())
}

#[test]
fn check_temporal_facts_warns_when_source_memory_is_expired() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("doctor-temporal-facts-expired");
    let conn = db::open_db()?;
    let memory_id = memory::insert_memory(
        &conn,
        Some("session-1"),
        "proj-a",
        None,
        "expired memory",
        "A source memory exists with an expired temporal fact.",
        "decision",
        None,
    )?;
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "UPDATE memories SET expires_at_epoch = ?1 WHERE id = ?2",
        params![now - 1, memory_id],
    )?;
    conn.execute(
        "INSERT INTO memory_facts
         (project, subject, predicate, object, valid_from_epoch, valid_to_epoch,
          learned_at_epoch, source_memory_id, source_event_ids, confidence, status,
          created_at_epoch, updated_at_epoch)
         VALUES ('proj-a', 'deploy', 'affects_project', 'prod', ?1, NULL,
                 ?1, ?2, '[]', 0.9, 'active', ?1, ?1)",
        params![now, memory_id],
    )?;
    let stats = db::query_memory_facts_stats(&conn)?;
    assert_eq!(stats.total, 1);
    assert_eq!(stats.retrieval_eligible, 0);

    let check = check_temporal_facts(Some(&conn));
    assert_eq!(check.icon(), "WARN");
    assert!(
        check.detail.contains("0 of 1 fact row(s)"),
        "{}",
        check.detail
    );
    Ok(())
}

#[test]
fn check_temporal_facts_is_ok_when_fact_table_has_rows() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("doctor-temporal-facts-present");
    let conn = db::open_db()?;
    let memory_id = memory::insert_memory(
        &conn,
        Some("session-1"),
        "proj-a",
        None,
        "source memory",
        "A source memory exists with a retrievable temporal fact.",
        "decision",
        None,
    )?;
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO memory_facts
         (project, subject, predicate, object, valid_from_epoch, valid_to_epoch,
          learned_at_epoch, source_memory_id, source_event_ids, confidence, status,
          created_at_epoch, updated_at_epoch)
         VALUES ('proj-a', 'deploy', 'affects_project', 'prod', ?1, NULL,
                 ?1, ?2, '[]', 0.9, 'active', ?1, ?1)",
        params![now, memory_id],
    )?;
    let stats = db::query_memory_facts_stats(&conn)?;
    assert_eq!(stats.total, 1);
    assert_eq!(stats.retrieval_eligible, 1);

    let check = check_temporal_facts(Some(&conn));
    assert_eq!(check.icon(), "ok");
    assert!(
        check
            .detail
            .contains("1 linked active memory fact(s) available"),
        "{}",
        check.detail
    );
    Ok(())
}

#[test]
fn check_schema_migration_reads_encrypted_database() -> anyhow::Result<()> {
    let test_dir = ScopedTestDataDir::new("doctor-encrypted-schema");
    std::fs::create_dir_all(&test_dir.path)?;
    std::fs::write(test_dir.path.join(".key"), "doctor-schema-key")?;
    let conn = db::open_db()?;

    let check = check_schema_migration(Some(&conn), None);
    assert_eq!(check.icon(), "ok");
    assert!(check.detail.contains("up to date"), "got: {}", check.detail);
    Ok(())
}

#[test]
fn check_schema_migration_reports_v022_schema_drift() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("doctor-schema-drift");
    let conn = db::open_db()?;
    conn.execute_batch(
        "PRAGMA foreign_keys=OFF;
         DROP TABLE memory_state_keys;
         PRAGMA foreign_keys=ON;",
    )?;
    let check = check_schema_migration(Some(&conn), None);
    assert_eq!(check.icon(), "FAIL");
    assert!(
        check.detail.contains("schema drift"),
        "got: {}",
        check.detail
    );
    assert!(
        check
            .detail
            .contains("v022_memory_state_keys marked applied"),
        "got: {}",
        check.detail
    );
    assert!(
        check.detail.contains("table memory_state_keys"),
        "got: {}",
        check.detail
    );
    Ok(())
}

#[test]
fn run_doctor_with_writer_returns_outcome_and_emits_human_lines() {
    let _test_dir = ScopedTestDataDir::new("doctor-run-human");
    // Ensure DB exists so the database probe doesn't FAIL the run.
    let _ = db::open_db().expect("db should open");

    let mut buf: Vec<u8> = Vec::new();
    let outcome = run_doctor_with_writer(DoctorOptions::default(), &mut buf)
        .expect("run_doctor_with_writer should succeed");

    let text = String::from_utf8(buf).expect("output should be utf-8");
    assert!(text.contains("system check"));
    assert!(text.contains("Database"));
    assert!(text.contains("ms)"), "{text}");
    // Exit code is a function of fails/warns; the absolute counts depend on
    // host config (claude/codex hooks may or may not exist on the test
    // machine), but the contract — fails maps to exit 2 — must hold.
    if outcome.fails > 0 {
        assert_eq!(outcome.exit_code(), 2);
    }
}

#[test]
fn run_doctor_with_writer_opens_one_shared_database_connection() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("doctor-run-one-connection");
    let conn = db::open_db()?;
    drop(conn);

    reset_runtime_connection_open_count();
    let mut buf = Vec::new();
    let outcome = run_doctor_with_writer(
        DoctorOptions {
            json: false,
            quiet: true,
        },
        &mut buf,
    )?;

    assert!(outcome.exit_code() <= 2);
    assert_eq!(runtime_connection_open_count(), 1);
    Ok(())
}

#[test]
fn run_doctor_with_writer_emits_parseable_json() {
    let _test_dir = ScopedTestDataDir::new("doctor-run-json");
    let _ = db::open_db().expect("db should open");

    let mut buf: Vec<u8> = Vec::new();
    let outcome = run_doctor_with_writer(
        DoctorOptions {
            json: true,
            quiet: false,
        },
        &mut buf,
    )
    .expect("run_doctor_with_writer should succeed in json mode");

    let text = String::from_utf8(buf).expect("output should be utf-8");
    let parsed: serde_json::Value =
        serde_json::from_str(text.trim()).expect("output must be a single JSON object");
    assert_eq!(parsed["schema_version"], 2);
    assert!(parsed["version"].is_string());
    assert!(parsed["status"].is_string());
    assert!(parsed["elapsed_ms"].is_u64());
    let checks = parsed["checks"].as_array().expect("checks must be array");
    assert!(!checks.is_empty(), "doctor should always emit some checks");
    for check in checks {
        assert!(check["duration_ms"].is_u64(), "{check}");
    }
    assert_eq!(
        parsed["fails"].as_u64().unwrap_or(0) as usize,
        outcome.fails
    );
    assert_eq!(
        parsed["warns"].as_u64().unwrap_or(0) as usize,
        outcome.warns
    );
}

#[test]
fn run_doctor_with_writer_json_wins_over_quiet() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("doctor-run-json-quiet");
    let conn = db::open_db()?;
    drop(conn);

    let mut buf: Vec<u8> = Vec::new();
    let outcome = run_doctor_with_writer(
        DoctorOptions {
            json: true,
            quiet: true,
        },
        &mut buf,
    )?;

    let text = String::from_utf8(buf).expect("output should be utf-8");
    let parsed: serde_json::Value =
        serde_json::from_str(text.trim()).expect("output must be a single JSON object");
    assert_eq!(
        parsed["fails"].as_u64().unwrap_or_default() as usize,
        outcome.fails
    );
    assert!(parsed["checks"].is_array());
    assert!(parsed["elapsed_ms"].is_u64());
    Ok(())
}

#[test]
fn run_doctor_with_writer_quiet_suppresses_output() {
    let _test_dir = ScopedTestDataDir::new("doctor-run-quiet");
    let _ = db::open_db().expect("db should open");

    let mut buf: Vec<u8> = Vec::new();
    let _outcome = run_doctor_with_writer(
        DoctorOptions {
            json: false,
            quiet: true,
        },
        &mut buf,
    )
    .expect("run_doctor_with_writer should succeed in quiet mode");

    assert!(buf.is_empty(), "quiet mode must not write to stdout");
}

#[test]
fn check_worker_daemon_reports_healthy_heartbeat() {
    let _test_dir = ScopedTestDataDir::new("doctor-worker-healthy");
    let conn = db::open_db().expect("db should open");
    let now = chrono::Utc::now().timestamp();
    db::upsert_worker_heartbeat(
        &conn,
        "worker-daemon",
        i64::from(std::process::id()),
        now - 5,
        now - 5,
    )
    .expect("heartbeat should insert");
    let check = check_worker_daemon(Some(&conn));
    assert_eq!(check.icon(), "ok");
    assert!(check.detail.contains("healthy"));
    assert!(check.detail.contains("worker-daemon"));
}

#[test]
fn check_worker_daemon_reports_missing_as_fallback_ok() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("doctor-worker-missing");
    let conn = db::open_db()?;
    let check = check_worker_daemon(Some(&conn));
    assert_eq!(check.icon(), "ok");
    assert_eq!(
        check.detail,
        "not running; safe fallback when Stop hooks are installed: `remem worker --once`"
    );
    Ok(())
}
