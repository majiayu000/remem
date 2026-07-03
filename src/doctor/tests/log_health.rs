use crate::db::test_support::ScopedTestDataDir;
use crate::log::test_support::with_log_test_data_dir as with_doctor_log_data_dir;

use super::super::logging::check_log_health;
use super::super::report::{run_doctor_with_writer, DoctorOptions};
use super::super::types::Status;

#[test]
fn check_log_health_warns_on_invalid_env_without_exposing_values() {
    with_doctor_log_data_dir(
        "doctor-log-invalid-env",
        &[
            ("REMEM_LOG_MAX_BYTES", Some("not-a-size-secret")),
            ("REMEM_LOG_MAX_ROTATED_FILES", Some("invalid-retention")),
            ("REMEM_LOG_LOCK_TIMEOUT_MS", Some("0")),
        ],
        |_| {
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
    with_doctor_log_data_dir(
        "doctor-log-rotation-issue",
        &[
            ("REMEM_LOG_MAX_BYTES", Some("1")),
            ("REMEM_LOG_MAX_ROTATED_FILES", Some("1")),
            ("REMEM_STDERR_TO_LOG", Some("1")),
        ],
        |data_dir| {
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

fn rotated_path(path: &std::path::Path, index: usize) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("{}.{}", path.display(), index))
}
