use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

use super::config::{
    log_lock_path, log_policy, log_rotation_issue_path, rotated_log_path, InvalidLogEnv, LogPolicy,
};

const ROTATION_ISSUE_FRESH_SECS: i64 = 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LogRotationIssue {
    pub kind: String,
    pub message: String,
    pub path: String,
    pub at_epoch: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct LogHealthSnapshot {
    pub path: PathBuf,
    pub active_bytes: u64,
    pub total_bytes: u64,
    pub max_bytes: u64,
    pub max_rotated_files: usize,
    pub lock_timeout_ms: u64,
    pub invalid_env: Vec<InvalidLogEnv>,
    pub issue: Option<LogRotationIssue>,
    pub issue_is_fresh: bool,
    pub issue_read_error: Option<String>,
}

pub(crate) fn log_health_snapshot() -> Option<LogHealthSnapshot> {
    let policy = log_policy()?;
    let active_bytes = file_size(&policy.path);
    let total_bytes = active_bytes + retained_log_bytes(&policy.path, policy.max_rotated_files);
    let (issue, issue_read_error) = match read_rotation_issue(&policy) {
        Ok(issue) => (issue, None),
        Err(error) => (None, Some(error)),
    };
    let issue_is_fresh = issue
        .as_ref()
        .is_some_and(|issue| issue.at_epoch >= now_epoch() - ROTATION_ISSUE_FRESH_SECS);
    Some(LogHealthSnapshot {
        path: policy.path,
        active_bytes,
        total_bytes,
        max_bytes: policy.max_bytes,
        max_rotated_files: policy.max_rotated_files,
        lock_timeout_ms: policy.lock_timeout_ms,
        invalid_env: policy.invalid_env,
        issue,
        issue_is_fresh,
        issue_read_error,
    })
}

pub(crate) fn rotate_if_needed(
    path: &Path,
    max_bytes: u64,
    max_rotated_files: usize,
) -> std::io::Result<()> {
    cleanup_suffixes_above(path, max_rotated_files)?;
    let size = match std::fs::metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(_) => 0,
    };
    if size < max_bytes {
        return Ok(());
    }

    if max_rotated_files == 0 {
        match std::fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        }
    }

    for index in (1..=max_rotated_files).rev() {
        let dst = rotated_log_path(path, index);
        if index == max_rotated_files {
            remove_if_exists(&dst)?;
        }
        let src = if index == 1 {
            path.to_path_buf()
        } else {
            rotated_log_path(path, index - 1)
        };
        if src.exists() {
            std::fs::rename(&src, &dst)?;
            set_private_permissions(&dst);
        }
    }
    Ok(())
}

fn write_log(level: &str, component: &str, msg: &str) {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let line = format!("[{}] [{}] [{}] {}", now, level, component, msg);
    if should_mirror_to_stderr(level, component) {
        eprintln!("{}", line);
    }
    let Some(policy) = log_policy() else {
        return;
    };
    if let Err(error) = write_line_locked(&policy, &line) {
        eprintln!("[remem] log write failed: {}", error);
    }
}

fn write_line_locked(policy: &LogPolicy, line: &str) -> std::io::Result<()> {
    match with_prepared_log(policy, |mut file| {
        writeln!(file, "{line}")?;
        Ok(None::<()>)
    }) {
        Ok(_) => Ok(()),
        Err(error) => Err(error),
    }
}

