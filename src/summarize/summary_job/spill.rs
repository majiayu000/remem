use anyhow::{Context, Result};
use fs2::FileExt;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static SUMMARY_SPILL_CLAIM_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SummaryHookSpillRecord {
    version: u32,
    pub(super) input: String,
    pub(super) host: Option<String>,
    pub(super) profile: Option<String>,
    db_error: String,
    created_at_epoch: i64,
}

pub(super) fn spill_summary_hook_payload(
    input: &str,
    host: Option<&str>,
    profile: Option<&str>,
    resolved_cwd: Option<&str>,
    db_error: &anyhow::Error,
) -> Result<PathBuf> {
    let path = summary_spill_path();
    let record = SummaryHookSpillRecord {
        version: 1,
        input: summary_spill_input(input, resolved_cwd, profile)?,
        host: host.map(crate::runtime_config::normalize_host),
        profile: profile.map(str::to_string),
        db_error: crate::db::truncate_str(
            &crate::db::capture::redact_capture_content(&db_error.to_string()),
            1000,
        )
        .to_string(),
        created_at_epoch: chrono::Utc::now().timestamp(),
    };
    append_record_to_spill(&path, &record)?;
    Ok(path)
}

pub(super) fn replay_spilled_summary_hook_payloads(
    conn: &Connection,
    mut replay: impl FnMut(&Connection, &SummaryHookSpillRecord) -> Result<()>,
) -> Result<usize> {
    let path = summary_spill_path();
    let claimed_path = claimed_summary_spill_path();
    match with_summary_spill_lock(|| {
        std::fs::rename(&path, &claimed_path)
            .with_context(|| format!("claim summary hook spill {}", path.display()))
    }) {
        Ok(()) => {}
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == ErrorKind::NotFound) =>
        {
            return Ok(0)
        }
        Err(error) => return Err(error),
    }
    let failed_path = failed_summary_spill_path_for_claim(&claimed_path);

    let result =
        replay_claimed_summary_hook_payloads(conn, &path, &claimed_path, &failed_path, &mut replay);
    if result.is_err() {
        restore_claimed_and_failed_spill(&claimed_path, &failed_path, &path);
    }
    result
}

fn replay_claimed_summary_hook_payloads(
    conn: &Connection,
    path: &Path,
    claimed_path: &Path,
    failed_path: &Path,
    replay: &mut impl FnMut(&Connection, &SummaryHookSpillRecord) -> Result<()>,
) -> Result<usize> {
    let contents = match std::fs::read_to_string(claimed_path) {
        Ok(contents) => contents,
        Err(error) => {
            restore_claimed_spill(claimed_path, path);
            return Err(error).with_context(|| format!("read {}", claimed_path.display()));
        }
    };

    let mut replayed = 0;
    for line in contents.lines().filter(|line| !line.trim().is_empty()) {
        match serde_json::from_str::<SummaryHookSpillRecord>(line) {
            Ok(record) => match replay(conn, &record) {
                Ok(()) => replayed += 1,
                Err(error) => append_failed_record(failed_path, &record, &error)?,
            },
            Err(error) => append_failed_line(failed_path, line, &error.into())?,
        }
    }

    if failed_path.exists() {
        append_file_to_spill_then_remove(path, failed_path)?;
    }
    match std::fs::remove_file(claimed_path) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("remove {}", claimed_path.display()));
        }
    }

    if replayed > 0 {
        crate::log::info(
            "summarize",
            &format!("replayed {replayed} spilled summary hook payload(s)"),
        );
    }
    Ok(replayed)
}

fn summary_spill_input(
    input: &str,
    resolved_cwd: Option<&str>,
    profile: Option<&str>,
) -> Result<String> {
    let mut payload: serde_json::Value = serde_json::from_str(input)?;
    let Some(obj) = payload.as_object_mut() else {
        return Ok(input.to_string());
    };
    if let Some(cwd) = resolved_cwd.filter(|cwd| !cwd.trim().is_empty()) {
        let needs_cwd = obj
            .get("cwd")
            .and_then(|value| value.as_str())
            .is_none_or(|value| value.trim().is_empty());
        if needs_cwd {
            obj.insert(
                "cwd".to_string(),
                serde_json::Value::String(cwd.to_string()),
            );
        }
    }
    if let Some(profile) = profile.map(str::trim).filter(|profile| !profile.is_empty()) {
        obj.insert(
            crate::runtime_config::MEMORY_AI_PROFILE_FIELD.to_string(),
            serde_json::Value::String(profile.to_string()),
        );
    }
    obj.remove("last_assistant_message");
    Ok(serde_json::to_string(&payload)?)
}

