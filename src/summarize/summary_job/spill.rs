use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

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
    db_error: &anyhow::Error,
) -> Result<PathBuf> {
    let path = summary_spill_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create summary hook spill dir {}", parent.display()))?;
    }
    let record = SummaryHookSpillRecord {
        version: 1,
        input: input.to_string(),
        host: host.map(crate::runtime_config::normalize_host),
        profile: profile.map(str::to_string),
        db_error: crate::db::truncate_str(
            &crate::db::capture::redact_capture_content(&db_error.to_string()),
            1000,
        )
        .to_string(),
        created_at_epoch: chrono::Utc::now().timestamp(),
    };
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open summary hook spill {}", path.display()))?;
    serde_json::to_writer(&mut file, &record)?;
    file.write_all(b"\n")?;
    Ok(path)
}

pub(super) fn replay_spilled_summary_hook_payloads(
    conn: &Connection,
    mut replay: impl FnMut(&Connection, &SummaryHookSpillRecord) -> Result<()>,
) -> Result<usize> {
    let path = summary_spill_path();
    if !path.exists() {
        return Ok(0);
    }

    let contents =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let failed_path = failed_summary_spill_path();
    if failed_path.exists() {
        std::fs::remove_file(&failed_path)
            .with_context(|| format!("remove stale {}", failed_path.display()))?;
    }

    let mut replayed = 0;
    for line in contents.lines().filter(|line| !line.trim().is_empty()) {
        match serde_json::from_str::<SummaryHookSpillRecord>(line) {
            Ok(record) => match replay(conn, &record) {
                Ok(()) => replayed += 1,
                Err(error) => append_failed_record(&failed_path, &record, &error)?,
            },
            Err(error) => append_failed_line(&failed_path, line, &error.into())?,
        }
    }

    std::fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
    if failed_path.exists() {
        std::fs::rename(&failed_path, &path).with_context(|| {
            format!(
                "move unreplayed summary hook spill {} to {}",
                failed_path.display(),
                path.display()
            )
        })?;
    }

    if replayed > 0 {
        crate::log::info(
            "summarize",
            &format!("replayed {replayed} spilled summary hook payload(s)"),
        );
    }
    Ok(replayed)
}

fn append_failed_line(path: &Path, line: &str, error: &anyhow::Error) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!("create failed summary hook spill dir {}", parent.display())
        })?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open failed summary hook spill {}", path.display()))?;
    writeln!(file, "{line}")?;
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
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!("create failed summary hook spill dir {}", parent.display())
        })?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open failed summary hook spill {}", path.display()))?;
    serde_json::to_writer(&mut file, record)?;
    file.write_all(b"\n")?;
    crate::log::warn(
        "summarize",
        &format!("summary hook spill replay failed: {error}"),
    );
    Ok(())
}

pub(super) fn summary_spill_path() -> PathBuf {
    crate::db::data_dir().join("summary-hook-spill.jsonl")
}

fn failed_summary_spill_path() -> PathBuf {
    crate::db::data_dir().join("summary-hook-spill.failed.jsonl")
}

#[cfg(test)]
mod tests {
    use crate::db::{self, test_support::ScopedTestDataDir};

    use super::summary_spill_path;

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
}
