//! Minimal Git CLI helpers used by capture identity. Pure parsers remain
//! separate from the shared bounded subprocess executor; recursive test
//! helpers exercise large-output, timeout, descendant, and cleanup paths.

use std::collections::HashSet;
use std::io::{self, Read};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};

const COMMIT_METADATA_FORMAT: &str = "--format=%H%x00%h%x00%at%x00%s";
pub(crate) const GIT_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const COMMAND_POLL_INTERVAL: Duration = Duration::from_millis(10);
const COMMAND_TERMINATE_GRACE: Duration = Duration::from_millis(100);
const COMMAND_CLEANUP_TIMEOUT: Duration = Duration::from_millis(500);

struct DrainWorker {
    label: &'static str,
    receiver: Receiver<io::Result<Vec<u8>>>,
    handle: Option<JoinHandle<()>>,
    bytes: Option<Vec<u8>>,
    error: Option<String>,
}

impl DrainWorker {
    fn spawn<R>(label: &'static str, mut reader: R) -> Self
    where
        R: Read + Send + 'static,
    {
        let (sender, receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = reader.read_to_end(&mut bytes).map(|_| bytes);
            let send_result = sender.send(result);
            drop(send_result);
        });
        Self {
            label,
            receiver,
            handle: Some(handle),
            bytes: None,
            error: None,
        }
    }

    fn await_until(&mut self, deadline: Instant) -> Result<()> {
        if self.bytes.is_some() {
            return Ok(());
        }
        if let Some(error) = &self.error {
            bail!("{error}");
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        let received = if remaining.is_zero() {
            self.receiver.try_recv().map_err(|error| match error {
                mpsc::TryRecvError::Empty => anyhow!("{} reader exceeded deadline", self.label),
                mpsc::TryRecvError::Disconnected => {
                    anyhow!("{} reader disconnected", self.label)
                }
            })?
        } else {
            self.receiver
                .recv_timeout(remaining)
                .map_err(|error| match error {
                    RecvTimeoutError::Timeout => anyhow!("{} reader exceeded deadline", self.label),
                    RecvTimeoutError::Disconnected => {
                        anyhow!("{} reader disconnected", self.label)
                    }
                })?
        };
        match received {
            Ok(bytes) => self.bytes = Some(bytes),
            Err(error) => {
                let message = format!("{} pipe read failed: {error}", self.label);
                self.error = Some(message.clone());
                bail!("{message}");
            }
        }
        let Some(handle) = self.handle.take() else {
            let message = format!("{} reader lost its join handle", self.label);
            self.error = Some(message.clone());
            bail!("{message}");
        };
        if handle.join().is_err() {
            let message = format!("{} reader thread panicked", self.label);
            self.error = Some(message.clone());
            bail!("{message}");
        }
        Ok(())
    }

    fn take_bytes(&mut self) -> Vec<u8> {
        self.bytes.take().unwrap_or_default()
    }
}

fn await_drains(
    stdout: &mut DrainWorker,
    stderr: &mut DrainWorker,
    deadline: Instant,
) -> Result<()> {
    stdout.await_until(deadline)?;
    stderr.await_until(deadline)?;
    Ok(())
}

fn signal_process_group(process_group_id: u32, signal: libc::c_int) -> Result<()> {
    #[cfg(unix)]
    {
        let result = unsafe { libc::kill(-(process_group_id as libc::pid_t), signal) };
        if result == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        Err(error).context("signal Git process group")
    }
    #[cfg(not(unix))]
    {
        let _ = process_group_id;
        let _ = signal;
        Ok(())
    }
}

fn reap_child_until(child: &mut Child, deadline: Instant) -> Result<()> {
    loop {
        match child.try_wait().context("poll Git child during cleanup")? {
            Some(_) => return Ok(()),
            None if Instant::now() < deadline => thread::sleep(COMMAND_POLL_INTERVAL),
            None => bail!("Git child reap exceeded cleanup deadline"),
        }
    }
}

fn cleanup_child(child: &mut Child, direct_reaped: bool, deadline: Instant) -> Vec<String> {
    let mut failures = Vec::new();
    #[cfg(unix)]
    {
        if let Err(error) = signal_process_group(child.id(), libc::SIGTERM) {
            failures.push(error.to_string());
        }
        let grace_deadline = (Instant::now() + COMMAND_TERMINATE_GRACE).min(deadline);
        if !direct_reaped {
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if Instant::now() < grace_deadline => {
                        thread::sleep(COMMAND_POLL_INTERVAL)
                    }
                    Ok(None) => break,
                    Err(error) => {
                        failures.push(format!("poll Git child after TERM failed: {error}"));
                        break;
                    }
                }
            }
        } else if Instant::now() < grace_deadline {
            thread::sleep(grace_deadline.saturating_duration_since(Instant::now()));
        }
        if let Err(error) = signal_process_group(child.id(), libc::SIGKILL) {
            failures.push(error.to_string());
        }
    }
    #[cfg(not(unix))]
    if !direct_reaped {
        if let Err(error) = child.kill() {
            failures.push(format!("kill Git child failed: {error}"));
        }
    }
    if !direct_reaped {
        if let Err(error) = reap_child_until(child, deadline) {
            failures.push(error.to_string());
        }
    }
    failures
}

