use std::ffi::OsString;
use std::sync::Mutex;

use crate::db::test_support::ScopedTestDataDir;

use super::super::logging::check_log_health;
use super::super::report::{run_doctor_with_writer, DoctorOptions};
use super::super::types::Status;

static LOG_ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn check_log_health_warns_on_invalid_env_without_exposing_values() {
    let _data_dir = ScopedTestDataDir::new("doctor-log-invalid-env");

    with_doctor_log_envs(
        &[
            ("REMEM_LOG_MAX_BYTES", Some("not-a-size-secret")),
            ("REMEM_LOG_MAX_ROTATED_FILES", Some("invalid-retention")),
            ("REMEM_LOG_LOCK_TIMEOUT_MS", Some("0")),
        ],
        || {
            let check = check_log_health();
            assert!(matches!(check.status, Status::Warn));
            assert!(check.detail.contains("invalid env fallback"));
            assert!(check.detail.contains("REMEM_LOG_MAX_BYTES"));
            assert!(check.detail.contains("REMEM_LOG_MAX_ROTATED_FILES"));
            assert!(check.detail.contains("REMEM_LOG_LOCK_TIMEOUT_MS"));
            assert!(!check.detail.contains("not-a-size-secret"));
            assert!(!check.detail.contains("invalid-retention"));
        },
    );
}

#[test]
fn check_log_health_reports_recent_rotation_issue() {
    let data_dir = ScopedTestDataDir::new("doctor-log-rotation-issue");

    with_doctor_log_envs(
        &[
            ("REMEM_LOG_MAX_BYTES", Some("1")),
            ("REMEM_LOG_MAX_ROTATED_FILES", Some("1")),
            ("REMEM_STDERR_TO_LOG", Some("1")),
        ],
        || {
            let path = data_dir.path.join("remem.log");
            std::fs::create_dir_all(path.parent().expect("log should have parent"))
                .expect("log parent should create");
            std::fs::write(&path, "oversized").expect("active log should write");
            std::fs::create_dir(rotated_path(&path, 1)).expect("blocking dir should create");

            crate::log::info("doctor-log-health-test", "preserved-doctor-line");

            let check = check_log_health();
            assert!(matches!(check.status, Status::Warn));
            assert!(check.detail.contains("recent rotation issue"));
            assert!(check.detail.contains("rotate_failed"));
            assert!(check.detail.contains("retained+active"));
        },
    );
}

#[test]
fn run_doctor_json_includes_log_health_check() {
    let _data_dir = ScopedTestDataDir::new("doctor-log-json");
    let _db = crate::db::open_db().expect("db should open");

    let mut buf = Vec::new();
    run_doctor_with_writer(
        DoctorOptions {
            json: true,
            quiet: false,
        },
        &mut buf,
    )
    .expect("doctor json should run");

    let json: serde_json::Value = serde_json::from_slice(&buf).expect("doctor json should parse");
    let checks = json["checks"]
        .as_array()
        .expect("doctor checks should be an array");
    assert!(
        checks
            .iter()
            .any(|check| check["name"].as_str() == Some("Log health")),
        "doctor JSON should include Log health check"
    );
}

fn with_doctor_log_envs<T>(vars: &[(&'static str, Option<&str>)], f: impl FnOnce() -> T) -> T {
    let _guard = LOG_ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _env = ScopedDoctorLogEnv::set(vars);
    f()
}

struct ScopedDoctorLogEnv {
    previous: Vec<(&'static str, Option<OsString>)>,
}

impl ScopedDoctorLogEnv {
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

impl Drop for ScopedDoctorLogEnv {
    fn drop(&mut self) {
        for (name, value) in self.previous.drain(..) {
            match value {
                Some(value) => unsafe { std::env::set_var(name, value) },
                None => unsafe { std::env::remove_var(name) },
            }
        }
    }
}

fn rotated_path(path: &std::path::Path, index: usize) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("{}.{}", path.display(), index))
}
