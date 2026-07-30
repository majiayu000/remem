use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use super::{check_plaintext_artifacts_in, Status, SQLITE_PLAINTEXT_HEADER};

fn arbitrary_backup_test_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "remem-plaintext-arbitrary-unreadable-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    fs::create_dir_all(&dir).expect("test directory should create");
    dir
}

#[test]
fn unreadable_arbitrary_root_backup_keeps_inspection_incomplete() {
    let dir = arbitrary_backup_test_dir();
    let db_path = dir.join("remem.db");
    fs::write(&db_path, [0xAB_u8; 64]).expect("encrypted-header fixture should write");
    let backup = dir.join("pre-encryption-backup");
    let mut plaintext = SQLITE_PLAINTEXT_HEADER.to_vec();
    plaintext.extend_from_slice(&[0_u8; 32]);
    fs::write(&backup, plaintext).expect("plaintext backup fixture should write");
    fs::set_permissions(&backup, fs::Permissions::from_mode(0o000))
        .expect("backup should become unreadable");

    let result = check_plaintext_artifacts_in(&dir, &db_path, true);

    assert_eq!(result.status, Status::Warn);
    assert!(result.detail.contains("pre-encryption-backup"));
    assert!(result.detail.contains("cannot open"));
    fs::set_permissions(&backup, fs::Permissions::from_mode(0o600))
        .expect("backup permissions should restore");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn short_named_artifact_keeps_inspection_incomplete() {
    let dir = arbitrary_backup_test_dir();
    let db_path = dir.join("remem.db");
    fs::write(&db_path, [0xAB_u8; 64]).expect("encrypted-header fixture should write");
    fs::write(dir.join("remem.db.bak"), b"short").expect("short fixture should write");

    let result = check_plaintext_artifacts_in(&dir, &db_path, true);

    assert_eq!(result.status, Status::Warn);
    assert!(result.detail.contains("shorter than the SQLite header"));
    fs::remove_dir_all(&dir).ok();
}