fn append_record_to_spill(path: &Path, record: &SummaryHookSpillRecord) -> Result<()> {
    let mut line = serde_json::to_vec(record)?;
    line.push(b'\n');
    append_bytes_to_spill(path, &line)
}

fn append_bytes_to_spill(path: &Path, contents: &[u8]) -> Result<()> {
    if contents.is_empty() {
        return Ok(());
    }
    with_summary_spill_lock(|| append_bytes_to_spill_unlocked(path, contents))
}

fn append_bytes_to_spill_unlocked(path: &Path, contents: &[u8]) -> Result<()> {
    if contents.is_empty() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create summary hook spill dir {}", parent.display()))?;
    }
    let mut file = append_open_options()
        .open(path)
        .with_context(|| format!("open summary hook spill {}", path.display()))?;
    file.write_all(contents)?;
    Ok(())
}

fn with_summary_spill_lock<T>(f: impl FnOnce() -> Result<T>) -> Result<T> {
    let lock_path = summary_spill_lock_path();
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create summary hook spill lock dir {}", parent.display()))?;
    }
    let lock_file = append_open_options()
        .open(&lock_path)
        .with_context(|| format!("open summary hook spill lock {}", lock_path.display()))?;
    lock_file
        .lock_exclusive()
        .with_context(|| format!("lock summary hook spill {}", lock_path.display()))?;
    let result = f();
    let unlock = lock_file
        .unlock()
        .with_context(|| format!("unlock summary hook spill {}", lock_path.display()));
    match (result, unlock) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(unlock_error)) => Err(error.context(unlock_error.to_string())),
    }
}

fn append_open_options() -> std::fs::OpenOptions {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

fn restore_claimed_and_failed_spill(claimed_path: &Path, failed_path: &Path, path: &Path) {
    if claimed_path.exists() {
        restore_claimed_spill(claimed_path, path);
        remove_redundant_failed_spill(failed_path);
    } else if failed_path.exists() {
        let result = append_file_to_spill_then_remove(path, failed_path);
        if let Err(error) = result {
            crate::log::error(
                "summarize",
                &format!("failed to restore failed summary hook spill: {error}"),
            );
        }
    }
}

fn remove_redundant_failed_spill(failed_path: &Path) {
    match std::fs::remove_file(failed_path) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => crate::log::warn(
            "summarize",
            &format!(
                "failed to remove redundant summary hook spill {}: {error}",
                failed_path.display()
            ),
        ),
    }
}

fn append_file_to_spill_then_remove(path: &Path, records_path: &Path) -> Result<()> {
    let contents = std::fs::read_to_string(records_path)
        .with_context(|| format!("read {}", records_path.display()))?;
    with_summary_spill_lock(|| {
        append_bytes_to_spill_unlocked(path, contents.as_bytes())?;
        std::fs::remove_file(records_path)
            .with_context(|| format!("remove {}", records_path.display()))
    })
}

fn restore_claimed_spill(claimed_path: &Path, path: &Path) {
    let result = append_file_to_spill_then_remove(path, claimed_path);
    if let Err(error) = result {
        crate::log::error(
            "summarize",
            &format!("failed to restore claimed summary hook spill: {error}"),
        );
    }
}

fn append_failed_line(path: &Path, line: &str, error: &anyhow::Error) -> Result<()> {
    let mut contents = line.as_bytes().to_vec();
    contents.push(b'\n');
    append_bytes_to_spill(path, &contents)?;
    crate::log::warn(
        "summarize",
        &format!("summary hook spill replay failed: {error}"),
    );
    Ok(())
}

