#[cfg(not(windows))]
use std::fs::{File, OpenOptions};
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};

#[cfg(windows)]
use crate::retrieval::embedding::local_semantic::{
    windows_cleanup,
    windows_security::{self, AnchoredFile, DirectoryAnchor},
};

#[cfg(windows)]
pub(super) type CacheLock = AnchoredFile;
#[cfg(not(windows))]
pub(super) type CacheLock = File;

pub(super) fn exclusive_lock(parent: &Path, name: &str) -> Result<CacheLock> {
    let file = open_lock_file(parent, name)?;
    fs2::FileExt::lock_exclusive(&file)
        .with_context(|| format!("lock private runtime cache exclusively {}", name))?;
    Ok(file)
}

pub(super) fn shared_lock(parent: &Path, name: &str) -> Result<CacheLock> {
    let file = open_lock_file(parent, name)?;
    fs2::FileExt::lock_shared(&file)
        .with_context(|| format!("lock private runtime cache for use {}", name))?;
    Ok(file)
}

pub(super) fn try_exclusive_lock(parent: &Path, name: &str) -> Result<Option<CacheLock>> {
    let file = open_lock_file(parent, name)?;
    match fs2::FileExt::try_lock_exclusive(&file) {
        Ok(()) => Ok(Some(file)),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
        Err(error) => {
            Err(error).with_context(|| format!("try exclusive private runtime cache lock {}", name))
        }
    }
}

pub(super) fn usage_lock_name(cache_name: &str) -> String {
    format!(".{cache_name}.usage.lock")
}

fn lock_path(parent: &Path, name: &str) -> PathBuf {
    parent.join(name)
}

pub(super) fn release_and_remove_usage_lock(
    parent: &Path,
    cache_name: &str,
    lock: CacheLock,
) -> Result<()> {
    #[cfg(windows)]
    let expected = windows_security::lock_file_identity(&lock)
        .with_context(|| format!("identify private runtime cache lock {cache_name}"))?;
    fs2::FileExt::unlock(&lock)
        .with_context(|| format!("unlock stale private runtime cache {cache_name}"))?;
    let path = lock_path(parent, &usage_lock_name(cache_name));
    drop(lock);
    #[cfg(windows)]
    return windows_cleanup::remove_owner_only_file(&path, expected);
    #[cfg(not(windows))]
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("remove private runtime usage lock {}", path.display())),
    }
}

fn open_lock_file(parent: &Path, name: &str) -> Result<CacheLock> {
    validate_lock_name(name)?;
    let canonical_parent = std::fs::canonicalize(parent)
        .with_context(|| format!("canonicalize lock parent {}", parent.display()))?;
    if canonical_parent != parent {
        bail!(
            "private runtime lock parent is not canonical: expected {}, got {}",
            parent.display(),
            canonical_parent.display()
        );
    }
    #[cfg(windows)]
    let parent_anchor = DirectoryAnchor::open_owner_only(parent, false)
        .with_context(|| format!("anchor private runtime lock parent {}", parent.display()))?;
    let path = parent.join(name);
    #[cfg(not(windows))]
    let file = {
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        options
            .open(&path)
            .with_context(|| format!("open private runtime lock {}", path.display()))?
    };
    #[cfg(windows)]
    let file = windows_security::open_or_create_owner_only_lock_file(&path)
        .with_context(|| format!("open secure private runtime lock {}", path.display()))?;
    let path_metadata = std::fs::symlink_metadata(&path)
        .with_context(|| format!("stat private runtime lock {}", path.display()))?;
    if !path_metadata.file_type().is_file() {
        bail!(
            "private runtime lock is not a regular file: {}",
            path.display()
        );
    }
    let canonical_path = std::fs::canonicalize(&path)
        .with_context(|| format!("canonicalize private runtime lock {}", path.display()))?;
    if canonical_path != path || !canonical_path.starts_with(parent) {
        bail!(
            "private runtime lock escapes cache parent: {}",
            canonical_path.display()
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let handle_metadata = file
            .metadata()
            .with_context(|| format!("stat private runtime lock handle {}", path.display()))?;
        if handle_metadata.dev() != path_metadata.dev()
            || handle_metadata.ino() != path_metadata.ino()
        {
            bail!(
                "private runtime lock changed while opening: {}",
                path.display()
            );
        }
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("secure private runtime lock {}", path.display()))?;
    }
    #[cfg(windows)]
    windows_security::validate_lock_file(&path, &file)
        .with_context(|| format!("verify Windows private runtime lock {}", path.display()))?;
    #[cfg(windows)]
    let file = AnchoredFile::new(file, parent_anchor);
    Ok(file)
}

fn validate_lock_name(name: &str) -> Result<()> {
    let mut components = Path::new(name).components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        bail!("invalid private runtime lock name {name}");
    }
    Ok(())
}
