use std::ffi::OsStr;
use std::fs::{self, File, Metadata};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use super::types::{Check, Status};
use crate::db;

mod hf_cache;

const SQLITE_PLAINTEXT_HEADER: &[u8; 16] = b"SQLite format 3\0";

#[derive(Debug)]
struct PlaintextArtifact {
    path: PathBuf,
    size_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FileHeader {
    Plaintext,
    NonPlaintext,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LiveDatabaseState {
    Encrypted,
    Plaintext,
    Missing,
    Unverified,
}

impl LiveDatabaseState {
    fn description(self) -> &'static str {
        match self {
            Self::Encrypted => "encrypted and readable",
            Self::Plaintext => "plaintext",
            Self::Missing => "missing",
            Self::Unverified => "unverified",
        }
    }

    fn remediation(self) -> &'static str {
        match self {
            Self::Encrypted => {
                "remem doctor did not delete any files. Run `remem status` to confirm the \
encrypted live database still opens, then run `remem admin backup` to create a new encrypted \
backup. After the new backup succeeds, manually delete the listed plaintext copies or retain \
them only in encrypted storage. Moving them outside the remem data dir alone does not protect them"
            }
            Self::Plaintext => {
                "remem doctor did not delete any files. First check for the encryption-blocking \
files `REMEM_DATA_DIR/remem.db.bak` and \
`REMEM_DATA_DIR/remem.db.enc`. If either exists, verify that \
`REMEM_DATA_DIR/backups/` is a real directory and not a symbolic link, choose a distinct unused \
destination name there for each blocker, and move each blocker without overwriting any existing \
file. Only after every existing blocker has been preserved successfully should you run `remem \
encrypt`, then run `remem status` to confirm the live database opens. Run `remem doctor` and \
verify that its Plaintext residue detail reports the live database as encrypted and readable; \
that check is expected to remain `Fail` while the preserved plaintext copies exist. Only after \
that confirmation should you run `remem admin backup` to create a new encrypted backup. After \
the backup succeeds, manually delete the listed plaintext copies or retain them only in encrypted \
storage, then rerun `remem doctor` until Plaintext residue passes. Moving them outside the remem \
data dir alone does not protect them"
            }
            Self::Missing => {
                "remem doctor did not delete any files. Do not create a new backup or delete any \
listed copy yet. First restore the missing live database, resolve any key mismatch, and make the \
database readable. After `remem status` succeeds and `remem doctor` reports the live database as \
encrypted and readable, run `remem admin backup` to create a new encrypted backup; only after \
that succeeds should you manually delete the listed plaintext copies or retain them only in \
encrypted storage. Moving them outside the remem data dir alone does not protect them"
            }
            Self::Unverified => {
                "remem doctor did not delete any files. Do not create a new backup or delete any \
listed copy yet. First repair the live database and resolve any key or readability problem. After \
`remem status` succeeds and `remem doctor` reports the live database as encrypted and readable, \
run `remem admin backup` to create a new encrypted backup; only after that succeeds should you \
manually delete the listed plaintext copies or retain them only in encrypted storage. Moving them \
outside the remem data dir alone does not protect them"
            }
        }
    }
}

/// Detect plaintext SQLite residue throughout the managed data directory.
///
/// The shared doctor connection is the source of truth for live-database
/// readability. A non-plaintext header alone is not proof of encryption: a
/// corrupt or inaccessible database can have the same bytes.
pub(super) fn check_plaintext_artifacts(live_db_readable: bool) -> Check {
    let db_path = db::db_path();
    let Some(data_dir) = db_path.parent().map(Path::to_path_buf) else {
        return Check::new(
            "Plaintext residue",
            Status::Warn,
            "cannot scan plaintext residue because the database path has no parent directory",
        );
    };
    check_plaintext_artifacts_in(&data_dir, &db_path, live_db_readable)
}

