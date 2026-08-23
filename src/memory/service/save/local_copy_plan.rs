use anyhow::{anyhow, ensure, Context, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::super::local_copy::{
    build_local_note_content, local_copy_enabled_override, resolve_local_note_path,
    write_local_note,
};
use super::super::types::{LocalCopyResult, SaveMemoryRequest};
use crate::memory::activation::SupplementalLocalCopyReceipt;

pub(super) struct LocalCopyPlan {
    status: String,
    path: Option<PathBuf>,
    reason: Option<String>,
    content: Option<String>,
    saved_at: Option<String>,
    backup: Option<LocalCopyBackup>,
    written: bool,
    exact_replay: bool,
}

impl LocalCopyPlan {
    pub(super) fn result(&self) -> LocalCopyResult {
        LocalCopyResult {
            status: self.status.clone(),
            path: self.path.as_ref().map(|path| path.display().to_string()),
            reason: self.reason.clone(),
        }
    }

    pub(super) fn receipt(&mut self) -> Result<SupplementalLocalCopyReceipt> {
        match self.status.as_str() {
            "disabled" => Ok(SupplementalLocalCopyReceipt::Disabled),
            "saved" => {
                let canonical_path = self
                    .path
                    .as_ref()
                    .context("saved local-copy plan is missing its path")?
                    .canonicalize()
                    .context("canonicalize saved local-copy receipt path")?;
                let path = canonical_path
                    .to_str()
                    .context("saved local-copy receipt path must be valid UTF-8")?
                    .to_string();
                let content = self
                    .content
                    .as_deref()
                    .context("saved local-copy plan is missing its content")?;
                let saved_at = self
                    .saved_at
                    .as_deref()
                    .context("saved local-copy plan is missing its render timestamp")?;
                let receipt = SupplementalLocalCopyReceipt::saved(
                    path,
                    saved_at,
                    file_content_sha256(content.as_bytes()),
                )?;
                self.path = Some(canonical_path);
                Ok(receipt)
            }
            status => anyhow::bail!("unsupported local-copy plan status: {status}"),
        }
    }
}

struct LocalCopyBackup {
    restore_path: PathBuf,
    backup_path: PathBuf,
}

pub(super) fn prepare_local_copy(
    project: &str,
    title: &str,
    req: &SaveMemoryRequest,
) -> Result<LocalCopyPlan> {
    if !local_copy_enabled_override(req.local_copy_enabled) {
        return Ok(disabled_plan());
    }

    let path = resolve_local_note_path(project, req.title.as_deref(), req.local_path.as_deref())?;
    let saved_at = chrono::Utc::now().to_rfc3339();
    let content = build_local_note_content(project, title, &req.text, &saved_at);
    Ok(LocalCopyPlan {
        status: "saved".to_string(),
        path: Some(path),
        reason: None,
        content: Some(content),
        saved_at: Some(saved_at),
        backup: None,
        written: false,
        exact_replay: false,
    })
}

pub(super) fn replay_local_copy(
    project: &str,
    title: &str,
    text: &str,
    receipt: &SupplementalLocalCopyReceipt,
) -> Result<LocalCopyPlan> {
    match receipt {
        SupplementalLocalCopyReceipt::LegacyUnknown => Ok(LocalCopyPlan {
            status: "unknown".to_string(),
            path: None,
            reason: Some(
                "legacy activation receipt did not record the local-copy outcome".to_string(),
            ),
            content: None,
            saved_at: None,
            backup: None,
            written: false,
            exact_replay: false,
        }),
        SupplementalLocalCopyReceipt::Disabled => Ok(disabled_plan()),
        SupplementalLocalCopyReceipt::Saved {
            path,
            saved_at,
            sha256: expected_sha256,
        } => {
            let confined_path = validate_receipt_path(project, title, path)?;
            let content = build_local_note_content(project, title, text, saved_at);
            ensure!(
                file_content_sha256(content.as_bytes()) == *expected_sha256,
                "local-copy replay content does not match its immutable receipt digest"
            );
            let mut plan = LocalCopyPlan {
                status: "saved".to_string(),
                path: Some(confined_path),
                reason: None,
                content: Some(content),
                saved_at: Some(saved_at.clone()),
                backup: None,
                written: false,
                exact_replay: true,
            };
            let path = plan
                .path
                .as_deref()
                .context("replayed local-copy plan is missing its path")?;
            match read_replayed_local_copy(path)? {
                Some(bytes) if file_content_sha256(&bytes) == *expected_sha256 => Ok(plan),
                Some(_) | None => {
                    write_replayed_local_copy(&mut plan)?;
                    Ok(plan)
                }
            }
        }
    }
}

#[cfg(unix)]
fn read_replayed_local_copy(path: &Path) -> Result<Option<Vec<u8>>> {
    secure_replay::read_no_follow(path)
}

#[cfg(not(unix))]
fn read_replayed_local_copy(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => {
            Err(error).with_context(|| format!("read replayed local copy at {}", path.display()))
        }
    }
}