fn append_failed_record(
    path: &Path,
    record: &SummaryHookSpillRecord,
    error: &anyhow::Error,
) -> Result<()> {
    append_record_to_spill(path, record)?;
    crate::log::warn(
        "summarize",
        &format!("summary hook spill replay failed: {error}"),
    );
    Ok(())
}

pub(super) fn summary_spill_path() -> PathBuf {
    crate::db::data_dir().join("summary-hook-spill.jsonl")
}

fn claimed_summary_spill_path() -> PathBuf {
    let sequence = SUMMARY_SPILL_CLAIM_COUNTER.fetch_add(1, Ordering::Relaxed);
    let now_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    crate::db::data_dir().join(format!(
        "summary-hook-spill.replay-{}-{now_nanos}-{sequence}.jsonl",
        std::process::id()
    ))
}

fn failed_summary_spill_path_for_claim(claimed_path: &Path) -> PathBuf {
    claimed_path.with_extension("failed.jsonl")
}

fn summary_spill_lock_path() -> PathBuf {
    crate::db::data_dir().join("summary-hook-spill.lock")
}

#[cfg(test)]
mod tests {
    use crate::db::{self, test_support::ScopedTestDataDir};

    use super::{
        replay_spilled_summary_hook_payloads, spill_summary_hook_payload, summary_spill_path,
    };