fn check_plaintext_artifacts_in(data_dir: &Path, db_path: &Path, live_db_readable: bool) -> Check {
    let mut plaintext = Vec::new();
    let mut issues = Vec::new();
    scan_data_directory(data_dir, db_path, &mut plaintext, &mut issues);
    let live_state = inspect_live_database(db_path, live_db_readable, &mut issues);
    plaintext.sort_by(|left, right| left.path.cmp(&right.path));
    plaintext.dedup_by(|left, right| left.path == right.path);
    issues.sort();
    issues.dedup();
    if plaintext.is_empty() {
        if issues.is_empty() {
            return Check::new(
                "Plaintext residue",
                Status::Ok,
                "no plaintext database artifacts found in the managed data directory tree",
            );
        }
        return Check::new(
            "Plaintext residue",
            Status::Warn,
            format!(
                "no plaintext database artifact was confirmed, but inspection was incomplete: {}",
                issues.join("; ")
            ),
        );
    }
    let artifacts = plaintext
        .iter()
        .map(|artifact| {
            format!(
                "{} ({:.1} MB)",
                artifact.path.display(),
                artifact.size_bytes as f64 / 1_048_576.0
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let inspection_issues = if issues.is_empty() {
        String::new()
    } else {
        format!(" Inspection issue(s): {}.", issues.join("; "))
    };
    let detail = format!(
        "confirmed plaintext database artifact(s): {artifacts}. Live database is {}.{inspection_issues} {}",
        live_state.description(),
        live_state.remediation()
    );
    let status = if live_state == LiveDatabaseState::Encrypted {
        Status::Fail
    } else {
        Status::Warn
    };
    Check::new("Plaintext residue", status, detail)
}

fn scan_data_directory(
    data_dir: &Path,
    db_path: &Path,
    plaintext: &mut Vec<PlaintextArtifact>,
    issues: &mut Vec<String>,
) {
    let mut directories = vec![data_dir.to_path_buf()];
    while let Some(directory) = directories.pop() {
        scan_directory(
            &directory,
            data_dir,
            db_path,
            plaintext,
            issues,
            &mut directories,
        );
    }
}

fn scan_directory(
    directory: &Path,
    data_dir: &Path,
    db_path: &Path,
    plaintext: &mut Vec<PlaintextArtifact>,
    issues: &mut Vec<String>,
    child_directories: &mut Vec<PathBuf>,
) {
    let directory_metadata = match fs::symlink_metadata(directory) {
        Ok(metadata) => metadata,
        Err(error) => {
            issues.push(format!(
                "cannot inspect directory {}: {error}",
                directory.display()
            ));
            return;
        }
    };
    if directory_metadata.file_type().is_symlink() {
        issues.push(format!(
            "refusing to scan symbolic-link directory {}",
            directory.display()
        ));
        return;
    }
    if !directory_metadata.is_dir() {
        issues.push(format!(
            "scan path {} is not a directory",
            directory.display()
        ));
        return;
    }
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            issues.push(format!(
                "cannot enumerate directory {}: {error}",
                directory.display()
            ));
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                issues.push(format!(
                    "cannot enumerate an entry in {}: {error}",
                    directory.display()
                ));
                continue;
            }
        };
        let path = entry.path();
        let entry_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                issues.push(format!(
                    "cannot determine file type for {}: {error}",
                    path.display()
                ));
                continue;
            }
        };
        if entry_type.is_symlink() {
            if is_known_non_artifact_path(data_dir, db_path, &path)
                || hf_cache::is_snapshot_pointer(data_dir, &path)
            {
                continue;
            }
            let description = if path == data_dir.join("backups") {
                "symbolic-link backup directory"
            } else {
                "symbolic-link artifact"
            };
            issues.push(format!(
                "refusing to inspect {description} {}",
                path.display()
            ));
            continue;
        }
        if entry_type.is_dir() {
            child_directories.push(path);
            continue;
        }
        if path == db_path {
            continue;
        }
        if path == data_dir.join("backups") {
            issues.push(format!("scan path {} is not a directory", path.display()));
            continue;
        }
        if !entry_type.is_file() {
            issues.push(format!(
                "artifact candidate {} is not a regular file",
                path.display()
            ));
            continue;
        }
        let path_metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                issues.push(format!(
                    "cannot inspect metadata for {}: {error}",
                    path.display()
                ));
                continue;
            }
        };
        if path_metadata.file_type().is_symlink() || !path_metadata.is_file() {
            issues.push(format!(
                "artifact candidate {} changed before it could be inspected safely",
                path.display()
            ));
            continue;
        }
        let ignore_short_file = !is_named_database_artifact(db_path, &path);
        match inspect_regular_file(&path, &path_metadata, ignore_short_file) {
            Ok((FileHeader::Plaintext, size_bytes)) => {
                plaintext.push(PlaintextArtifact { path, size_bytes });
            }
            Ok((FileHeader::NonPlaintext, _)) => {}
            Err(error) => issues.push(error),
        }
    }
}

