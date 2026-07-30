use std::fs;
#[cfg(unix)]
use std::fs::File;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::{check_plaintext_artifacts_in, Status, SQLITE_PLAINTEXT_HEADER};

fn arbitrary_backup_test_dir() -> PathBuf {
    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    let dir = std::env::temp_dir().join(format!(
        "remem-plaintext-arbitrary-unreadable-{}-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).expect("test directory should create");
    dir
}

fn write_plaintext_db(path: &Path) {
    let mut content = SQLITE_PLAINTEXT_HEADER.to_vec();
    content.extend_from_slice(&[0_u8; 32]);
    fs::write(path, content).expect("plaintext fixture should write");
}

fn write_non_plaintext_db(path: &Path) {
    fs::write(path, [0xAB_u8; 64]).expect("non-plaintext fixture should write");
}

#[test]
fn root_custom_sqlite_backup_is_detected() {
    let dir = arbitrary_backup_test_dir();
    let db_path = dir.join("remem.db");
    write_non_plaintext_db(&db_path);
    let backup = dir.join("pre-encryption.sqlite");
    write_plaintext_db(&backup);

    let result = check_plaintext_artifacts_in(&dir, &db_path, true);

    assert_eq!(result.status, Status::Fail);
    assert!(result.detail.contains(&backup.display().to_string()));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn custom_subdirectory_backup_is_detected() {
    let dir = arbitrary_backup_test_dir();
    let db_path = dir.join("remem.db");
    write_non_plaintext_db(&db_path);
    let backup = dir.join("archive").join("pre-encryption.sqlite");
    fs::create_dir_all(backup.parent().expect("backup should have parent"))
        .expect("custom backup directory should create");
    write_plaintext_db(&backup);

    let result = check_plaintext_artifacts_in(&dir, &db_path, true);

    assert_eq!(result.status, Status::Fail);
    assert!(result.detail.contains(&backup.display().to_string()));
    fs::remove_dir_all(&dir).ok();
}

#[cfg(unix)]
#[test]
fn readable_external_key_symlink_is_not_an_artifact_warning() {
    use std::os::unix::fs::symlink;

    let dir = arbitrary_backup_test_dir();
    let external = arbitrary_backup_test_dir();
    let db_path = dir.join("remem.db");
    write_non_plaintext_db(&db_path);
    let external_key = external.join("key");
    fs::write(&external_key, format!("v2:{}", "a".repeat(64)))
        .expect("external key fixture should write");
    symlink(&external_key, dir.join(".key")).expect("key symlink should create");

    let result = check_plaintext_artifacts_in(&dir, &db_path, true);

    assert_eq!(result.status, Status::Ok);
    assert!(!result.detail.contains(".key"), "{}", result.detail);
    fs::remove_dir_all(&dir).ok();
    fs::remove_dir_all(&external).ok();
}

#[cfg(unix)]
#[test]
fn live_database_sidecar_symlinks_are_not_artifact_warnings() {
    use std::os::unix::fs::symlink;

    let dir = arbitrary_backup_test_dir();
    let external = arbitrary_backup_test_dir();
    let db_path = dir.join("remem.db");
    write_non_plaintext_db(&db_path);
    for suffix in ["-wal", "-shm", "-journal"] {
        let target = external.join(format!("sidecar{suffix}"));
        fs::write(&target, b"runtime sidecar").expect("sidecar target should write");
        symlink(&target, dir.join(format!("remem.db{suffix}")))
            .expect("sidecar symlink should create");
    }

    let result = check_plaintext_artifacts_in(&dir, &db_path, true);

    assert_eq!(result.status, Status::Ok);
    for suffix in ["-wal", "-shm", "-journal"] {
        assert!(
            !result.detail.contains(&format!("remem.db{suffix}")),
            "{}",
            result.detail
        );
    }
    fs::remove_dir_all(&dir).ok();
    fs::remove_dir_all(&external).ok();
}

#[cfg(unix)]
#[test]
fn permission_restricted_arbitrary_backup_never_reports_ok() {
    let dir = arbitrary_backup_test_dir();
    let db_path = dir.join("remem.db");
    fs::write(&db_path, [0xAB_u8; 64]).expect("encrypted-header fixture should write");
    let backup = dir.join("pre-encryption-backup");
    let mut plaintext = SQLITE_PLAINTEXT_HEADER.to_vec();
    plaintext.extend_from_slice(&[0_u8; 32]);
    fs::write(&backup, plaintext).expect("plaintext backup fixture should write");
    fs::set_permissions(&backup, fs::Permissions::from_mode(0o000))
        .expect("backup should become unreadable");
    let access_denied = File::open(&backup).is_err();

    let result = check_plaintext_artifacts_in(&dir, &db_path, true);

    assert_eq!(
        result.status,
        if access_denied {
            Status::Warn
        } else {
            Status::Fail
        }
    );
    assert!(result.detail.contains("pre-encryption-backup"));
    if access_denied {
        assert!(result.detail.contains("cannot open"));
    } else {
        assert!(result.detail.contains("confirmed plaintext"));
    }
    fs::set_permissions(&backup, fs::Permissions::from_mode(0o600))
        .expect("backup permissions should restore");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn root_content_scan_reports_candidates_without_key_or_log_noise() {
    let dir = arbitrary_backup_test_dir();
    let db_path = dir.join("remem.db");
    write_non_plaintext_db(&db_path);
    write_plaintext_db(&dir.join("pre-encryption-backup"));
    fs::write(dir.join("truncated.sqlite"), b"short").expect("short fixture should write");
    fs::write(dir.join(".key"), b"short").expect("key fixture should write");
    fs::write(dir.join("remem.log"), b"short").expect("log fixture should write");
    write_non_plaintext_db(&dir.join("unrelated.txt"));
    write_non_plaintext_db(&dir.join("noise.sqlite"));
    #[cfg(unix)]
    let unreadable = {
        let path = dir.join("unreadable.sqlite");
        write_plaintext_db(&path);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000))
            .expect("fixture permissions should change");
        let access_denied = File::open(&path).is_err();
        (path, access_denied)
    };

    let result = check_plaintext_artifacts_in(&dir, &db_path, true);

    assert_eq!(result.status, Status::Fail);
    for expected in ["pre-encryption-backup", "truncated.sqlite", "shorter than"] {
        assert!(result.detail.contains(expected), "{}", result.detail);
    }
    #[cfg(unix)]
    if unreadable.1 {
        assert!(result.detail.contains("cannot open"));
        assert!(result.detail.contains("unreadable.sqlite"));
    } else {
        assert!(result.detail.contains("unreadable.sqlite"));
    }
    for excluded in [".key", "remem.log", "unrelated.txt", "noise.sqlite"] {
        assert!(!result.detail.contains(excluded), "{}", result.detail);
    }
    #[cfg(unix)]
    fs::set_permissions(unreadable.0, fs::Permissions::from_mode(0o600))
        .expect("fixture permissions should restore");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn plaintext_live_db_remediation_orders_encryption_backup_and_disposal() {
    let dir = arbitrary_backup_test_dir();
    let db_path = dir.join("remem.db");
    for name in ["remem.db", "remem.db.bak", "remem.db.enc"] {
        write_plaintext_db(&dir.join(name));
    }

    let result = check_plaintext_artifacts_in(&dir, &db_path, true);

    assert_eq!(result.status, Status::Warn);
    let steps = [
        "`REMEM_DATA_DIR/remem.db.bak`",
        "`REMEM_DATA_DIR/remem.db.enc`",
        "`REMEM_DATA_DIR/backups/`",
        "real directory and not a symbolic link",
        "distinct unused destination name",
        "without overwriting any existing file",
        "`remem encrypt`",
        "`remem status`",
        "`remem doctor`",
        "`remem admin backup`",
        "manually delete",
    ]
    .map(|step| result.detail.find(step).expect("ordered remediation step"));
    assert!(steps.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(result
        .detail
        .contains("rerun `remem doctor` until Plaintext residue passes"));
    for shell_fragment in ["export ", "mkdir -p", "mktemp", "mv \"", "[ -L "] {
        assert!(!result.detail.contains(shell_fragment), "{}", result.detail);
    }
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