fn with_prepared_log<T>(
    policy: &LogPolicy,
    action: impl FnOnce(File) -> std::io::Result<Option<T>>,
) -> std::io::Result<Option<T>> {
    let prepare_started_epoch = now_epoch();
    create_parent_dir(policy)?;
    let lock_path = log_lock_path(&policy.path);
    let lock_file = match private_read_write_create_options().open(&lock_path) {
        Ok(file) => file,
        Err(error) => {
            record_rotation_issue(
                policy,
                "lock_open_failed",
                &format!("open log lock {} failed: {}", lock_path.display(), error),
            );
            return append_fallback(policy, action);
        }
    };
    set_private_permissions(&lock_path);
    match try_lock_until(&lock_file, Duration::from_millis(policy.lock_timeout_ms)) {
        Ok(true) => {}
        Ok(false) => {
            record_rotation_issue(
                policy,
                "lock_timeout",
                &format!(
                    "timed out after {}ms waiting for {}",
                    policy.lock_timeout_ms,
                    lock_path.display()
                ),
            );
            return append_fallback(policy, action);
        }
        Err(error) => {
            record_rotation_issue(
                policy,
                "lock_failed",
                &format!("lock {} failed: {}", lock_path.display(), error),
            );
            return append_fallback(policy, action);
        }
    }

    let rotate_result = rotate_if_needed(&policy.path, policy.max_bytes, policy.max_rotated_files);
    if let Err(error) = rotate_result {
        record_rotation_issue(
            policy,
            "rotate_failed",
            &format!("rotate {} failed: {}", policy.path.display(), error),
        );
        return append_fallback(policy, action);
    }

    match open_private_append(&policy.path) {
        Ok(file) => {
            let result = action(file);
            if result.is_ok() {
                clear_stale_rotation_issue(policy, prepare_started_epoch);
            }
            result
        }
        Err(error) => {
            record_rotation_issue(
                policy,
                "open_failed",
                &format!("open {} failed: {}", policy.path.display(), error),
            );
            append_fallback(policy, action)
        }
    }
}

fn should_mirror_to_stderr(level: &str, component: &str) -> bool {
    should_mirror_to_stderr_with_env(
        level,
        component,
        debug_enabled(),
        std::env::var_os("REMEM_STDERR_TO_LOG").is_some(),
    )
}

fn should_mirror_to_stderr_with_env(
    level: &str,
    component: &str,
    debug_enabled: bool,
    stderr_to_log: bool,
) -> bool {
    if stderr_to_log {
        return false;
    }
    if level == "INFO" && component == "migrate" {
        return debug_enabled;
    }
    true
}

pub fn open_log_append() -> Option<std::fs::File> {
    let policy = log_policy()?;
    match with_prepared_log(&policy, |file| Ok(Some(file))) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("[remem] open log for child stderr failed: {}", error);
            None
        }
    }
}

pub fn debug_enabled() -> bool {
    std::env::var("REMEM_DEBUG").is_ok()
}

pub fn debug(component: &str, msg: &str) {
    if debug_enabled() {
        write_log("DEBUG", component, msg);
    }
}

pub fn info(component: &str, msg: &str) {
    write_log("INFO", component, msg);
}

pub fn warn(component: &str, msg: &str) {
    write_log("WARN", component, msg);
}

pub fn error(component: &str, msg: &str) {
    write_log("ERROR", component, msg);
}

fn try_lock_until(file: &File, timeout: Duration) -> std::io::Result<bool> {
    let started = Instant::now();
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if started.elapsed() >= timeout {
                    return Ok(false);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => return Err(error),
        }
    }
}

fn append_fallback<T>(
    policy: &LogPolicy,
    action: impl FnOnce(File) -> std::io::Result<Option<T>>,
) -> std::io::Result<Option<T>> {
    create_parent_dir(policy)?;
    let file = open_private_append(&policy.path)?;
    action(file)
}

fn create_parent_dir(policy: &LogPolicy) -> std::io::Result<()> {
    if let Some(parent) = policy.path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn open_private_append(path: &Path) -> std::io::Result<File> {
    let file = private_append_create_options().open(path)?;
    set_private_permissions(path);
    Ok(file)
}

fn cleanup_suffixes_above(path: &Path, max_rotated_files: usize) -> std::io::Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    let Some(base_name) = path.file_name().and_then(|name| name.to_str()) else {
        return Ok(());
    };
    let prefix = format!("{base_name}.");
    for entry in std::fs::read_dir(parent)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(suffix) = name.strip_prefix(&prefix) else {
            continue;
        };
        let Ok(index) = suffix.parse::<usize>() else {
            continue;
        };
        if index > max_rotated_files {
            remove_if_exists(&entry.path())?;
        }
    }
    Ok(())
}

