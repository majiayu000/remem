use std::fs::File;
use std::os::windows::io::AsRawHandle;
use std::path::Path;

use anyhow::{bail, Context, Result};
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Storage::FileSystem::{
    FileDispositionInfo, SetFileInformationByHandle, FILE_DISPOSITION_INFO,
};

use super::fs_cleanup::remove_managed_tree;
use super::windows_security::{self, DirectoryAnchor, WindowsFileId};

pub(super) fn remove_created_staging(path: &Path, anchor: DirectoryAnchor) -> Result<()> {
    remove_anchored_tree(path, anchor)
}

pub(super) fn remove_existing_owner_only_tree(path: &Path, anchor: DirectoryAnchor) -> Result<()> {
    let expected = anchor.identity();
    anchor
        .verify_path()
        .with_context(|| format!("verify existing owner-only Windows tree {}", path.display()))?;
    drop(anchor);
    let cleanup = DirectoryAnchor::open_owner_only_for_cleanup(path, false)
        .with_context(|| format!("reopen owner-only Windows tree {}", path.display()))?;
    if cleanup.identity() != expected {
        bail!(
            "owner-only Windows tree identity changed before cleanup: {}",
            path.display()
        );
    }
    remove_anchored_tree(path, cleanup)
}

pub(super) fn remove_renameable_staging(path: &Path, anchor: DirectoryAnchor) -> Result<()> {
    let expected = anchor.identity();
    anchor.verify_path().with_context(|| {
        format!(
            "verify renameable Windows staging identity {}",
            path.display()
        )
    })?;
    let cleanup = DirectoryAnchor::open_owner_only_for_cleanup(path, false)
        .with_context(|| format!("reopen Windows staging for cleanup {}", path.display()))?;
    if cleanup.identity() != expected {
        bail!(
            "Windows staging identity changed before cleanup: {}",
            path.display()
        );
    }
    drop(anchor);
    remove_anchored_tree(path, cleanup)
}

pub(super) fn remove_stale_staging(path: &Path) -> Result<()> {
    let cleanup = DirectoryAnchor::open_owner_only_for_cleanup(path, false)
        .with_context(|| format!("validate stale Windows staging {}", path.display()))?;
    remove_anchored_tree(path, cleanup)
}

pub(super) fn publish_identity_handoff(
    staging_path: &Path,
    published_path: &Path,
    staging: DirectoryAnchor,
) -> Result<DirectoryAnchor> {
    let expected = staging.identity();
    staging
        .verify_path()
        .with_context(|| format!("verify Windows staging {}", staging_path.display()))?;
    std::fs::rename(staging_path, published_path).with_context(|| {
        format!(
            "atomically publish private local embedding runtime cache {}",
            published_path.display()
        )
    })?;
    let bridge = DirectoryAnchor::open_owner_only(published_path, true).with_context(|| {
        format!(
            "open Windows publish identity bridge {}",
            published_path.display()
        )
    })?;
    if bridge.identity() != expected {
        bail!(
            "private runtime cache identity changed during publish: {}",
            published_path.display()
        );
    }
    let published = DirectoryAnchor::open_owner_only(published_path, false).with_context(|| {
        format!(
            "anchor published private runtime cache {}",
            published_path.display()
        )
    })?;
    if published.identity() != expected || bridge.identity() != published.identity() {
        bail!(
            "private runtime cache identity changed during anchor handoff: {}",
            published_path.display()
        );
    }
    drop(staging);
    bridge.verify_path()?;
    published.verify_path()?;
    Ok(published)
}

pub(super) fn remove_owner_only_file(path: &Path, expected: WindowsFileId) -> Result<()> {
    let cleanup = match windows_security::open_owner_only_lock_file_for_cleanup(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("reopen Windows lock for cleanup {}", path.display()))
        }
    };
    if windows_security::lock_file_identity(&cleanup)? != expected {
        bail!(
            "Windows lock identity changed before cleanup: {}",
            path.display()
        );
    }
    mark_delete(&cleanup)
        .with_context(|| format!("delete Windows lock by handle {}", path.display()))?;
    drop(cleanup);
    verify_removed(path, "Windows lock")
}

fn remove_anchored_tree(path: &Path, anchor: DirectoryAnchor) -> Result<()> {
    anchor
        .verify_path()
        .with_context(|| format!("verify Windows staging identity {}", path.display()))?;
    clear_contents(path)?;
    anchor
        .verify_path()
        .with_context(|| format!("revalidate Windows staging identity {}", path.display()))?;
    mark_delete(anchor.file())
        .with_context(|| format!("delete Windows staging by handle {}", path.display()))?;
    drop(anchor);
    verify_removed(path, "Windows staging")
}

