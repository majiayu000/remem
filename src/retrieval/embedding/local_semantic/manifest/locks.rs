use std::fs::File;
#[cfg(not(windows))]
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

#[cfg(windows)]
use crate::retrieval::embedding::local_semantic::windows_security::{
    self, AnchoredFile, DirectoryAnchor,
};

#[cfg(windows)]
pub(in crate::retrieval::embedding::local_semantic) type ModelLock = AnchoredFile;
#[cfg(not(windows))]
pub(in crate::retrieval::embedding::local_semantic) type ModelLock = File;

pub(in crate::retrieval::embedding::local_semantic) fn canonical_real_install_dir(
    install_dir: &Path,
) -> Result<PathBuf> {
    let metadata = std::fs::symlink_metadata(install_dir)
        .with_context(|| format!("stat local model install {}", install_dir.display()))?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        bail!(
            "local model install must be a real directory: {}",
            install_dir.display()
        );
    }
    std::fs::canonicalize(install_dir)
        .with_context(|| format!("canonicalize local model install {}", install_dir.display()))
}

pub(in crate::retrieval::embedding::local_semantic) fn open_or_create_model_lock(
    install_dir: &Path,
    lock_name: &str,
) -> Result<(PathBuf, ModelLock)> {
    let canonical_install_dir = canonical_real_install_dir(install_dir)?;
    #[cfg(windows)]
    let install_anchor = DirectoryAnchor::open_owner_only(&canonical_install_dir, false)
        .with_context(|| {
            format!(
                "validate and anchor owner-only local model install {}",
                canonical_install_dir.display()
            )
        })?;
    let lock_path = canonical_install_dir.join(lock_name);
    for _ in 0..4 {
        match std::fs::symlink_metadata(&lock_path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
                    bail!(
                        "local model lock is not a real file: {}",
                        lock_path.display()
                    );
                }
                let file = open_lock_file(&lock_path, false)?;
                validate_opened_lock(&canonical_install_dir, &lock_path, &file)?;
                #[cfg(windows)]
                let file = AnchoredFile::new(file, install_anchor);
                return Ok((lock_path, file));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match open_lock_file(&lock_path, true) {
                    Ok(file) => {
                        validate_opened_lock(&canonical_install_dir, &lock_path, &file)?;
                        #[cfg(windows)]
                        let file = AnchoredFile::new(file, install_anchor);
                        return Ok((lock_path, file));
                    }
                    Err(error)
                        if error.downcast_ref::<std::io::Error>().is_some_and(|error| {
                            error.kind() == std::io::ErrorKind::AlreadyExists
                        }) =>
                    {
                        continue;
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("stat local model lock {}", lock_path.display()));
            }
        }
    }
    bail!(
        "local model lock changed repeatedly while opening {}",
        lock_path.display()
    )
}

#[cfg(not(windows))]
fn open_lock_file(path: &Path, create_new: bool) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).truncate(false);
    if create_new {
        options.create_new(true);
    }
    configure_no_follow(&mut options);
    options
        .open(path)
        .with_context(|| format!("open local model lock {}", path.display()))
}

#[cfg(windows)]
fn open_lock_file(path: &Path, create_new: bool) -> Result<File> {
    let file = if create_new {
        windows_security::create_owner_only_lock_file(path)
    } else {
        windows_security::open_owner_only_lock_file(path)
    };
    file.with_context(|| format!("open secure local model lock {}", path.display()))
}

#[cfg(unix)]
fn configure_no_follow(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;

    options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
}

#[cfg(not(any(unix, windows)))]
fn configure_no_follow(_options: &mut OpenOptions) {}

fn validate_opened_lock(install_dir: &Path, path: &Path, file: &File) -> Result<()> {
    let path_metadata =
        std::fs::symlink_metadata(path).with_context(|| format!("stat {}", path.display()))?;
    let file_metadata = file
        .metadata()
        .with_context(|| format!("stat open lock {}", path.display()))?;
    if path_metadata.file_type().is_symlink()
        || !path_metadata.file_type().is_file()
        || !file_metadata.file_type().is_file()
    {
        bail!("local model lock is not a real file: {}", path.display());
    }
    let canonical = std::fs::canonicalize(path)
        .with_context(|| format!("canonicalize local model lock {}", path.display()))?;
    if canonical != path || !canonical.starts_with(install_dir) {
        bail!(
            "local model lock escapes install directory {}: {}",
            install_dir.display(),
            canonical.display()
        );
    }
    validate_lock_identity(path, file, &path_metadata, &file_metadata)
}

#[cfg(unix)]
fn validate_lock_identity(
    path: &Path,
    _file: &File,
    path_metadata: &std::fs::Metadata,
    file_metadata: &std::fs::Metadata,
) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    if path_metadata.dev() != file_metadata.dev() || path_metadata.ino() != file_metadata.ino() {
        bail!("local model lock changed while opening: {}", path.display());
    }
    Ok(())
}

#[cfg(windows)]
fn validate_lock_identity(
    path: &Path,
    file: &File,
    _path_metadata: &std::fs::Metadata,
    file_metadata: &std::fs::Metadata,
) -> Result<()> {
    if !file_metadata.file_type().is_file() {
        bail!("local model lock handle is not a file: {}", path.display());
    }
    windows_security::validate_lock_file(path, file)
        .with_context(|| format!("verify Windows local model lock {}", path.display()))
}

#[cfg(not(any(unix, windows)))]
fn validate_lock_identity(
    _path: &Path,
    _file: &File,
    _path_metadata: &std::fs::Metadata,
    _file_metadata: &std::fs::Metadata,
) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn exact_install_symlink_is_rejected_before_lock_creation() -> Result<()> {
        use std::os::unix::fs::symlink;

        let root = test_root("install-symlink");
        let referent = root.join("referent");
        let install_link = root.join("install");
        std::fs::create_dir_all(&referent)?;
        symlink(&referent, &install_link)?;

        let error = open_or_create_model_lock(&install_link, ".test-model.lock").unwrap_err();

        assert!(error.to_string().contains("real directory"), "{error:#}");
        assert!(!referent.join(".test-model.lock").exists());
        std::fs::remove_file(&install_link)?;
        std::fs::remove_dir_all(&root)?;
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn preseeded_lock_symlink_is_rejected_without_touching_referent() -> Result<()> {
        use std::os::unix::fs::symlink;

        let root = test_root("lock-symlink");
        let install_dir = root.join("install");
        std::fs::create_dir_all(&install_dir)?;
        let victim = root.join("victim");
        std::fs::write(&victim, b"outside-sentinel")?;
        let lock_path = install_dir.join(".test-model.lock");
        symlink(&victim, &lock_path)?;

        let error = open_or_create_model_lock(&install_dir, ".test-model.lock").unwrap_err();

        assert!(error.to_string().contains("real file"), "{error:#}");
        assert_eq!(std::fs::read(&victim)?, b"outside-sentinel");
        std::fs::remove_file(&lock_path)?;
        std::fs::remove_dir_all(&root)?;
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn newly_created_lock_is_owner_only() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let root = test_root("lock-mode");
        let install_dir = root.join("install");
        std::fs::create_dir_all(&install_dir)?;

        let (path, file) = open_or_create_model_lock(&install_dir, ".test-model.lock")?;

        assert_eq!(file.metadata()?.permissions().mode() & 0o777, 0o600);
        drop(file);
        std::fs::remove_file(path)?;
        std::fs::remove_dir_all(&root)?;
        Ok(())
    }

    fn test_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "remem-model-lock-{label}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ))
    }
}
