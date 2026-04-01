use anyhow::Result;

use crate::db;

use super::super::constants::SUMMARIZE_STDIN_TIMEOUT_MS;
use super::super::input::{read_stdin_with_timeout, SummarizeInput};

pub async fn summarize() -> Result<()> {
    let Some(input) = read_stdin_with_timeout(SUMMARIZE_STDIN_TIMEOUT_MS)? else {
        return Ok(());
    };

    let hook: SummarizeInput = match serde_json::from_str(&input) {
        Ok(value) => value,
        Err(err) => {
            crate::log::warn(
                "summarize",
                &format!("invalid hook payload, skipping: {}", err),
            );
            return Ok(());
        }
    };
    let Some(session_id) = &hook.session_id else {
        return Ok(());
    };
    let cwd = hook.cwd.as_deref().unwrap_or(".");
    let project = db::project_from_cwd(cwd);
    let conn = db::open_db()?;

    enqueue_summary_jobs(&conn, session_id, &project, &input)?;
    spawn_worker_once()?;
    Ok(())
}

fn enqueue_summary_jobs(
    conn: &rusqlite::Connection,
    session_id: &str,
    project: &str,
    input: &str,
) -> Result<()> {
    let obs_payload = serde_json::json!({
        "session_id": session_id,
        "project": project,
    });
    db::enqueue_job(
        conn,
        db::JobType::Observation,
        project,
        Some(session_id),
        &obs_payload.to_string(),
        50,
    )?;
    db::enqueue_job(
        conn,
        db::JobType::Summary,
        project,
        Some(session_id),
        input,
        100,
    )?;
    db::enqueue_job(conn, db::JobType::Compress, project, None, "{}", 200)?;
    crate::log::info(
        "summarize",
        &format!(
            "QUEUED observation+summary session={} project={}",
            session_id, project
        ),
    );
    Ok(())
}

fn spawn_worker_once() -> Result<()> {
    let exe = std::env::current_exe()?;
    let stderr_file = crate::log::open_log_append();
    let stderr_cfg = match stderr_file {
        Some(file) => std::process::Stdio::from(file),
        None => std::process::Stdio::null(),
    };
    let _child = std::process::Command::new(&exe)
        .arg("worker")
        .arg("--once")
        .env("REMEM_STDERR_TO_LOG", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(stderr_cfg)
        .spawn()?;
    Ok(())
}