fn mark_delete(file: &File) -> std::io::Result<()> {
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    // SAFETY: the live handle was opened with DELETE access and the fixed-size
    // input buffer remains valid for the duration of this call.
    if unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle() as HANDLE,
            FileDispositionInfo,
            (&disposition as *const FILE_DISPOSITION_INFO).cast(),
            std::mem::size_of_val(&disposition) as u32,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn verify_removed(path: &Path, label: &str) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => bail!(
            "{label} remained after handle-bound cleanup: {}",
            path.display()
        ),
        Err(error) => {
            Err(error).with_context(|| format!("verify removed {label} {}", path.display()))
        }
    }
}

fn clear_contents(path: &Path) -> Result<()> {
    for entry in std::fs::read_dir(path)
        .with_context(|| format!("read Windows staging {}", path.display()))?
    {
        let child = entry?.path();
        let metadata = std::fs::symlink_metadata(&child)
            .with_context(|| format!("stat Windows staging child {}", child.display()))?;
        if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
            remove_managed_tree(path, &child)?;
        } else if metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            if metadata.permissions().readonly() {
                let mut permissions = metadata.permissions();
                permissions.set_readonly(false);
                std::fs::set_permissions(&child, permissions)?;
            }
            std::fs::remove_file(&child)
                .with_context(|| format!("remove Windows staging child {}", child.display()))?;
        } else {
            bail!(
                "refusing to remove special Windows staging child {}",
                child.display()
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use crate::retrieval::embedding::local_semantic::windows_security;

    #[test]
    fn handle_bound_cleanup_removes_the_anchored_tree() -> Result<()> {
        let root = test_root("remove");
        std::fs::create_dir(&root)?;
        let path = root.join("staging");
        let anchor = windows_security::create_owner_only_cleanup_directory(&path, false)?;
        std::fs::create_dir(path.join("nested"))?;
        std::fs::write(path.join("nested/payload"), b"payload")?;

        remove_created_staging(&path, anchor)?;

        assert!(!path.exists());
        std::fs::remove_dir(&root)?;
        Ok(())
    }

    #[test]
    fn renameable_cleanup_hands_off_to_a_delete_anchor() -> Result<()> {
        let root = test_root("renameable-remove");
        std::fs::create_dir(&root)?;
        let path = root.join("staging");
        let anchor = windows_security::create_owner_only_directory(&path, true)?;
        std::fs::write(path.join("payload"), b"payload")?;

        remove_renameable_staging(&path, anchor)?;

        assert!(!path.exists());
        std::fs::remove_dir(&root)?;
        Ok(())
    }

    #[test]
    fn identity_mismatch_preserves_the_replacement() -> Result<()> {
        let root = test_root("replacement");
        std::fs::create_dir(&root)?;
        let path = root.join("staging");
        let parked = root.join("parked");
        let original = windows_security::create_owner_only_cleanup_directory(&path, true)?;
        std::fs::rename(&path, &parked)?;
        let replacement = windows_security::create_owner_only_cleanup_directory(&path, false)?;
        std::fs::write(path.join("sentinel"), b"keep")?;

        let error = remove_renameable_staging(&path, original).unwrap_err();

        assert!(error.to_string().contains("identity"), "{error:#}");
        assert_eq!(std::fs::read(path.join("sentinel"))?, b"keep");
        drop(replacement);
        std::fs::remove_dir_all(&path)?;
        std::fs::remove_dir_all(&parked)?;
        std::fs::remove_dir(&root)?;
        Ok(())
    }

    #[test]
    fn handle_bound_lock_cleanup_removes_only_the_expected_file() -> Result<()> {
        let root = test_root("lock-remove");
        std::fs::create_dir(&root)?;
        let path = root.join("usage.lock");
        let file = windows_security::create_owner_only_lock_file(&path)?;
        let expected = windows_security::lock_file_identity(&file)?;
        drop(file);

        remove_owner_only_file(&path, expected)?;

        assert!(!path.exists());
        std::fs::remove_dir(&root)?;
        Ok(())
    }

    #[test]
    fn lock_identity_mismatch_preserves_the_replacement() -> Result<()> {
        let root = test_root("lock-replacement");
        std::fs::create_dir(&root)?;
        let path = root.join("usage.lock");
        let parked = root.join("parked.lock");
        let original = windows_security::create_owner_only_lock_file(&path)?;
        let expected = windows_security::lock_file_identity(&original)?;
        drop(original);
        std::fs::rename(&path, &parked)?;
        let mut replacement = windows_security::create_owner_only_lock_file(&path)?;
        replacement.write_all(b"keep")?;
        drop(replacement);

        let error = remove_owner_only_file(&path, expected).unwrap_err();

        assert!(error.to_string().contains("identity"), "{error:#}");
        assert_eq!(std::fs::read(&path)?, b"keep");
        std::fs::remove_file(&path)?;
        std::fs::remove_file(&parked)?;
        std::fs::remove_dir(&root)?;
        Ok(())
    }

    fn test_root(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "remem-windows-cleanup-{label}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ))
    }
}