    #[tokio::test]
    async fn stale_db_spill_replays_after_schema_is_initialized() -> anyhow::Result<()> {
        let test_dir = ScopedTestDataDir::new("summary-hook-spill-replay");
        std::fs::create_dir_all(&test_dir.path)?;
        let setup = rusqlite::Connection::open(test_dir.db_path())?;
        setup.execute("CREATE TABLE marker (id INTEGER PRIMARY KEY)", [])?;
        drop(setup);
        let spilled_input = serde_json::json!({
            "session_id": "sess-summary-spilled",
            "cwd": "/tmp/remem"
        })
        .to_string();

        let err = super::super::hook::summarize_input(&spilled_input, Some("codex-cli"), None)
            .await
            .expect_err("stale hook database should spill and fail closed");

        assert!(
            err.to_string().contains("hook database open requires"),
            "unexpected error: {err:#}"
        );
        assert!(summary_spill_path().exists());
        std::fs::remove_file(test_dir.db_path())?;

        let conn = db::open_db()?;
        let now = chrono::Utc::now().timestamp();
        db::upsert_worker_heartbeat(
            &conn,
            "worker-daemon",
            i64::from(std::process::id()),
            now,
            now,
        )?;
        drop(conn);

        let current_input = serde_json::json!({
            "session_id": "sess-summary-current",
            "cwd": "/tmp/remem"
        })
        .to_string();
        super::super::hook::summarize_input(&current_input, Some("codex-cli"), None).await?;

        assert!(!summary_spill_path().exists());
        let conn = db::open_db()?;
        let summary_jobs: i64 = conn.query_row(
            "SELECT COUNT(*) FROM jobs
             WHERE job_type = 'summary'
               AND session_id IN ('sess-summary-spilled', 'sess-summary-current')",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(summary_jobs, 2);
        Ok(())
    }

    #[test]
    fn replay_preserves_spills_appended_after_claim() -> anyhow::Result<()> {
        let _test_dir = ScopedTestDataDir::new("summary-hook-spill-claim-race");
        let conn = db::open_db()?;
        let first_input = serde_json::json!({
            "session_id": "sess-summary-first",
            "cwd": "/tmp/remem"
        })
        .to_string();
        let later_input = serde_json::json!({
            "session_id": "sess-summary-later",
            "cwd": "/tmp/remem"
        })
        .to_string();
        spill_summary_hook_payload(
            &first_input,
            Some("codex-cli"),
            None,
            Some("/tmp/remem"),
            &anyhow::anyhow!("stale db"),
        )?;

        let mut wrote_later = false;
        let replayed = replay_spilled_summary_hook_payloads(&conn, |_conn, record| {
            assert_eq!(record.input, first_input);
            if !wrote_later {
                spill_summary_hook_payload(
                    &later_input,
                    Some("codex-cli"),
                    None,
                    Some("/tmp/remem"),
                    &anyhow::anyhow!("still stale"),
                )?;
                wrote_later = true;
            }
            Ok(())
        })?;

        assert_eq!(replayed, 1);
        let remaining = std::fs::read_to_string(summary_spill_path())?;
        assert!(remaining.contains("sess-summary-later"));
        assert!(!remaining.contains("sess-summary-first"));
        Ok(())
    }

    #[test]
    fn spill_payload_fills_cwd_and_omits_last_assistant_message() -> anyhow::Result<()> {
        let _test_dir = ScopedTestDataDir::new("summary-hook-spill-sanitize");
        let input = serde_json::json!({
            "session_id": "sess-summary-sensitive",
            "last_assistant_message": "private assistant answer"
        })
        .to_string();

        spill_summary_hook_payload(
            &input,
            Some("codex-cli"),
            Some("quality"),
            Some("/tmp/original-project"),
            &anyhow::anyhow!("stale db"),
        )?;

        let stored = std::fs::read_to_string(summary_spill_path())?;
        assert!(!stored.contains("private assistant answer"));
        let record: super::SummaryHookSpillRecord = serde_json::from_str(stored.trim())?;
        let payload: serde_json::Value = serde_json::from_str(&record.input)?;
        assert_eq!(payload["cwd"].as_str(), Some("/tmp/original-project"));
        assert_eq!(payload["remem_ai_profile"].as_str(), Some("quality"));
        assert!(payload.get("last_assistant_message").is_none());
        Ok(())
    }

    #[test]
    fn restore_claimed_spill_makes_records_visible_to_future_replay() -> anyhow::Result<()> {
        let _test_dir = ScopedTestDataDir::new("summary-hook-spill-restore-claim");
        std::fs::create_dir_all(db::data_dir())?;
        let claimed_path = db::data_dir().join("summary-hook-spill.replay-test.jsonl");
        let failed_path = super::failed_summary_spill_path_for_claim(&claimed_path);
        std::fs::write(
            &claimed_path,
            format!(
                "{}\n",
                r#"{"version":1,"input":"{\"session_id\":\"sess-restored\"}","host":"codex-cli","profile":null,"db_error":"stale","created_at_epoch":1}"#
            ),
        )?;
        std::fs::write(
            &failed_path,
            format!(
                "{}\n",
                r#"{"version":1,"input":"{\"session_id\":\"sess-duplicate\"}","host":"codex-cli","profile":null,"db_error":"stale","created_at_epoch":1}"#
            ),
        )?;
        std::fs::write(
            summary_spill_path(),
            format!(
                "{}\n",
                r#"{"version":1,"input":"{\"session_id\":\"sess-active\"}","host":"codex-cli","profile":null,"db_error":"stale","created_at_epoch":1}"#
            ),
        )?;

        super::restore_claimed_and_failed_spill(&claimed_path, &failed_path, &summary_spill_path());

        assert!(!claimed_path.exists());
        assert!(!failed_path.exists());
        let restored = std::fs::read_to_string(summary_spill_path())?;
        assert!(restored.contains("sess-active"));
        assert!(restored.contains("sess-restored"));
        assert!(!restored.contains("sess-duplicate"));
        Ok(())
    }

    #[tokio::test]
    async fn replay_error_does_not_drop_current_summary_payload() -> anyhow::Result<()> {
        let _test_dir = ScopedTestDataDir::new("summary-hook-replay-error-current");
        let conn = db::open_db()?;
        let now = chrono::Utc::now().timestamp();
        db::upsert_worker_heartbeat(
            &conn,
            "worker-daemon",
            i64::from(std::process::id()),
            now,
            now,
        )?;
        drop(conn);
        std::fs::create_dir_all(summary_spill_path())?;

        let current_input = serde_json::json!({
            "session_id": "sess-summary-current-after-replay-error",
            "cwd": "/tmp/remem"
        })
        .to_string();
        super::super::hook::summarize_input(&current_input, Some("codex-cli"), None).await?;

        let conn = db::open_db()?;
        let summary_jobs: i64 = conn.query_row(
            "SELECT COUNT(*) FROM jobs
             WHERE job_type = 'summary'
               AND session_id = 'sess-summary-current-after-replay-error'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(summary_jobs, 1);
        Ok(())
    }
}