fn lifecycle_error(
    primary: impl Into<String>,
    child: &mut Child,
    direct_reaped: bool,
    stdout: &mut DrainWorker,
    stderr: &mut DrainWorker,
) -> anyhow::Error {
    let cleanup_deadline = Instant::now() + COMMAND_CLEANUP_TIMEOUT;
    let mut failures = cleanup_child(child, direct_reaped, cleanup_deadline);
    if let Err(error) = stdout.await_until(cleanup_deadline) {
        failures.push(error.to_string());
    }
    if let Err(error) = stderr.await_until(cleanup_deadline) {
        failures.push(error.to_string());
    }
    let primary = primary.into();
    if failures.is_empty() {
        anyhow!(primary)
    } else {
        anyhow!("{primary}; cleanup: {}", failures.join("; "))
    }
}

pub(crate) fn command_output_with_timeout(command: Command, timeout: Duration) -> Result<Output> {
    command_output_with_timeout_inner(command, timeout, false)
}

fn command_output_with_timeout_inner(
    mut command: Command,
    timeout: Duration,
    inject_poll_error: bool,
) -> Result<Output> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn().context("spawn bounded command")?;
    let stdout = child
        .stdout
        .take()
        .context("bounded command omitted stdout pipe");
    let stderr = child
        .stderr
        .take()
        .context("bounded command omitted stderr pipe");
    let (stdout, stderr) = match (stdout, stderr) {
        (Ok(stdout), Ok(stderr)) => (stdout, stderr),
        (stdout, stderr) => {
            let cleanup_deadline = Instant::now() + COMMAND_CLEANUP_TIMEOUT;
            let failures = cleanup_child(&mut child, false, cleanup_deadline);
            let detail = stdout
                .err()
                .or_else(|| stderr.err())
                .map(|error| error.to_string())
                .unwrap_or_else(|| "missing output pipe".to_string());
            if failures.is_empty() {
                bail!("{detail}");
            }
            bail!("{detail}; cleanup: {}", failures.join("; "));
        }
    };
    let mut stdout_worker = DrainWorker::spawn("stdout", stdout);
    let mut stderr_worker = DrainWorker::spawn("stderr", stderr);
    let deadline = Instant::now() + timeout;
    let mut inject_poll_error = inject_poll_error;

    loop {
        let polled = if inject_poll_error {
            inject_poll_error = false;
            Err(io::Error::other("injected poll error"))
        } else {
            child.try_wait()
        };
        match polled {
            Ok(Some(status)) => {
                if let Err(error) = await_drains(&mut stdout_worker, &mut stderr_worker, deadline) {
                    return Err(lifecycle_error(
                        format!("collect bounded command output: {error}"),
                        &mut child,
                        true,
                        &mut stdout_worker,
                        &mut stderr_worker,
                    ));
                }
                return Ok(Output {
                    status,
                    stdout: stdout_worker.take_bytes(),
                    stderr: stderr_worker.take_bytes(),
                });
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(COMMAND_POLL_INTERVAL),
            Ok(None) => {
                return Err(lifecycle_error(
                    format!("bounded command timed out after {} ms", timeout.as_millis()),
                    &mut child,
                    false,
                    &mut stdout_worker,
                    &mut stderr_worker,
                ));
            }
            Err(error) => {
                return Err(lifecycle_error(
                    format!("poll bounded command failed: {error}"),
                    &mut child,
                    false,
                    &mut stdout_worker,
                    &mut stderr_worker,
                ));
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitEvidenceKind {
    ObservedCommit,
    TerminalSnapshot,
}

impl GitEvidenceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ObservedCommit => "observed_commit",
            Self::TerminalSnapshot => "terminal_snapshot",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GitCommitMetadata {
    pub repo_path: String,
    pub sha: String,
    pub short_sha: String,
    pub branch: Option<String>,
    pub message: Option<String>,
    pub authored_at_epoch: Option<i64>,
    pub changed_files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GitCommitEvidence {
    pub kind: GitEvidenceKind,
    pub metadata: GitCommitMetadata,
    pub locator: Option<String>,
}

impl GitCommitMetadata {
    pub fn matches_sha(&self, sha: &str) -> bool {
        let needle = sha.trim();
        !needle.is_empty()
            && (self.sha == needle
                || self.short_sha == needle
                || self.sha.starts_with(needle)
                || self.short_sha.starts_with(needle))
    }
}

/// Parse the output of `git rev-parse --show-toplevel`. Returns `None` for
/// empty or whitespace-only input so callers can fall back cleanly.
pub fn parse_toplevel_output(stdout: &str) -> Option<PathBuf> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

/// Spawn `git rev-parse --show-toplevel` in `cwd`. Returns `None` when git
/// is unavailable, the directory is outside any git worktree, or the output
/// is empty. Single subprocess, no network.
pub fn resolve_toplevel(cwd: &Path) -> Option<PathBuf> {
    let output = git_output_soft(cwd, &["rev-parse", "--show-toplevel"])?;
    if !output.status.success() {
        return None;
    }
    parse_toplevel_output(&String::from_utf8_lossy(&output.stdout))
}

fn resolve_toplevel_required(cwd: &Path) -> Result<PathBuf> {
    let stdout = git_stdout_required_path(cwd, &["rev-parse", "--show-toplevel"])?;
    parse_toplevel_output(&stdout)
        .with_context(|| format!("git returned an empty repository root in {}", cwd.display()))
}

pub(crate) fn git_output_soft(cwd: &Path, args: &[&str]) -> Option<Output> {
    let mut command = Command::new("git");
    command.args(args).current_dir(cwd);
    match command_output_with_timeout(command, GIT_PROBE_TIMEOUT) {
        Ok(output) => Some(output),
        Err(error) => {
            crate::log::error(
                "git",
                &format!(
                    "git {} lifecycle failure in {}: {error:#}",
                    args.join(" "),
                    cwd.display()
                ),
            );
            None
        }
    }
}

pub fn parse_changed_files_output(stdout: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    for file in stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if seen.insert(file.to_string()) {
            files.push(file.to_string());
        }
    }
    files
}

pub fn parse_authored_epoch_output(stdout: &str) -> Option<i64> {
    stdout.trim().parse::<i64>().ok()
}

pub fn parse_branch_output(stdout: &str) -> Option<String> {
    let branch = stdout.trim();
    if branch.is_empty() || branch == "HEAD" {
        None
    } else {
        Some(branch.to_string())
    }
}

pub fn short_sha_for(sha: &str) -> String {
    sha.chars().take(7).collect()
}

pub fn detect_commit_metadata(cwd: &str) -> Result<Option<GitCommitMetadata>> {
    let Some(_) = resolve_toplevel(Path::new(cwd)) else {
        return Ok(None);
    };
    let sha = git_stdout_required(cwd, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    let sha = sha.trim().to_ascii_lowercase();
    if sha.is_empty() {
        bail!("git returned an empty HEAD commit in {cwd}");
    }

    let metadata = resolve_commit_metadata(cwd, &sha)?;
    let final_sha = git_stdout_required(cwd, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    if !final_sha.trim().eq_ignore_ascii_case(&sha) {
        bail!("git HEAD changed while capturing commit metadata in {cwd}");
    }
    Ok(Some(metadata))
}

pub fn resolve_commit_metadata(cwd: &str, commitish: &str) -> Result<GitCommitMetadata> {
    let repo_path = resolve_toplevel_required(Path::new(cwd))
        .with_context(|| format!("resolve required Git repository in {cwd}"))?;
    let commit_ref = format!("{}^{{commit}}", commitish.trim());
    let sha = git_stdout_required(cwd, &["rev-parse", "--verify", &commit_ref])?;
    let sha = sha.trim().to_ascii_lowercase();
    if sha.is_empty() {
        bail!("git returned an empty commit for evidence {commitish}");
    }
    let raw_metadata = git_stdout_required(cwd, &["show", "-s", COMMIT_METADATA_FORMAT, &sha])?;
    let mut fields = raw_metadata.trim_end_matches(['\r', '\n']).splitn(4, '\0');
    let resolved_sha = fields
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let short_sha = fields
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let authored_at_epoch = fields
        .next()
        .and_then(parse_authored_epoch_output)
        .context("git commit metadata omitted authored epoch")?;
    let message = fields
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if resolved_sha != sha || short_sha.is_empty() || !sha.starts_with(&short_sha) {
        bail!("git returned inconsistent metadata for captured commit {sha}");
    }

    let current_head = git_stdout_required(cwd, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    let branch = if current_head.trim().eq_ignore_ascii_case(&sha) {
        parse_branch_output(&git_stdout_required(cwd, &["branch", "--show-current"])?)
    } else {
        None
    };
    let changed_files = parse_changed_files_output(&git_stdout_required(
        cwd,
        &[
            "diff-tree",
            "--root",
            "-m",
            "--no-commit-id",
            "--name-only",
            "-r",
            &sha,
        ],
    )?);
    Ok(sanitize_commit_metadata(GitCommitMetadata {
        repo_path: repo_path.to_string_lossy().to_string(),
        sha,
        short_sha,
        branch,
        message,
        authored_at_epoch: Some(authored_at_epoch),
        changed_files,
    }))
}

pub fn sanitize_commit_metadata(mut metadata: GitCommitMetadata) -> GitCommitMetadata {
    metadata.branch = metadata
        .branch
        .map(|value| crate::adapter::common::redact_sensitive_text(&value));
    metadata.message = metadata
        .message
        .map(|value| crate::adapter::common::redact_sensitive_text(&value));
    metadata.changed_files = metadata
        .changed_files
        .into_iter()
        .map(|value| crate::adapter::common::redact_sensitive_text(&value))
        .collect();
    metadata
}

fn git_stdout_required(cwd: &str, args: &[&str]) -> Result<String> {
    git_stdout_required_path(Path::new(cwd), args)
}

fn git_stdout_required_path(cwd: &Path, args: &[&str]) -> Result<String> {
    let mut command = Command::new("git");
    command.args(args).current_dir(cwd);
    let output = command_output_with_timeout(command, GIT_PROBE_TIMEOUT)
        .with_context(|| format!("run required git {} in {}", args.join(" "), cwd.display()))?;
    if !output.status.success() {
        let stderr_text = String::from_utf8_lossy(&output.stderr);
        let stderr = crate::db::truncate_str(&stderr_text, 400);
        bail!(
            "git {} failed in {} with status {}: {stderr}",
            args.join(" "),
            cwd.display(),
            output.status
        );
    }
    String::from_utf8(output.stdout).with_context(|| {
        format!(
            "git {} returned non-UTF-8 output in {}",
            args.join(" "),
            cwd.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_PID_FILE: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn parsers_preserve_git_identity_contracts() {
        assert_eq!(
            parse_toplevel_output("/Users/foo/project\n"),
            Some(PathBuf::from("/Users/foo/project"))
        );
        assert_eq!(parse_toplevel_output(""), None);
        assert_eq!(parse_toplevel_output("   \t\n  "), None);
        assert_eq!(
            parse_changed_files_output("src/lib.rs\n\nREADME.md\nsrc/lib.rs\n"),
            vec!["src/lib.rs".to_string(), "README.md".to_string()]
        );
        assert_eq!(
            parse_authored_epoch_output("1700000000\n"),
            Some(1700000000)
        );
        assert_eq!(parse_authored_epoch_output("not-an-epoch"), None);
        assert_eq!(
            parse_branch_output("feature/x\n"),
            Some("feature/x".to_string())
        );
        assert_eq!(parse_branch_output("HEAD\n"), None);
        assert_eq!(parse_branch_output("\n"), None);
    }

    #[test]
    fn commit_metadata_matches_only_non_empty_sha_prefixes() {
        let metadata = GitCommitMetadata {
            repo_path: "/repo".to_string(),
            sha: "abcdef1234567890".to_string(),
            short_sha: "abcdef1".to_string(),
            branch: Some("main".to_string()),
            message: None,
            authored_at_epoch: None,
            changed_files: Vec::new(),
        };

        assert!(metadata.matches_sha("abcdef1234567890"));
        assert!(metadata.matches_sha("abcdef1"));
        assert!(metadata.matches_sha("abcdef"));
        assert!(!metadata.matches_sha(""));
        assert!(!metadata.matches_sha("fedcba"));
    }

    fn helper_command(mode: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().expect("test executable"));
        command
            .args([
                "--exact",
                "git_util::tests::bounded_command_test_helper",
                "--nocapture",
            ])
            .env("REMEM_BOUNDED_COMMAND_HELPER", mode);
        command
    }

    #[test]
    fn bounded_command_test_helper() {
        let Ok(mode) = std::env::var("REMEM_BOUNDED_COMMAND_HELPER") else {
            return;
        };
        match mode.as_str() {
            "sleep" => thread::sleep(Duration::from_secs(5)),
            "large" => {
                let bytes = vec![b'x'; 256 * 1024];
                std::io::stdout().write_all(&bytes).expect("write stdout");
                std::io::stderr().write_all(&bytes).expect("write stderr");
            }
            "descendant" => {
                let child = helper_command("sleep")
                    .stdout(Stdio::inherit())
                    .stderr(Stdio::inherit())
                    .spawn()
                    .expect("spawn descendant");
                if let Ok(path) = std::env::var("REMEM_DESCENDANT_PID_FILE") {
                    std::fs::write(path, child.id().to_string()).expect("write descendant pid");
                }
                std::mem::forget(child);
            }
            other => panic!("unknown helper mode {other}"),
        }
    }

    fn assert_descendant_cleanup() -> anyhow::Error {
        let pid_file = std::env::temp_dir().join(format!(
            "remem-git-util-descendant-{}-{}",
            std::process::id(),
            NEXT_PID_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        if pid_file.exists() {
            std::fs::remove_file(&pid_file).expect("remove stale pid file");
        }
        let mut command = helper_command("descendant");
        command.env("REMEM_DESCENDANT_PID_FILE", &pid_file);
        let started = Instant::now();
        let error = command_output_with_timeout(command, Duration::from_millis(150))
            .expect_err("inherited pipes must exceed the command deadline");
        assert!(started.elapsed() < Duration::from_secs(2));
        let pid: libc::pid_t = std::fs::read_to_string(&pid_file)
            .expect("read descendant pid")
            .parse()
            .expect("parse descendant pid");
        std::fs::remove_file(pid_file).expect("remove pid file");
        #[cfg(unix)]
        for _ in 0..50 {
            if unsafe { libc::kill(pid, 0) } != 0 {
                return error;
            }
            thread::sleep(Duration::from_millis(10));
        }
        #[cfg(unix)]
        panic!("descendant process {pid} survived process-group cleanup");
        #[cfg(not(unix))]
        error
    }

    #[cfg(unix)]
    #[test]
    fn command_output_with_timeout_kills_process_group() {
        let error = assert_descendant_cleanup();
        assert!(error.to_string().contains("reader exceeded deadline"));
    }

    #[test]
    fn command_output_with_timeout_bounds_reader_completion() {
        let error = assert_descendant_cleanup();
        assert!(!error.to_string().contains("cleanup:"));
    }

    #[test]
    fn command_output_with_timeout_cleans_up_after_poll_error() {
        let started = Instant::now();
        let error = command_output_with_timeout_inner(
            helper_command("sleep"),
            Duration::from_secs(2),
            true,
        )
        .expect_err("injected poll error must fail");
        assert!(error.to_string().contains("injected poll error"));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn command_output_with_timeout_drains_large_output() {
        let output = command_output_with_timeout(helper_command("large"), Duration::from_secs(2))
            .expect("large output must drain concurrently");
        assert!(output.status.success());
        assert!(output.stdout.len() >= 256 * 1024);
        assert!(output.stderr.len() >= 256 * 1024);
    }

    #[test]
    fn required_toplevel_preserves_timeout_context() {
        let missing = Path::new("/remem-gh864-definitely-missing");
        let error = resolve_toplevel_required(missing).expect_err("missing cwd must fail");
        let message = format!("{error:#}");
        assert!(message.contains("rev-parse --show-toplevel"));
        assert!(message.contains(missing.to_string_lossy().as_ref()));
    }

    #[test]
    fn git_metadata_commands_use_bounded_executor() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
        let metadata = resolve_commit_metadata(repo.to_str().expect("UTF-8 repo"), "HEAD")
            .expect("repository HEAD metadata");
        assert!(!metadata.sha.is_empty());
        assert_eq!(GIT_PROBE_TIMEOUT, Duration::from_secs(2));
    }
}
