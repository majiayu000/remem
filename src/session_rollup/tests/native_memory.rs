use anyhow::Result;
use rusqlite::Connection;

use super::side_effects::{custom_capture, job_types, EnvVarGuard};
use super::*;

fn user_context_followup_count(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM extraction_tasks
         WHERE task_kind = 'user_context_candidate'",
        [],
        |row| row.get(0),
    )?)
}

#[tokio::test]
async fn native_memory_write_failure_does_not_block_durable_rollup_followups() -> Result<()> {
    let data_dir = crate::db::test_support::ScopedTestDataDir::new("rollup-native-sync-failure");
    let home = data_dir.path.join("home");
    let cwd_path = data_dir.path.join("project");
    std::fs::create_dir_all(&cwd_path)?;
    let cwd = std::fs::canonicalize(&cwd_path)?
        .to_string_lossy()
        .to_string();
    let memory_dir = home
        .join(".claude/projects")
        .join(cwd.replace('/', "-"))
        .join("memory");
    std::fs::create_dir_all(memory_dir.join(crate::context::claude_memory::REMEM_FILE))?;
    let _home = EnvVarGuard::set_path("HOME", &home);
    let _native_sync =
        EnvVarGuard::remove(crate::context::claude_memory::DISABLE_NATIVE_MEMORY_SYNC_ENV);

    let mut conn = crate::db::open_db()?;
    custom_capture(
        &conn,
        "sess-rollup-native-sync-failure",
        &cwd,
        Some(&cwd),
        &serde_json::json!({"session_id":"sess-rollup-native-sync-failure","cwd":cwd}).to_string(),
    )?;
    let task = claim_rollup_task(&mut conn)?;
    let result = process_with_summarizer(&mut conn, &task, |_prompt| async {
        Ok(xml_response_with_structured_fields(
            "Persisted rollup survives optional native-memory failure.",
            "Keep durable follow-ups",
            "Native memory is an optional mirror.",
            "",
            "Run every durable follow-up.",
            "",
            "",
        ))
    })
    .await?;
    assert_eq!(result, SessionRollupResult::Written);
    assert_eq!(summary_count(&conn), 1);
    assert_eq!(job_types(&conn)?, ["compress", "dream"]);
    assert_eq!(user_context_followup_count(&conn)?, 1);

    let retry = process_with_summarizer(&mut conn, &task, |_prompt| async {
        anyhow::bail!("persisted retry must not call the summarizer")
    })
    .await?;
    assert_eq!(retry, SessionRollupResult::AlreadyExists);
    assert_eq!(job_types(&conn)?, ["compress", "dream"]);
    assert_eq!(user_context_followup_count(&conn)?, 1);

    let log = std::fs::read_to_string(data_dir.path.join("remem.log"))?;
    assert!(
        log.contains("[ERROR]")
            && log.contains("native memory sync failed")
            && log.contains("session_row_id=1")
            && log.contains("event_range=1..1"),
        "native-memory failure must remain error-visible: {log}"
    );
    Ok(())
}
