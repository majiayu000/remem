use std::ffi::OsString;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;

use fs2::FileExt;

use super::config::{
    log_lock_path, log_max_bytes, log_path, log_policy, log_rotation_issue_path, rotated_log_path,
    with_log_dir, DEFAULT_LOG_LOCK_TIMEOUT_MS, DEFAULT_LOG_MAX_BYTES,
    DEFAULT_LOG_MAX_ROTATED_FILES,
};
use super::write::{rotate_if_needed, LogRotationIssue};
use super::{info, open_log_append};
use crate::db::test_support::ScopedTestDataDir;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn with_log_envs<T>(vars: &[(&'static str, Option<&str>)], f: impl FnOnce() -> T) -> T {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let _env = ScopedLogEnv::set(vars);
    f()
}

fn with_log_env<T>(value: Option<&str>, f: impl FnOnce() -> T) -> T {
    with_log_envs(&[("REMEM_LOG_MAX_BYTES", value)], f)
}

#[test]
fn log_max_bytes_uses_positive_env_override() {
    with_log_env(Some("4096"), || {
        assert_eq!(log_max_bytes(), 4096);
    });
}

#[test]
fn log_max_bytes_rejects_zero_and_invalid() {
    with_log_env(Some("0"), || {
        assert_eq!(log_max_bytes(), DEFAULT_LOG_MAX_BYTES);
    });
    with_log_env(Some("invalid"), || {
        assert_eq!(log_max_bytes(), DEFAULT_LOG_MAX_BYTES);
    });
}

#[test]
fn log_policy_parses_rotation_env_and_collects_invalid_values() {
    let _data_dir = ScopedTestDataDir::new("log-policy");

    with_log_envs(
        &[
            ("REMEM_LOG_MAX_BYTES", Some("4096")),
            ("REMEM_LOG_MAX_ROTATED_FILES", Some("0")),
            ("REMEM_LOG_LOCK_TIMEOUT_MS", Some("50")),
        ],
        || {
            let policy = log_policy().expect("log policy should resolve");
            assert_eq!(policy.max_bytes, 4096);
            assert_eq!(policy.max_rotated_files, 0);
            assert_eq!(policy.lock_timeout_ms, 50);
            assert!(policy.invalid_env.is_empty());
        },
    );

    with_log_envs(
        &[
            ("REMEM_LOG_MAX_BYTES", Some("0")),
            ("REMEM_LOG_MAX_ROTATED_FILES", Some("invalid")),
            ("REMEM_LOG_LOCK_TIMEOUT_MS", Some("0")),
        ],
        || {
            let policy = log_policy().expect("log policy should resolve");
            assert_eq!(policy.max_bytes, DEFAULT_LOG_MAX_BYTES);
            assert_eq!(policy.max_rotated_files, DEFAULT_LOG_MAX_ROTATED_FILES);
            assert_eq!(policy.lock_timeout_ms, DEFAULT_LOG_LOCK_TIMEOUT_MS);
            let names = policy
                .invalid_env
                .iter()
                .map(|item| item.name)
                .collect::<Vec<_>>();
            assert_eq!(
                names,
                vec![
                    "REMEM_LOG_MAX_BYTES",
                    "REMEM_LOG_MAX_ROTATED_FILES",
                    "REMEM_LOG_LOCK_TIMEOUT_MS"
                ]
            );
        },
    );
}

#[test]
fn open_log_append_creates_log_file_in_data_dir() {
    let _data_dir = ScopedTestDataDir::new("log-open-append");

    let file = open_log_append().expect("log file should open");
    drop(file);

    let path = log_path().expect("log path should resolve");
    assert!(path.exists(), "log file should exist at {:?}", path);
}

#[test]
fn open_log_append_rotates_before_returning_handle() {
    let dir = unique_temp_dir("log-open-append-rotates");
    std::fs::create_dir_all(&dir).expect("log dir should create");

    with_log_envs(&[("REMEM_LOG_MAX_BYTES", Some("4"))], || {
        with_log_dir(&dir, || {
            let path = dir.join("remem.log");
            std::fs::write(&path, "oversized").expect("oversized log should write");

            let mut file = open_log_append().expect("log file should open");
            writeln!(file, "child-stderr").expect("child stderr line should write");
            drop(file);

            assert_eq!(
                std::fs::read_to_string(&path).expect("active log should read"),
                "child-stderr\n"
            );
            assert_eq!(
                std::fs::read_to_string(rotated_log_path(&path, 1))
                    .expect("rotated log should read"),
                "oversized"
            );
        });
    });
    std::fs::remove_dir_all(dir).expect("log dir should remove");
}

#[test]
fn with_log_dir_overrides_log_path_for_current_thread() {
    let dir = std::env::temp_dir().join(format!(
        "remem-log-override-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("log override dir should create");

    let path = with_log_dir(&dir, || log_path().expect("log path should resolve"));

    assert_eq!(path, dir.join("remem.log"));
    std::fs::remove_dir_all(dir).expect("log override dir should remove");
}

#[test]
fn rotate_if_needed_shifts_existing_files() {
    let data_dir = ScopedTestDataDir::new("log-rotate");
    // Use a dedicated test path — NOT the real log path — so concurrent
    // tests' log writes (e.g. migration auto-upgrade) cannot contaminate
    // the file we are about to rotate.
    let path = data_dir.path.join("logs").join("rotate-test.log");
    let parent = path.parent().expect("log file should have parent");
    std::fs::create_dir_all(parent).expect("log dir should create");

    std::fs::write(&path, "base-payload").expect("base log should write");
    std::fs::write(format!("{}.1", path.display()), "older-1").expect("log.1 should write");
    std::fs::write(format!("{}.2", path.display()), "older-2").expect("log.2 should write");
    std::fs::write(format!("{}.3", path.display()), "older-3").expect("log.3 should write");

    rotate_if_needed(&path, 4, DEFAULT_LOG_MAX_ROTATED_FILES).expect("rotation should succeed");

    assert!(
        !path.exists(),
        "base log should be renamed away during rotation"
    );
    assert_eq!(
        std::fs::read_to_string(format!("{}.1", path.display())).expect("log.1 should read"),
        "base-payload"
    );
    assert_eq!(
        std::fs::read_to_string(format!("{}.2", path.display())).expect("log.2 should read"),
        "older-1"
    );
    assert_eq!(
        std::fs::read_to_string(format!("{}.3", path.display())).expect("log.3 should read"),
        "older-2"
    );
}

#[test]
fn rotate_if_needed_honors_configured_retention() {
    let data_dir = ScopedTestDataDir::new("log-rotate-retention");
    let path = data_dir.path.join("logs").join("rotate-retention.log");
    std::fs::create_dir_all(path.parent().expect("log file should have parent"))
        .expect("log dir should create");

    std::fs::write(&path, "active").expect("active log should write");
    for index in 1..=6 {
        std::fs::write(rotated_log_path(&path, index), format!("older-{index}"))
            .expect("rotated log should write");
    }

    rotate_if_needed(&path, 1, 5).expect("rotation should succeed");

    assert_eq!(
        std::fs::read_to_string(rotated_log_path(&path, 1)).expect("log.1 should read"),
        "active"
    );
    assert_eq!(
        std::fs::read_to_string(rotated_log_path(&path, 5)).expect("log.5 should read"),
        "older-4"
    );
    assert!(
        !rotated_log_path(&path, 6).exists(),
        "suffix above configured retention should be removed"
    );
}

#[test]
fn rotate_if_needed_reduced_retention_cleans_stale_suffixes_before_size_check() {
    let data_dir = ScopedTestDataDir::new("log-rotate-reduced-retention");
    let path = data_dir.path.join("logs").join("rotate-reduced.log");
    std::fs::create_dir_all(path.parent().expect("log file should have parent"))
        .expect("log dir should create");

    std::fs::write(&path, "tiny").expect("active log should write");
    std::fs::write(rotated_log_path(&path, 4), "stale-4").expect("log.4 should write");
    std::fs::write(rotated_log_path(&path, 5), "stale-5").expect("log.5 should write");

    rotate_if_needed(&path, 9999, 3).expect("rotation should succeed");

    assert_eq!(
        std::fs::read_to_string(&path).expect("active log should read"),
        "tiny"
    );
    assert!(!rotated_log_path(&path, 4).exists());
    assert!(!rotated_log_path(&path, 5).exists());
}

#[test]
fn rotate_if_needed_zero_retention_removes_active_and_suffixes() {
    let data_dir = ScopedTestDataDir::new("log-rotate-zero-retention");
    let path = data_dir.path.join("logs").join("rotate-zero.log");
    std::fs::create_dir_all(path.parent().expect("log file should have parent"))
        .expect("log dir should create");

    std::fs::write(&path, "active").expect("active log should write");
    std::fs::write(rotated_log_path(&path, 1), "older-1").expect("log.1 should write");
    std::fs::write(rotated_log_path(&path, 2), "older-2").expect("log.2 should write");

    rotate_if_needed(&path, 1, 0).expect("rotation should succeed");

    assert!(!path.exists(), "active log should be removed");
    assert!(!rotated_log_path(&path, 1).exists());
    assert!(!rotated_log_path(&path, 2).exists());
}

#[test]
fn write_log_lock_timeout_preserves_line_and_records_issue() {
    let _data_dir = ScopedTestDataDir::new("log-lock-timeout");

    with_log_envs(
        &[
            ("REMEM_LOG_LOCK_TIMEOUT_MS", Some("1")),
            ("REMEM_STDERR_TO_LOG", Some("1")),
        ],
        || {
            let path = log_path().expect("log path should resolve");
            let lock_path = log_lock_path(&path);
            std::fs::create_dir_all(lock_path.parent().expect("lock should have parent"))
                .expect("lock parent should create");
            let lock_file = std::fs::OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .truncate(false)
                .open(&lock_path)
                .expect("lock file should open");
            lock_file
                .lock_exclusive()
                .expect("lock file should lock for test");

            info("log-timeout-test", "preserved-timeout-line");

            lock_file.unlock().expect("lock file should unlock");
            assert!(
                std::fs::read_to_string(&path)
                    .expect("active log should read")
                    .contains("preserved-timeout-line"),
                "fallback should preserve log line"
            );
            let issue = read_issue(&log_rotation_issue_path(&path));
            assert_eq!(issue.kind, "lock_timeout");
        },
    );
}

#[test]
fn write_log_rotate_failure_preserves_line_and_records_issue() {
    let _data_dir = ScopedTestDataDir::new("log-rotate-failure");

    with_log_envs(
        &[
            ("REMEM_LOG_MAX_BYTES", Some("1")),
            ("REMEM_LOG_MAX_ROTATED_FILES", Some("1")),
            ("REMEM_STDERR_TO_LOG", Some("1")),
        ],
        || {
            let path = log_path().expect("log path should resolve");
            std::fs::create_dir_all(path.parent().expect("log should have parent"))
                .expect("log parent should create");
            std::fs::write(&path, "oversized").expect("active log should write");
            std::fs::create_dir(rotated_log_path(&path, 1)).expect("blocking dir should create");

            info("log-rotate-failure-test", "preserved-rotate-line");

            assert!(
                std::fs::read_to_string(&path)
                    .expect("active log should read")
                    .contains("preserved-rotate-line"),
                "fallback should preserve log line"
            );
            let issue = read_issue(&log_rotation_issue_path(&path));
            assert_eq!(issue.kind, "rotate_failed");
        },
    );
}

#[cfg(unix)]
#[test]
fn log_files_are_created_private_on_unix() {
    use std::os::unix::fs::PermissionsExt;

    let _data_dir = ScopedTestDataDir::new("log-permissions");

    with_log_envs(
        &[
            ("REMEM_LOG_MAX_BYTES", Some("1")),
            ("REMEM_LOG_MAX_ROTATED_FILES", Some("1")),
            ("REMEM_STDERR_TO_LOG", Some("1")),
        ],
        || {
            info("log-permission-test", "first-line");
            info("log-permission-test", "second-line");

            let path = log_path().expect("log path should resolve");
            assert_eq!(mode(&path), 0o600, "active log mode");
            assert_eq!(mode(&rotated_log_path(&path, 1)), 0o600, "rotated log mode");
            assert_eq!(mode(&log_lock_path(&path)), 0o600, "lock file mode");

            std::fs::remove_file(rotated_log_path(&path, 1)).expect("rotated log should remove");
            std::fs::create_dir(rotated_log_path(&path, 1)).expect("blocking dir should create");
            info("log-permission-test", "diagnostic-line");
            assert_eq!(
                mode(&log_rotation_issue_path(&path)),
                0o600,
                "diagnostic sidecar mode"
            );
        },
    );

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path)
            .expect("path should have metadata")
            .permissions()
            .mode()
            & 0o777
    }
}

#[test]
fn subprocess_writers_preserve_lines_and_retention() {
    let data_dir = unique_temp_dir("log-subprocess-writers");
    std::fs::create_dir_all(&data_dir).expect("subprocess log dir should create");
    let exe = std::env::current_exe().expect("test binary should resolve");
    let workers = 4;
    let lines_per_worker = 6;
    let max_rotated_files = 20;

    let mut children = Vec::new();
    for worker in 0..workers {
        let prefix = format!("worker-{worker}");
        let child = Command::new(&exe)
            .arg("--exact")
            .arg("log::tests::subprocess_log_writer_helper")
            .arg("--ignored")
            .arg("--nocapture")
            .env("REMEM_LOG_SUBPROCESS_WRITER", "1")
            .env("REMEM_LOG_SUBPROCESS_PREFIX", &prefix)
            .env("REMEM_LOG_SUBPROCESS_LINES", lines_per_worker.to_string())
            .env("REMEM_DATA_DIR", &data_dir)
            .env("REMEM_ALLOW_PLAINTEXT_DB", "1")
            .env("REMEM_LOG_MAX_BYTES", "256")
            .env("REMEM_LOG_MAX_ROTATED_FILES", max_rotated_files.to_string())
            .env("REMEM_LOG_LOCK_TIMEOUT_MS", "1000")
            .env("REMEM_STDERR_TO_LOG", "1")
            .spawn()
            .expect("child log writer should spawn");
        children.push((prefix, child));
    }

    for (prefix, mut child) in children {
        let status = child.wait().expect("child log writer should wait");
        assert!(status.success(), "child {prefix} should succeed: {status}");
    }

    let path = data_dir.join("remem.log");
    let combined = read_all_log_text(&path, max_rotated_files);
    for worker in 0..workers {
        for line in 0..lines_per_worker {
            let needle = format!("worker-{worker}-{line}");
            assert!(
                combined.contains(&needle),
                "combined logs should contain {needle}; got {combined:?}"
            );
        }
    }
    assert_no_suffix_above(&path, max_rotated_files);
    std::fs::remove_dir_all(&data_dir).expect("subprocess log dir should remove");
}

#[test]
#[ignore]
fn subprocess_log_writer_helper() {
    if std::env::var("REMEM_LOG_SUBPROCESS_WRITER").as_deref() != Ok("1") {
        return;
    }
    let prefix = std::env::var("REMEM_LOG_SUBPROCESS_PREFIX").expect("prefix should be set");
    let lines = std::env::var("REMEM_LOG_SUBPROCESS_LINES")
        .expect("line count should be set")
        .parse::<usize>()
        .expect("line count should parse");
    for index in 0..lines {
        info("log-subprocess-writer", &format!("{prefix}-{index}"));
    }
}

fn read_issue(path: &Path) -> LogRotationIssue {
    let bytes = std::fs::read(path).expect("issue sidecar should read");
    serde_json::from_slice(&bytes).expect("issue sidecar should parse")
}

fn read_all_log_text(path: &Path, max_rotated_files: usize) -> String {
    let mut text = std::fs::read_to_string(path).unwrap_or_default();
    for index in 1..=max_rotated_files {
        text.push_str(&std::fs::read_to_string(rotated_log_path(path, index)).unwrap_or_default());
    }
    text
}

fn assert_no_suffix_above(path: &Path, max_rotated_files: usize) {
    let Some(parent) = path.parent() else {
        return;
    };
    let Some(base_name) = path.file_name().and_then(|name| name.to_str()) else {
        return;
    };
    let prefix = format!("{base_name}.");
    for entry in std::fs::read_dir(parent).expect("log dir should read") {
        let entry = entry.expect("log dir entry should read");
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(suffix) = name.strip_prefix(&prefix) else {
            continue;
        };
        let Ok(index) = suffix.parse::<usize>() else {
            continue;
        };
        assert!(
            index <= max_rotated_files,
            "suffix {index} should not exceed retention {max_rotated_files}"
        );
    }
}

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "remem-test-{}-{}-{}",
        label,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos()
    ))
}

struct ScopedLogEnv {
    previous: Vec<(&'static str, Option<OsString>)>,
}

impl ScopedLogEnv {
    fn set(vars: &[(&'static str, Option<&str>)]) -> Self {
        let previous = vars
            .iter()
            .map(|(name, _)| (*name, std::env::var_os(name)))
            .collect::<Vec<_>>();

        for (name, value) in vars {
            match value {
                Some(value) => unsafe { std::env::set_var(name, value) },
                None => unsafe { std::env::remove_var(name) },
            }
        }

        Self { previous }
    }
}

impl Drop for ScopedLogEnv {
    fn drop(&mut self) {
        for (name, value) in self.previous.drain(..) {
            match value {
                Some(value) => unsafe { std::env::set_var(name, value) },
                None => unsafe { std::env::remove_var(name) },
            }
        }
    }
}