fn write_replayed_local_copy(local_copy: &mut LocalCopyPlan) -> Result<()> {
    let path = local_copy
        .path
        .as_deref()
        .context("replayed local-copy plan is missing its path")?;
    let content = local_copy
        .content
        .as_deref()
        .context("replayed local-copy plan is missing its content")?;
    #[cfg(unix)]
    secure_replay::atomic_write_no_follow(path, content.as_bytes())?;
    #[cfg(not(unix))]
    anyhow::bail!(
        "exact local-copy repair is unavailable on this platform; refusing a path-based replay write"
    );
    local_copy.written = true;
    Ok(())
}

fn disabled_plan() -> LocalCopyPlan {
    LocalCopyPlan {
        status: "disabled".to_string(),
        path: None,
        reason: Some("local copy disabled by request or configuration".to_string()),
        content: None,
        saved_at: None,
        backup: None,
        written: false,
        exact_replay: false,
    }
}

fn validate_receipt_path(project: &str, title: &str, stored: &str) -> Result<PathBuf> {
    let stored_path = PathBuf::from(stored);
    let confined_path = resolve_local_note_path(project, Some(title), Some(stored))?;
    ensure!(
        confined_path == stored_path,
        "local-copy receipt path resolution has drifted"
    );
    if let Ok(metadata) = std::fs::symlink_metadata(&stored_path) {
        ensure!(
            !metadata.file_type().is_symlink(),
            "local-copy receipt path became a symlink"
        );
    }
    Ok(stored_path)
}

pub(super) fn write_local_copy(local_copy: &mut LocalCopyPlan) -> Result<()> {
    if let (Some(path), Some(content)) = (local_copy.path.as_deref(), local_copy.content.as_deref())
    {
        if local_copy.exact_replay {
            let stored = path
                .to_str()
                .context("replayed local-copy path must remain valid UTF-8")?;
            // Re-resolve immediately before replacement so a leaf symlink added
            // after the first replay check cannot redirect the write.
            validate_receipt_path("replay", "replay", stored)?;
        }
        let backup = backup_existing_local_copy(path, !local_copy.exact_replay)?;
        if let Err(error) = write_local_note(path, content) {
            let cleanup = match backup.as_ref() {
                Some(backup) => restore_local_copy(Some(backup)),
                None => remove_local_copy_file(path),
            };
            if let Err(cleanup_error) = cleanup {
                return Err(error.context(format!(
                    "write local copy failed and cleanup failed: {cleanup_error}"
                )));
            }
            return Err(error);
        }
        local_copy.backup = backup;
        local_copy.written = true;
    }
    Ok(())
}

pub(super) fn cleanup_local_copy(local_copy: &LocalCopyPlan) -> Result<()> {
    if !local_copy.written {
        return Ok(());
    }
    restore_local_copy(local_copy.backup.as_ref())?;
    match (local_copy.path.as_deref(), local_copy.backup.as_ref()) {
        (Some(path), None) => remove_local_copy_file(path),
        _ => Ok(()),
    }
}

fn backup_existing_local_copy(
    path: &Path,
    allow_existing_symlink: bool,
) -> Result<Option<LocalCopyBackup>> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() && !allow_existing_symlink {
                anyhow::bail!("local-copy receipt path became a symlink");
            }
            let restore_path = backup_restore_path(path, &metadata)?;
            let backup_path = allocate_backup_path(&restore_path);
            std::fs::rename(&restore_path, &backup_path).with_context(|| {
                format!(
                    "move existing local copy {} to backup {}",
                    restore_path.display(),
                    backup_path.display()
                )
            })?;
            Ok(Some(LocalCopyBackup {
                restore_path,
                backup_path,
            }))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(anyhow!(
            "check existing local copy at {}: {error}",
            path.display()
        )),
    }
}

