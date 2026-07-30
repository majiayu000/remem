use std::path::Path;

use anyhow::{bail, Context, Result};

pub(super) fn remove_managed_tree(managed_root: &Path, target: &Path) -> Result<()> {
    let canonical_root = std::fs::canonicalize(managed_root)
        .with_context(|| format!("canonicalize managed root {}", managed_root.display()))?;
    let target_name = target
        .file_name()
        .context("managed cleanup target must have a file name")?;
    let expected_target = canonical_root.join(target_name);
    let target_parent = target
        .parent()
        .context("managed cleanup target must have a parent")?;
    let canonical_parent = std::fs::canonicalize(target_parent).with_context(|| {
        format!(
            "canonicalize managed target parent {}",
            target_parent.display()
        )
    })?;
    if canonical_parent != canonical_root {
        bail!(
            "refusing to remove tree outside managed root {}: {}",
            canonical_root.display(),
            target.display()
        );
    }
    let metadata = match std::fs::symlink_metadata(&expected_target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("stat managed tree {}", expected_target.display()));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        bail!(
            "managed cleanup target is not a real directory: {}",
            expected_target.display()
        );
    }
    #[cfg(windows)]
    clear_windows_readonly_tree(&canonical_root, &expected_target)?;
    std::fs::remove_dir_all(&expected_target)
        .with_context(|| format!("remove managed tree {}", expected_target.display()))
}

#[cfg(windows)]
fn clear_windows_readonly_tree(managed_root: &Path, path: &Path) -> Result<()> {
    let metadata =
        std::fs::symlink_metadata(path).with_context(|| format!("stat {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        bail!(
            "refusing to traverse symlink in managed cleanup tree: {}",
            path.display()
        );
    }
    if metadata.file_type().is_dir() {
        let canonical = std::fs::canonicalize(path)
            .with_context(|| format!("canonicalize managed directory {}", path.display()))?;
        if canonical != path || !canonical.starts_with(managed_root) {
            bail!(
                "managed cleanup directory escapes root {}: {}",
                managed_root.display(),
                canonical.display()
            );
        }
        for entry in std::fs::read_dir(path)
            .with_context(|| format!("read managed tree {}", path.display()))?
        {
            clear_windows_readonly_tree(managed_root, &entry?.path())?;
        }
        return Ok(());
    }
    if !metadata.file_type().is_file() {
        bail!(
            "refusing to remove special file from managed cleanup tree: {}",
            path.display()
        );
    }
    let canonical = std::fs::canonicalize(path)
        .with_context(|| format!("canonicalize managed file {}", path.display()))?;
    if canonical != path || !canonical.starts_with(managed_root) {
        bail!(
            "managed cleanup file escapes root {}: {}",
            managed_root.display(),
            canonical.display()
        );
    }
    if metadata.permissions().readonly() {
        let mut permissions = metadata.permissions();
        permissions.set_readonly(false);
        std::fs::set_permissions(path, permissions)
            .with_context(|| format!("clear readonly managed file {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_cleanup_removes_readonly_regular_tree() -> Result<()> {
        let root = test_root("readonly");
        std::fs::create_dir(&root)?;
        let target = root.join("managed");
        std::fs::create_dir(&target)?;
        let file = target.join("artifact");
        std::fs::write(&file, b"verified artifact")?;
        let mut permissions = std::fs::metadata(&file)?.permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&file, permissions)?;

        remove_managed_tree(&root, &target)?;

        assert!(!target.exists());
        std::fs::remove_dir(&root)?;
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn managed_cleanup_rejects_target_symlink_without_touching_referent() -> Result<()> {
        use std::os::unix::fs::symlink;

        let root = test_root("symlink");
        std::fs::create_dir(&root)?;
        let referent = root.join("referent");
        std::fs::create_dir(&referent)?;
        std::fs::write(referent.join("sentinel"), b"keep")?;
        let target = root.join("managed");
        symlink(&referent, &target)?;

        let error = remove_managed_tree(&root, &target).unwrap_err();

        assert!(error.to_string().contains("not a real directory"));
        assert_eq!(std::fs::read(referent.join("sentinel"))?, b"keep");
        std::fs::remove_file(&target)?;
        std::fs::remove_dir_all(&root)?;
        Ok(())
    }

    fn test_root(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "remem-managed-cleanup-{label}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ))
    }
}
