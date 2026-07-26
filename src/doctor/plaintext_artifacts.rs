use std::io::Read;
use std::path::Path;

use super::types::{Check, Status};
use crate::db;

const SQLITE_PLAINTEXT_HEADER: &[u8; 16] = b"SQLite format 3\0";

/// Detect plaintext SQLite residue next to the (normally encrypted) database.
///
/// Older remem versions could leave `remem.db.bak` / `remem.db.bak-<ts>` /
/// `remem.db.enc` artifacts behind after encryption or repair flows. A
/// plaintext copy beside an encrypted database defeats SQLCipher entirely, so
/// this surfaces as a failure with manual disposal guidance — doctor never
/// deletes user data itself.
pub(super) fn check_plaintext_artifacts() -> Check {
    let db_path = db::db_path();
    let Some(data_dir) = db_path.parent().map(Path::to_path_buf) else {
        return Check::new(
            "Plaintext residue",
            Status::Ok,
            "database path has no parent directory to scan",
        );
    };
    check_plaintext_artifacts_in(&data_dir, &db_path)
}

fn check_plaintext_artifacts_in(data_dir: &Path, db_path: &Path) -> Check {
    let db_file_name = db_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string();
    let entries = match std::fs::read_dir(data_dir) {
        Ok(entries) => entries,
        Err(error) => {
            return Check::new(
                "Plaintext residue",
                Status::Warn,
                format!("cannot scan data dir {}: {error}", data_dir.display()),
            );
        }
    };

    let mut plaintext = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !is_database_artifact_name(name, &db_file_name) {
            continue;
        }
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if file_has_plaintext_sqlite_header(&path) {
            let size_mb = std::fs::metadata(&path)
                .map(|meta| meta.len() as f64 / 1_048_576.0)
                .unwrap_or(0.0);
            plaintext.push(format!("{} ({size_mb:.1} MB)", path.display()));
        }
    }

    if plaintext.is_empty() {
        return Check::new(
            "Plaintext residue",
            Status::Ok,
            "no plaintext database artifacts in the data dir",
        );
    }

    let main_db_encrypted = db_path.exists() && !file_has_plaintext_sqlite_header(db_path);
    let detail = format!(
        "plaintext database artifact(s) beside the {} database: {}; verify the live database opens (`remem status`), then delete these files or move them outside the data dir",
        if main_db_encrypted {
            "encrypted"
        } else {
            "plaintext"
        },
        plaintext.join(", ")
    );
    let status = if main_db_encrypted {
        Status::Fail
    } else {
        Status::Warn
    };
    Check::new("Plaintext residue", status, detail)
}

/// Sibling artifacts of the database file (`remem.db.bak`, `remem.db.bak-<ts>`,
/// `remem.db.enc`, …) — everything that starts with the database file name
/// except the database itself and its live SQLite sidecars.
fn is_database_artifact_name(name: &str, db_file_name: &str) -> bool {
    if db_file_name.is_empty() || name == db_file_name {
        return false;
    }
    let Some(suffix) = name.strip_prefix(db_file_name) else {
        return false;
    };
    !matches!(suffix, "-wal" | "-shm" | "-journal")
}

fn file_has_plaintext_sqlite_header(path: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut header = [0_u8; 16];
    match file.read_exact(&mut header) {
        Ok(()) => &header == SQLITE_PLAINTEXT_HEADER,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "remem-plaintext-artifacts-{name}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir should create");
        dir
    }

    fn write_plaintext_db(path: &Path) {
        let mut content = SQLITE_PLAINTEXT_HEADER.to_vec();
        content.extend_from_slice(&[0_u8; 32]);
        std::fs::write(path, content).expect("plaintext fixture should write");
    }

    fn write_encrypted_db(path: &Path) {
        std::fs::write(path, [0xAB_u8; 64]).expect("encrypted fixture should write");
    }

    #[test]
    fn reports_ok_without_artifacts() {
        let dir = temp_dir("ok");
        let db_path = dir.join("remem.db");
        write_encrypted_db(&db_path);
        std::fs::write(dir.join("remem.db-wal"), b"SQLite format 3\0wal")
            .expect("wal fixture should write");

        let check = check_plaintext_artifacts_in(&dir, &db_path);

        assert_eq!(check.status, Status::Ok);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fails_on_plaintext_bak_beside_encrypted_db() {
        let dir = temp_dir("fail");
        let db_path = dir.join("remem.db");
        write_encrypted_db(&db_path);
        write_plaintext_db(&dir.join("remem.db.bak"));
        write_plaintext_db(&dir.join("remem.db.bak-20260324160017"));

        let check = check_plaintext_artifacts_in(&dir, &db_path);

        assert_eq!(check.status, Status::Fail);
        assert!(check.detail.contains("remem.db.bak"));
        assert!(check.detail.contains("remem.db.bak-20260324160017"));
        assert!(check.detail.contains("`remem status`"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn warns_when_main_db_is_also_plaintext() {
        let dir = temp_dir("warn");
        let db_path = dir.join("remem.db");
        write_plaintext_db(&db_path);
        write_plaintext_db(&dir.join("remem.db.bak"));

        let check = check_plaintext_artifacts_in(&dir, &db_path);

        assert_eq!(check.status, Status::Warn);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ignores_encrypted_artifacts_and_live_sidecars() {
        let dir = temp_dir("ignore");
        let db_path = dir.join("remem.db");
        write_encrypted_db(&db_path);
        write_encrypted_db(&dir.join("remem.db.bak"));
        std::fs::write(dir.join("remem.db-shm"), b"SQLite format 3\0shm")
            .expect("shm fixture should write");
        std::fs::write(dir.join("unrelated.txt"), b"SQLite format 3\0nope")
            .expect("unrelated fixture should write");

        let check = check_plaintext_artifacts_in(&dir, &db_path);

        assert_eq!(check.status, Status::Ok);
        std::fs::remove_dir_all(&dir).ok();
    }
}
