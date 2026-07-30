use std::ffi::OsStr;
use std::fs::{self, DirEntry, File, Metadata};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use super::types::{Check, Status};
use crate::db;

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
encrypt`, then run `remem status` and `remem doctor` until they report the live database as \
encrypted and readable. Before that confirmation, do not treat any `remem admin backup` output \
as encrypted. After confirmation, run `remem admin backup` to create a new encrypted backup; \
after it succeeds, manually delete the listed plaintext copies or retain them only in encrypted \
storage. Moving them outside the remem data dir alone does not protect them"
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

/// Detect plaintext SQLite residue next to the live database and in the first
/// level of the managed backup directory.
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
    scan_sibling_artifacts(data_dir, db_path, &mut plaintext, &mut issues);
    scan_backup_artifacts(data_dir, &mut plaintext, &mut issues);
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
                "no plaintext database artifacts found beside the live database or in the first level of the backups directory",
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

fn scan_sibling_artifacts(
    data_dir: &Path,
    db_path: &Path,
    plaintext: &mut Vec<PlaintextArtifact>,
    issues: &mut Vec<String>,
) {
    let Some(db_file_name) = db_path.file_name().and_then(OsStr::to_str) else {
        issues.push(format!(
            "cannot derive a UTF-8 database filename from {}",
            db_path.display()
        ));
        return;
    };
    let bak_name = format!("{db_file_name}.bak");
    let bak_prefix = format!("{db_file_name}.bak-");
    let enc_name = format!("{db_file_name}.enc");
    scan_directory(
        data_dir,
        false,
        false,
        |name| {
            name.to_str().is_some_and(|name| {
                name == bak_name
                    || name == enc_name
                    || name.starts_with(&bak_prefix)
                    || (name != db_file_name
                        && Path::new(name)
                            .extension()
                            .and_then(OsStr::to_str)
                            .is_some_and(|extension| {
                                ["db", "db3", "sqlite", "sqlite3"]
                                    .iter()
                                    .any(|candidate| extension.eq_ignore_ascii_case(candidate))
                            }))
            })
        },
        plaintext,
        issues,
    );
    let mut ignored_probe_issues = Vec::new();
    scan_directory(
        data_dir,
        false,
        true,
        |name| name != OsStr::new(db_file_name),
        plaintext,
        &mut ignored_probe_issues,
    );
}

fn scan_backup_artifacts(
    data_dir: &Path,
    plaintext: &mut Vec<PlaintextArtifact>,
    issues: &mut Vec<String>,
) {
    scan_directory(
        &data_dir.join("backups"),
        true,
        true,
        |_| true,
        plaintext,
        issues,
    );
}

fn scan_directory(
    directory: &Path,
    missing_ok: bool,
    ignore_child_directories: bool,
    is_candidate: impl Fn(&OsStr) -> bool,
    plaintext: &mut Vec<PlaintextArtifact>,
    issues: &mut Vec<String>,
) {
    let directory_metadata = match fs::symlink_metadata(directory) {
        Ok(metadata) => metadata,
        Err(error) if missing_ok && error.kind() == io::ErrorKind::NotFound => return,
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
        if !is_candidate(&entry.file_name()) {
            continue;
        }
        inspect_candidate(&entry, ignore_child_directories, plaintext, issues);
    }
}

fn inspect_candidate(
    entry: &DirEntry,
    ignore_directories: bool,
    plaintext: &mut Vec<PlaintextArtifact>,
    issues: &mut Vec<String>,
) {
    let path = entry.path();
    let entry_type = match entry.file_type() {
        Ok(file_type) => file_type,
        Err(error) => {
            issues.push(format!(
                "cannot determine file type for {}: {error}",
                path.display()
            ));
            return;
        }
    };
    if entry_type.is_symlink() {
        issues.push(format!(
            "refusing to inspect symbolic-link artifact {}",
            path.display()
        ));
        return;
    }
    if entry_type.is_dir() && ignore_directories {
        return;
    }
    if !entry_type.is_file() {
        issues.push(format!(
            "artifact candidate {} is not a regular file",
            path.display()
        ));
        return;
    }
    let path_metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) => {
            issues.push(format!(
                "cannot inspect metadata for {}: {error}",
                path.display()
            ));
            return;
        }
    };
    if path_metadata.file_type().is_symlink() || !path_metadata.is_file() {
        issues.push(format!(
            "artifact candidate {} changed before it could be inspected safely",
            path.display()
        ));
        return;
    }
    match inspect_regular_file(&path, &path_metadata) {
        Ok((FileHeader::Plaintext, size_bytes)) => {
            plaintext.push(PlaintextArtifact { path, size_bytes });
        }
        Ok((FileHeader::NonPlaintext, _)) => {}
        Err(error) => issues.push(error),
    }
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
    match inspect_regular_file(db_path, &path_metadata) {
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
    file.read_exact(&mut header).map_err(|error| {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            format!(
                "cannot confirm {} because it is shorter than the SQLite header",
                path.display()
            )
        } else {
            format!("cannot read header from {}: {error}", path.display())
        }
    })?;
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
    let header = if &header == SQLITE_PLAINTEXT_HEADER {
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
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

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
    fn scans_custom_root_sqlite_without_treating_arbitrary_files_as_candidates() {
        let dir = temp_dir("custom-root");
        write_non_plaintext_db(&dir.join("remem.db"));
        write_plaintext_db(&dir.join("pre-encryption-backup"));
        fs::write(dir.join("truncated.sqlite"), b"short").unwrap();
        fs::write(dir.join(".key"), b"short").unwrap();
        fs::write(dir.join("remem.log"), b"short").unwrap();
        write_non_plaintext_db(&dir.join("unrelated.txt"));
        write_non_plaintext_db(&dir.join("noise.sqlite"));
        #[cfg(unix)]
        let unreadable = {
            let path = dir.join("unreadable.sqlite");
            write_plaintext_db(&path);
            fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
            path
        };
        let result = check(&dir, true);
        assert_eq!(result.status, Status::Fail);
        let detail = result.detail.as_str();
        for expected in ["pre-encryption-backup", "truncated.sqlite", "shorter than"] {
            assert!(detail.contains(expected), "{detail}");
        }
        #[cfg(unix)]
        assert!(detail.contains("cannot open") && detail.contains("unreadable.sqlite"));
        for excluded in [".key", "remem.log", "unrelated.txt", "noise.sqlite"] {
            assert!(!detail.contains(excluded), "{detail}");
        }
        #[cfg(unix)]
        fs::set_permissions(unreadable, fs::Permissions::from_mode(0o600)).unwrap();
        cleanup(&dir);
    }

    #[test]
    fn backup_scan_is_not_recursive() {
        let dir = temp_dir("nonrecursive");
        write_non_plaintext_db(&dir.join("remem.db"));
        let nested = dir.join("backups").join("nested");
        fs::create_dir_all(&nested).unwrap();
        write_plaintext_db(&nested.join("plaintext.sqlite"));
        let result = check(&dir, true);
        assert_eq!(result.status, Status::Ok);
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
        assert!(detail.contains("`remem doctor` until"));
        assert!(detail.contains("do not treat any `remem admin backup` output as encrypted"));
        assert!(detail.contains("After confirmation, run `remem admin backup`"));
        assert!(detail.contains("did not delete any files"));
        assert!(detail.contains("alone does not protect"));
        cleanup(&dir);
    }

    #[test]
    fn plaintext_live_db_preserves_encrypt_blockers_before_encryption() {
        let dir = temp_dir("live-plaintext-blockers");
        for name in ["remem.db", "remem.db.bak", "remem.db.enc"] {
            write_plaintext_db(&dir.join(name));
        }
        let result = check(&dir, true);
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
        for shell_fragment in ["export ", "mkdir -p", "mktemp", "mv \"", "[ -L "] {
            assert!(
                !result.detail.contains(shell_fragment),
                "{detail}",
                detail = result.detail
            );
        }
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
    fn short_artifact_is_an_incomplete_inspection_warning() {
        let dir = temp_dir("short-artifact");
        write_non_plaintext_db(&dir.join("remem.db"));
        fs::write(dir.join("remem.db.bak"), b"short").unwrap();
        let result = check(&dir, true);
        assert_eq!(result.status, Status::Warn);
        assert!(result.detail.contains("shorter than the SQLite header"));
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
        write_non_plaintext_db(&dir.join("remem.db"));
        fs::create_dir(dir.join("targets")).unwrap();
        let target = dir.join("targets/target.sqlite");
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