fn is_known_non_artifact_path(data_dir: &Path, db_path: &Path, path: &Path) -> bool {
    if path == data_dir.join(".key") {
        return true;
    }
    ["-wal", "-shm", "-journal"].iter().any(|suffix| {
        let mut sidecar = db_path.as_os_str().to_os_string();
        sidecar.push(suffix);
        path == Path::new(&sidecar)
    })
}

fn is_named_database_artifact(db_path: &Path, path: &Path) -> bool {
    if path
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| {
            ["db", "db3", "sqlite", "sqlite3"]
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
    {
        return true;
    }
    let Some(db_file_name) = db_path.file_name().and_then(OsStr::to_str) else {
        return false;
    };
    path.file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|name| {
            name == format!("{db_file_name}.bak")
                || name == format!("{db_file_name}.enc")
                || name.starts_with(&format!("{db_file_name}.bak-"))
        })
}

fn inspect_live_database(
    db_path: &Path,
    live_db_readable: bool,
    issues: &mut Vec<String>,
) -> LiveDatabaseState {
    let path_metadata = match fs::symlink_metadata(db_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return LiveDatabaseState::Missing;
        }
        Err(error) => {
            issues.push(format!(
                "cannot inspect live database metadata {}: {error}",
                db_path.display()
            ));
            return LiveDatabaseState::Unverified;
        }
    };
    if path_metadata.file_type().is_symlink() {
        issues.push(format!(
            "refusing to inspect symbolic-link live database {}",
            db_path.display()
        ));
        return LiveDatabaseState::Unverified;
    }
    if !path_metadata.is_file() {
        issues.push(format!(
            "live database {} is not a regular file",
            db_path.display()
        ));
        return LiveDatabaseState::Unverified;
    }
    match inspect_regular_file(db_path, &path_metadata, false) {
        Ok((FileHeader::Plaintext, _)) => LiveDatabaseState::Plaintext,
        Ok((FileHeader::NonPlaintext, _)) if live_db_readable => LiveDatabaseState::Encrypted,
        Ok((FileHeader::NonPlaintext, _)) => LiveDatabaseState::Unverified,
        Err(error) => {
            issues.push(error);
            LiveDatabaseState::Unverified
        }
    }
}

fn inspect_regular_file(
    path: &Path,
    path_metadata: &Metadata,
    ignore_short_file: bool,
) -> Result<(FileHeader, u64), String> {
    let mut file =
        File::open(path).map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    let opened_metadata = file
        .metadata()
        .map_err(|error| format!("cannot inspect opened file {}: {error}", path.display()))?;
    if !opened_metadata.is_file() || !same_file(path_metadata, &opened_metadata) {
        return Err(format!(
            "file {} changed before it could be inspected safely",
            path.display()
        ));
    }
    let mut header = [0_u8; SQLITE_PLAINTEXT_HEADER.len()];
    let short_file = match file.read_exact(&mut header) {
        Ok(()) => false,
        Err(error) if ignore_short_file && error.kind() == io::ErrorKind::UnexpectedEof => true,
        Err(error) => {
            return Err(if error.kind() == io::ErrorKind::UnexpectedEof {
                format!(
                    "cannot confirm {} because it is shorter than the SQLite header",
                    path.display()
                )
            } else {
                format!("cannot read header from {}: {error}", path.display())
            });
        }
    };
    let current_metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot recheck metadata for {}: {error}", path.display()))?;
    if current_metadata.file_type().is_symlink()
        || !current_metadata.is_file()
        || !same_file(&opened_metadata, &current_metadata)
    {
        return Err(format!(
            "file {} changed while it was being inspected",
            path.display()
        ));
    }
    let header = if !short_file && &header == SQLITE_PLAINTEXT_HEADER {
        FileHeader::Plaintext
    } else {
        FileHeader::NonPlaintext
    };
    Ok((header, opened_metadata.len()))
}