fn backup_restore_path(path: &Path, metadata: &std::fs::Metadata) -> Result<PathBuf> {
    let file_type = metadata.file_type();
    if file_type.is_dir() {
        return Err(anyhow!(
            "local_path {} must reference a file, not a directory",
            path.display()
        ));
    }

    if file_type.is_symlink() {
        let target_path = path
            .canonicalize()
            .with_context(|| format!("resolve local_path symlink target at {}", path.display()))?;
        if target_path.is_dir() {
            return Err(anyhow!(
                "local_path {} must reference a file, not a directory",
                path.display()
            ));
        }
        return Ok(target_path);
    }

    Ok(path.to_path_buf())
}

fn restore_local_copy(backup: Option<&LocalCopyBackup>) -> Result<()> {
    if let Some(backup) = backup {
        remove_local_copy_file(&backup.restore_path)?;
        std::fs::rename(&backup.backup_path, &backup.restore_path).with_context(|| {
            format!(
                "restore local copy from backup {} to {}",
                backup.backup_path.display(),
                backup.restore_path.display()
            )
        })?;
    }
    Ok(())
}

fn remove_local_copy_file(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("remove local copy at {}", path.display()))
        }
    }
}

pub(super) fn discard_local_copy_backup(local_copy: &LocalCopyPlan) {
    if let Some(backup) = local_copy.backup.as_ref() {
        let _ = std::fs::remove_file(&backup.backup_path);
    }
}

fn allocate_backup_path(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("local-copy");
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    parent.join(format!(
        ".{file_name}.remem-backup-{}-{timestamp}.tmp",
        std::process::id()
    ))
}

fn file_content_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(unix)]
mod secure_replay {
    use anyhow::{anyhow, bail, Context, Result};
    use std::ffi::{CString, OsStr};
    use std::io::{Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Component, Path};
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_NONCE: AtomicU64 = AtomicU64::new(1);

