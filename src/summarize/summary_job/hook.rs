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
    let mut command = std::process::Command::new(&exe);
    command
        .arg("worker")
        .arg("--once")
        .env("REMEM_STDERR_TO_LOG", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(stderr_cfg);
    configure_worker_executor_env(&mut command);
    let _child = command.spawn()?;
    Ok(())
}

fn configure_worker_executor_env(command: &mut std::process::Command) {
    if std::env::var_os("REMEM_SUMMARY_EXECUTOR").is_none() {
        if let Some(executor) = std::env::var_os("REMEM_EXECUTOR") {
            command.env("REMEM_SUMMARY_EXECUTOR", executor);
        }
    }
    command.env_remove("REMEM_EXECUTOR");
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::sync::Mutex;

    use super::configure_worker_executor_env;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_env_vars<T>(vars: &[(&str, Option<&str>)], f: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().expect("env lock should acquire");
        let old_values = vars
            .iter()
            .map(|(key, _)| ((*key).to_string(), std::env::var(key).ok()))
            .collect::<Vec<_>>();

        for (key, value) in vars {
            match value {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            }
        }

        let result = f();

        for (key, value) in old_values {
            match value {
                Some(value) => unsafe { std::env::set_var(&key, value) },
                None => unsafe { std::env::remove_var(&key) },
            }
        }

        result
    }

    fn command_env<'a>(command: &'a std::process::Command, key: &str) -> Option<Option<&'a OsStr>> {
        command
            .get_envs()
            .find(|(name, _)| *name == OsStr::new(key))
            .map(|(_, value)| value)
    }

    #[test]
    fn worker_env_translates_legacy_global_executor_to_summary_only() {
        with_env_vars(
            &[
                ("REMEM_EXECUTOR", Some("codex-cli")),
                ("REMEM_SUMMARY_EXECUTOR", None),
            ],
            || {
                let mut command = std::process::Command::new("remem");
                configure_worker_executor_env(&mut command);

                assert_eq!(
                    command_env(&command, "REMEM_SUMMARY_EXECUTOR"),
                    Some(Some(OsStr::new("codex-cli")))
                );
                assert_eq!(command_env(&command, "REMEM_EXECUTOR"), Some(None));
            },
        );
    }
}