#[cfg(unix)]
fn same_file(left: &Metadata, right: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(not(unix))]
fn same_file(left: &Metadata, right: &Metadata) -> bool {
    let _ = (left, right);
    true
}

#[cfg(test)]
mod regression_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "remem-plaintext-artifacts-{name}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&dir).expect("temp dir should create");
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

    fn check(dir: &Path, live_db_readable: bool) -> Check {
        check_plaintext_artifacts_in(dir, &dir.join("remem.db"), live_db_readable)
    }

    fn cleanup(dir: &Path) {
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn reports_ok_without_artifacts_and_excludes_sidecars_and_near_matches() {
        let dir = temp_dir("ok");
        write_non_plaintext_db(&dir.join("remem.db"));
        for name in [
            "remem.db-wal",
            "remem.db-shm",
            "remem.db-journal",
            "remem.db.bakcopy",
            "remem.db.enc-old",
            "unrelated.txt",
        ] {
            write_non_plaintext_db(&dir.join(name));
        }
        let result = check(&dir, true);
        assert_eq!(result.status, Status::Ok);
        cleanup(&dir);
    }

    #[test]
    fn missing_data_directory_is_an_incomplete_inspection_warning() {
        let dir = temp_dir("missing-data-dir");
        let db_path = dir.join("remem.db");
        cleanup(&dir);
        let result = check_plaintext_artifacts_in(&dir, &db_path, false);
        assert_eq!(result.status, Status::Warn);
        assert!(result.detail.contains("cannot inspect directory"));
        assert!(result.detail.contains(&dir.display().to_string()));
    }

    #[test]
    fn encrypted_sibling_and_backup_artifacts_are_ok() {
        let dir = temp_dir("encrypted-artifacts");
        write_non_plaintext_db(&dir.join("remem.db"));
        write_non_plaintext_db(&dir.join("remem.db.bak"));
        write_non_plaintext_db(&dir.join("remem.db.bak-"));
        write_non_plaintext_db(&dir.join("remem.db.enc"));
        fs::create_dir(dir.join("backups")).unwrap();
        write_non_plaintext_db(&dir.join("backups").join("backup.sqlite"));
        let result = check(&dir, true);
        assert_eq!(result.status, Status::Ok);
        cleanup(&dir);
    }

    #[test]
    fn scans_only_exact_sibling_artifact_names_in_stable_order() {
        let dir = temp_dir("siblings");
        write_non_plaintext_db(&dir.join("remem.db"));
        for name in [
            "remem.db.enc",
            "remem.db.bak-z",
            "remem.db.bak",
            "remem.db.bak-a",
        ] {
            write_plaintext_db(&dir.join(name));
        }
        let result = check(&dir, true);
        assert_eq!(result.status, Status::Fail);
        let positions = [
            "remem.db.bak (",
            "remem.db.bak-a (",
            "remem.db.bak-z (",
            "remem.db.enc (",
        ]
        .map(|needle| result.detail.find(needle).expect("artifact should appear"));
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
        cleanup(&dir);
    }

    #[test]
    fn scans_default_and_custom_files_in_backups_but_ignores_encrypted_backup() {
        let dir = temp_dir("backups");
        write_non_plaintext_db(&dir.join("remem.db"));
        let backups = dir.join("backups");
        fs::create_dir(&backups).unwrap();
        write_plaintext_db(&backups.join("remem-backup-20260728-120000.sqlite"));
        write_plaintext_db(&backups.join("custom-name.sqlite"));
        write_non_plaintext_db(&backups.join("encrypted.sqlite"));
        let result = check(&dir, true);
        assert_eq!(result.status, Status::Fail);
        assert!(result.detail.contains("custom-name.sqlite"));
        assert!(result
            .detail
            .contains("remem-backup-20260728-120000.sqlite"));
        assert!(!result.detail.contains("encrypted.sqlite"));
        cleanup(&dir);
    }

    #[test]
    fn scans_nested_backup_destinations() {
        let dir = temp_dir("nonrecursive");
        write_non_plaintext_db(&dir.join("remem.db"));
        let nested = dir.join("backups").join("nested");
        fs::create_dir_all(&nested).unwrap();
        write_plaintext_db(&nested.join("plaintext.sqlite"));
        let result = check(&dir, true);
        assert_eq!(result.status, Status::Fail);
        assert!(result.detail.contains("nested/plaintext.sqlite"));
        cleanup(&dir);
    }

    #[test]
    fn scans_custom_backup_directories_anywhere_in_data_dir() {
        let dir = temp_dir("custom-backup-tree");
        write_non_plaintext_db(&dir.join("remem.db"));
        let nested = dir.join("archive").join("before-encryption");
        fs::create_dir_all(&nested).unwrap();
        write_plaintext_db(&nested.join("custom-output"));
        let result = check(&dir, true);
        assert_eq!(result.status, Status::Fail);
        assert!(result
            .detail
            .contains("archive/before-encryption/custom-output"));
        cleanup(&dir);
    }

    #[test]
    fn plaintext_artifact_fails_only_for_readable_non_plaintext_live_db() {
        let dir = temp_dir("live-encrypted");
        write_non_plaintext_db(&dir.join("remem.db"));
        write_plaintext_db(&dir.join("remem.db.bak"));
        let result = check(&dir, true);
        assert_eq!(result.status, Status::Fail);
        assert!(result.detail.contains("encrypted and readable"));
        assert!(result.detail.contains("`remem status`"));
        assert!(result.detail.contains("`remem admin backup`"));
        assert!(result.detail.contains("create a new encrypted backup"));
        assert!(result.detail.contains("After the new backup succeeds"));
        assert!(result.detail.contains("did not delete any files"));
        assert!(result.detail.contains("alone does not protect"));
        assert!(!result.detail.contains("`remem encrypt`"));
        cleanup(&dir);
    }

    #[test]
    fn artifact_with_plaintext_live_db_warns_even_if_connection_is_readable() {
        let dir = temp_dir("live-plaintext");
        write_plaintext_db(&dir.join("remem.db"));
        write_plaintext_db(&dir.join("remem.db.enc"));
        let result = check(&dir, true);
        assert_eq!(result.status, Status::Warn);
        let detail = result.detail.as_str();
        assert!(detail.contains("Live database is plaintext"));
        assert!(detail.contains("every existing blocker has been preserved successfully"));
        assert!(detail.contains("expected to remain `Fail`"));
        assert!(detail.contains("Only after that confirmation"));
        assert!(detail.contains("rerun `remem doctor` until Plaintext residue passes"));
        assert!(detail.contains("did not delete any files"));
        assert!(detail.contains("alone does not protect"));
        cleanup(&dir);
    }

    #[test]
    fn artifact_with_missing_live_db_warns() {
        let dir = temp_dir("live-missing");
        write_plaintext_db(&dir.join("remem.db.bak"));
        let result = check(&dir, false);
        assert_eq!(result.status, Status::Warn);
        let detail = result.detail.as_str();
        assert!(detail.contains("Live database is missing"));
        assert!(detail.contains("Do not create a new backup or delete any listed copy yet"));
        assert!(detail.contains("restore the missing live database"));
        assert!(detail.contains("After `remem status` succeeds"));
        assert!(
            detail.contains("`remem doctor` reports the live database as encrypted and readable")
        );
        assert!(detail.contains("did not delete any files"));
        assert!(detail.contains("alone does not protect"));
        cleanup(&dir);
    }

    #[test]
    fn artifact_with_unreadable_non_plaintext_live_db_warns() {
        let dir = temp_dir("live-unverified");
        write_non_plaintext_db(&dir.join("remem.db"));
        write_plaintext_db(&dir.join("remem.db.bak"));
        let result = check(&dir, false);
        assert_eq!(result.status, Status::Warn);
        let detail = result.detail.as_str();
        assert!(detail.contains("Live database is unverified"));
        assert!(detail.contains("Do not create a new backup or delete any listed copy yet"));
        assert!(
            detail.contains("repair the live database and resolve any key or readability problem")
        );
        assert!(detail.contains("After `remem status` succeeds"));
        assert!(
            detail.contains("`remem doctor` reports the live database as encrypted and readable")
        );
        assert!(detail.contains("did not delete any files"));
        assert!(detail.contains("alone does not protect"));
        cleanup(&dir);
    }

    #[test]
    fn short_live_db_is_unverified_and_errors_are_aggregated_with_plaintext() {
        let dir = temp_dir("short-live");
        fs::write(dir.join("remem.db"), b"short").unwrap();
        write_plaintext_db(&dir.join("remem.db.bak"));
        fs::write(dir.join("remem.db.enc"), b"also short").unwrap();
        let result = check(&dir, false);
        assert_eq!(result.status, Status::Warn);
        assert!(result.detail.contains("remem.db.bak"));
        assert!(result.detail.contains("remem.db.enc"));
        assert!(result.detail.contains("remem.db"));
        assert!(result.detail.contains("Inspection issue(s)"));
        cleanup(&dir);
    }

    #[test]
    fn non_directory_backups_path_warns_without_hiding_plaintext() {
        let dir = temp_dir("bad-backups");
        write_non_plaintext_db(&dir.join("remem.db"));
        write_plaintext_db(&dir.join("remem.db.bak"));
        fs::write(dir.join("backups"), b"not a directory").unwrap();
        let result = check(&dir, true);
        assert_eq!(result.status, Status::Fail);
        assert!(result.detail.contains("remem.db.bak"));
        assert!(result.detail.contains("is not a directory"));
        cleanup(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_link_artifact_is_not_followed_and_warns() {
        use std::os::unix::fs::symlink;

        let dir = temp_dir("symlink");
        let target_dir = temp_dir("symlink-target");
        write_non_plaintext_db(&dir.join("remem.db"));
        let target = target_dir.join("target.sqlite");
        write_plaintext_db(&target);
        fs::create_dir(dir.join("backups")).unwrap();
        symlink(&target, dir.join("backups").join("linked.sqlite")).unwrap();
        let result = check(&dir, true);
        assert_eq!(result.status, Status::Warn);
        assert!(result.detail.contains("symbolic-link artifact"));
        assert!(!result
            .detail
            .contains("confirmed plaintext database artifact(s)"));
        cleanup(&dir);
        cleanup(&target_dir);
    }

    #[cfg(unix)]
    #[test]
    fn configured_key_symlink_is_excluded_but_database_symlinks_warn() {
        use std::os::unix::fs::symlink;

        let dir = temp_dir("key-symlink");
        let target_dir = temp_dir("key-symlink-target");
        write_non_plaintext_db(&dir.join("remem.db"));
        fs::write(target_dir.join("key"), "configured-key").unwrap();
        write_plaintext_db(&target_dir.join("backup"));
        symlink(target_dir.join("key"), dir.join(".key")).unwrap();
        symlink(target_dir.join("backup"), dir.join("custom.sqlite")).unwrap();
        let result = check(&dir, true);
        assert_eq!(result.status, Status::Warn);
        assert!(result.detail.contains("custom.sqlite"));
        assert!(!result.detail.contains(".key"));
        cleanup(&dir);
        cleanup(&target_dir);
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_link_error_is_reported_alongside_confirmed_plaintext() {
        use std::os::unix::fs::symlink;

        let dir = temp_dir("symlink-and-plaintext");
        write_non_plaintext_db(&dir.join("remem.db"));
        write_plaintext_db(&dir.join("remem.db.bak"));
        fs::create_dir(dir.join("targets")).unwrap();
        let target = dir.join("targets/target.sqlite");
        write_plaintext_db(&target);
        fs::create_dir(dir.join("backups")).unwrap();
        symlink(&target, dir.join("backups").join("linked.sqlite")).unwrap();
        let result = check(&dir, true);
        assert_eq!(result.status, Status::Fail);
        assert!(result.detail.contains("remem.db.bak"));
        assert!(result.detail.contains("symbolic-link artifact"));
        cleanup(&dir);
    }
}