    pub(super) fn read_no_follow(path: &Path) -> Result<Option<Vec<u8>>> {
        let Some((parent, leaf)) = open_parent(path, false)? else {
            return Ok(None);
        };
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                leaf.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::NotFound {
                return Ok(None);
            }
            return Err(error).with_context(|| {
                format!(
                    "open replayed local copy at {} without following links",
                    path.display()
                )
            });
        }
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };
        ensure_regular_file(&owned, path)?;
        let mut file = std::fs::File::from(owned);
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .with_context(|| format!("read replayed local copy at {}", path.display()))?;
        Ok(Some(bytes))
    }

    pub(super) fn atomic_write_no_follow(path: &Path, bytes: &[u8]) -> Result<()> {
        let (parent, leaf) = open_parent(path, true)?
            .context("replayed local-copy parent reconstruction returned no directory")?;
        let temp = temp_name()?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                temp.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                0o600,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error()).with_context(|| {
                format!("create staged replay local copy for {}", path.display())
            });
        }
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };
        let mut published = false;
        let result = (|| -> Result<()> {
            ensure_regular_file(&owned, path)?;
            let mut file = std::fs::File::from(owned);
            file.write_all(bytes).with_context(|| {
                format!("write staged replay local copy for {}", path.display())
            })?;
            file.sync_all()
                .with_context(|| format!("sync staged replay local copy for {}", path.display()))?;
            drop(file);
            let renamed = unsafe {
                libc::renameat(
                    parent.as_raw_fd(),
                    temp.as_ptr(),
                    parent.as_raw_fd(),
                    leaf.as_ptr(),
                )
            };
            if renamed != 0 {
                return Err(std::io::Error::last_os_error())
                    .with_context(|| format!("publish replayed local copy at {}", path.display()));
            }
            published = true;
            let synced = unsafe { libc::fsync(parent.as_raw_fd()) };
            if synced != 0 {
                return Err(std::io::Error::last_os_error()).with_context(|| {
                    format!("sync replayed local copy directory for {}", path.display())
                });
            }
            Ok(())
        })();
        match result {
            Ok(()) => Ok(()),
            Err(error) if !published => {
                let removed = unsafe { libc::unlinkat(parent.as_raw_fd(), temp.as_ptr(), 0) };
                if removed != 0 {
                    let cleanup_error = std::io::Error::last_os_error();
                    if cleanup_error.kind() != std::io::ErrorKind::NotFound {
                        return Err(anyhow!(
                            "staged replay local-copy cleanup failed: {cleanup_error}; original staged replay failure: {error:#}"
                        ));
                    }
                }
                Err(error)
            }
            Err(error) => Err(error),
        }
    }

    fn open_parent(path: &Path, create_missing: bool) -> Result<Option<(OwnedFd, CString)>> {
        let parent = path
            .parent()
            .with_context(|| format!("local-copy path {} has no parent", path.display()))?;
        if !parent.is_absolute() {
            bail!("replayed local-copy path must be absolute");
        }
        let leaf = component_name(
            path.file_name()
                .context("replayed local-copy path has no file name")?,
        )?;
        let root = CString::new("/").expect("static root path has no NUL");
        let fd = unsafe {
            libc::open(
                root.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error()).context("open filesystem root");
        }
        let mut directory = unsafe { OwnedFd::from_raw_fd(fd) };
        for component in parent.components() {
            let Component::Normal(name) = component else {
                if matches!(component, Component::RootDir | Component::CurDir) {
                    continue;
                }
                bail!("replayed local-copy parent is not canonical");
            };
            let name = component_name(name)?;
            let mut next = open_directory_at(&directory, &name);
            if next < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
                if !create_missing {
                    return Ok(None);
                }
                let created = unsafe { libc::mkdirat(directory.as_raw_fd(), name.as_ptr(), 0o700) };
                if created != 0 {
                    let error = std::io::Error::last_os_error();
                    if error.kind() != std::io::ErrorKind::AlreadyExists {
                        return Err(error).with_context(|| {
                            format!(
                                "recreate replayed local-copy parent {} without following links",
                                parent.display()
                            )
                        });
                    }
                }
                next = open_directory_at(&directory, &name);
            }
            if next < 0 {
                return Err(std::io::Error::last_os_error()).with_context(|| {
                    format!(
                        "open replayed local-copy parent {} without following links",
                        parent.display()
                    )
                });
            }
            directory = unsafe { OwnedFd::from_raw_fd(next) };
        }
        Ok(Some((directory, leaf)))
    }

    fn open_directory_at(directory: &OwnedFd, name: &CString) -> i32 {
        unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        }
    }

    fn component_name(name: &OsStr) -> Result<CString> {
        CString::new(name.as_bytes()).map_err(|_| anyhow!("local-copy path component contains NUL"))
    }

    fn ensure_regular_file(fd: &OwnedFd, path: &Path) -> Result<()> {
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        let status = unsafe { libc::fstat(fd.as_raw_fd(), stat.as_mut_ptr()) };
        if status != 0 {
            return Err(std::io::Error::last_os_error())
                .with_context(|| format!("inspect replayed local copy at {}", path.display()));
        }
        let stat = unsafe { stat.assume_init() };
        if stat.st_mode & libc::S_IFMT != libc::S_IFREG {
            bail!("replayed local-copy path must reference a regular file");
        }
        Ok(())
    }

    fn temp_name() -> Result<CString> {
        let nonce = TEMP_NONCE.fetch_add(1, Ordering::Relaxed);
        CString::new(format!(
            ".remem-local-copy-{}-{}-{nonce}.tmp",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ))
        .context("build staged replay local-copy name")
    }

    #[cfg(test)]
    mod tests {
        use super::atomic_write_no_follow;
        use std::os::unix::fs::symlink;

        #[test]
        fn atomic_publication_replaces_leaf_symlink_without_following_target() -> anyhow::Result<()>
        {
            let root = std::env::temp_dir().join(format!(
                "remem-secure-replay-{}-{}",
                std::process::id(),
                chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
            ));
            std::fs::create_dir_all(&root)?;
            let root = root.canonicalize()?;
            let victim = root.join("victim.md");
            let receipt_path = root.join("receipt.md");
            std::fs::write(&victim, b"victim")?;
            symlink(&victim, &receipt_path)?;

            atomic_write_no_follow(&receipt_path, b"receipt")?;

            assert_eq!(std::fs::read(&victim)?, b"victim");
            assert_eq!(std::fs::read(&receipt_path)?, b"receipt");
            assert!(!std::fs::symlink_metadata(&receipt_path)?
                .file_type()
                .is_symlink());
            std::fs::remove_dir_all(root)?;
            Ok(())
        }
    }
}