fn remove_if_exists(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn retained_log_bytes(path: &Path, max_rotated_files: usize) -> u64 {
    let configured_bytes = (1..=max_rotated_files)
        .map(|index| file_size(&rotated_log_path(path, index)))
        .sum::<u64>();
    configured_bytes + suffixes_above_bytes(path, max_rotated_files)
}

fn suffixes_above_bytes(path: &Path, max_rotated_files: usize) -> u64 {
    let Some(parent) = path.parent() else {
        return 0;
    };
    let Some(base_name) = path.file_name().and_then(|name| name.to_str()) else {
        return 0;
    };
    let prefix = format!("{base_name}.");
    let Ok(entries) = std::fs::read_dir(parent) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let suffix = name.strip_prefix(&prefix)?;
            let index = suffix.parse::<usize>().ok()?;
            (index > max_rotated_files).then(|| file_size(&entry.path()))
        })
        .sum()
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

fn record_rotation_issue(policy: &LogPolicy, kind: &str, message: &str) {
    let issue = LogRotationIssue {
        kind: kind.to_string(),
        message: message.to_string(),
        path: policy.path.display().to_string(),
        at_epoch: now_epoch(),
    };
    let path = log_rotation_issue_path(&policy.path);
    if let Some(parent) = path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            report_internal_io_error("create log rotation issue directory failed", &error);
        }
    }
    let tmp = path.with_file_name(format!(
        ".{}.{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("remem-log-issue"),
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let write_result = (|| -> std::io::Result<()> {
        let mut file = private_write_create_new_options().open(&tmp)?;
        let bytes = serde_json::to_vec_pretty(&issue).map_err(std::io::Error::other)?;
        file.write_all(&bytes)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        std::fs::rename(&tmp, &path)?;
        set_private_permissions(&path);
        Ok(())
    })();
    if let Err(error) = write_result {
        remove_temp_issue_file(&tmp);
        eprintln!("[remem] log rotation issue write failed: {}", error);
    }
}

fn read_rotation_issue(policy: &LogPolicy) -> Result<Option<LogRotationIssue>, String> {
    let path = log_rotation_issue_path(&policy.path);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!("read {} failed: {}", path.display(), error));
        }
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| format!("parse {} failed: {}", path.display(), error))
}

fn clear_stale_rotation_issue(policy: &LogPolicy, prepare_started_epoch: i64) {
    let path = log_rotation_issue_path(&policy.path);
    if let Ok(Some(issue)) = read_rotation_issue(policy) {
        if issue.at_epoch < prepare_started_epoch {
            remove_rotation_issue_file(&path);
        }
    }
}

fn now_epoch() -> i64 {
    chrono::Utc::now().timestamp()
}

fn private_append_create_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    set_create_mode(&mut options);
    options
}

fn private_read_write_create_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true).truncate(false);
    set_create_mode(&mut options);
    options
}

fn private_write_create_new_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    set_create_mode(&mut options);
    options
}

#[cfg(unix)]
fn set_create_mode(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
}

#[cfg(not(unix))]
fn set_create_mode(_options: &mut OpenOptions) {}

pub(crate) fn set_private_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(error) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
            report_internal_io_error("set private log permissions failed", &error);
        }
    }
}

fn remove_temp_issue_file(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            report_internal_io_error("remove temporary log rotation issue failed", &error)
        }
    }
}

fn remove_rotation_issue_file(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => report_internal_io_error("clear stale log rotation issue failed", &error),
    }
}

fn report_internal_io_error(context: &str, error: &std::io::Error) {
    eprintln!("[remem] {context}: {error}");
}

#[cfg(test)]
mod tests {
    #[test]
    fn migrate_info_is_not_mirrored_to_stderr_by_default() {
        assert!(!super::should_mirror_to_stderr_with_env(
            "INFO", "migrate", false, false
        ));
        assert!(super::should_mirror_to_stderr_with_env(
            "INFO", "install", false, false
        ));
        assert!(super::should_mirror_to_stderr_with_env(
            "ERROR", "migrate", false, false
        ));
    }

    #[test]
    fn migrate_info_is_mirrored_to_stderr_when_debug_enabled() {
        assert!(super::should_mirror_to_stderr_with_env(
            "INFO", "migrate", true, false
        ));
    }

    #[test]
    fn stderr_to_log_disables_stderr_mirroring() {
        assert!(!super::should_mirror_to_stderr_with_env(
            "INFO", "migrate", true, true
        ));
        assert!(!super::should_mirror_to_stderr_with_env(
            "ERROR", "migrate", true, true
        ));
    }
}
