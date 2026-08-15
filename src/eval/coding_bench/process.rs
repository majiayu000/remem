use std::ffi::OsStr;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

#[derive(Debug)]
pub(super) struct CommandOutcome {
    pub(super) stdout: String,
    pub(super) stderr: String,
    pub(super) exit_code: Option<i32>,
    pub(super) timed_out: bool,
}

pub(super) fn command_output<I, S>(
    program: &str,
    args: I,
    cwd: &Path,
    env: &[(String, String)],
    timeout_ms: u64,
) -> Result<CommandOutcome>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("REMEM_DATA_DIR")
        .env_remove("REMEM_CIPHER_KEY")
        .env_remove("REMEM_DISABLE_HOOKS")
        .env_remove("REMEM_ALLOW_PLAINTEXT_DB")
        .env_remove("CODEX_HOME")
        .env_remove("CODEX_THREAD_ID")
        .env_remove("VIRTUAL_ENV")
        .env_remove("PYTHONHOME")
        .env_remove("PYTHONPATH")
        .env_remove("CONDA_PREFIX")
        .env_remove("CONDA_DEFAULT_ENV")
        .env_remove("PIPENV_ACTIVE")
        .env_remove("POETRY_ACTIVE")
        .env_remove("PYENV_VERSION");
    for (key, value) in env {
        command.env(key, value);
    }
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .spawn()
        .with_context(|| format!("spawn command {program}"))?;
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut timed_out = false;
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            terminate_command(&mut child);
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let output = child
        .wait_with_output()
        .with_context(|| format!("wait for command {program}"))?;
    Ok(CommandOutcome {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        exit_code: output.status.code(),
        timed_out,
    })
}

pub(super) fn ensure_success(label: &str, outcome: &CommandOutcome) -> Result<()> {
    if outcome.exit_code == Some(0) && !outcome.timed_out {
        return Ok(());
    }
    bail!(
        "{label} failed with exit={:?} timed_out={} stderr={}",
        outcome.exit_code,
        outcome.timed_out,
        outcome.stderr
    )
}

fn terminate_command(child: &mut Child) {
    #[cfg(unix)]
    {
        let process_group = -(child.id() as libc::pid_t);
        terminate_unix_process_group("TERM", libc::SIGTERM, process_group);
        std::thread::sleep(Duration::from_millis(200));
        match child.try_wait() {
            Ok(Some(_)) => {}
            Ok(None) => terminate_unix_process_group("KILL", libc::SIGKILL, process_group),
            Err(error) => eprintln!(
                "[coding-bench] failed to poll timed-out runner process {}: {error}",
                child.id()
            ),
        }
    }
    #[cfg(not(unix))]
    if let Err(error) = child.kill() {
        eprintln!(
            "[coding-bench] failed to kill timed-out runner process {}: {error}",
            child.id()
        );
    }
}

#[cfg(unix)]
fn terminate_unix_process_group(
    signal_name: &str,
    signal: libc::c_int,
    process_group: libc::pid_t,
) {
    if unsafe { libc::kill(process_group, signal) } != 0 {
        let error = std::io::Error::last_os_error();
        eprintln!(
            "[coding-bench] failed to send {signal_name} to process group {process_group}: {error}"
        );
    }
}
