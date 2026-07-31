use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{bail, Context, Result};

#[cfg(not(windows))]
use crate::retrieval::embedding::local_semantic::fs_cleanup::remove_managed_tree;
#[cfg(windows)]
use crate::retrieval::embedding::local_semantic::{
    windows_cleanup,
    windows_security::{self, DirectoryAnchor},
};

const STAGING_PREFIX: &str = ".remem-download-staging.";
static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub(in crate::retrieval::embedding::local_semantic) struct DownloadStaging {
    #[cfg(not(windows))]
    install_dir: PathBuf,
    path: PathBuf,
    #[cfg(windows)]
    staging_anchor: Option<DirectoryAnchor>,
    #[cfg(windows)]
    _install_anchor: DirectoryAnchor,
    armed: bool,
}

impl DownloadStaging {
    pub(in crate::retrieval::embedding::local_semantic) fn create(
        install_dir: &Path,
    ) -> Result<Self> {
        let metadata = std::fs::symlink_metadata(install_dir)
            .with_context(|| format!("stat local model install {}", install_dir.display()))?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
            bail!(
                "local model install must be a real directory: {}",
                install_dir.display()
            );
        }
        let canonical_install_dir = std::fs::canonicalize(install_dir).with_context(|| {
            format!("canonicalize local model install {}", install_dir.display())
        })?;
        #[cfg(windows)]
        let install_anchor = DirectoryAnchor::open_owner_only(&canonical_install_dir, false)
            .with_context(|| {
                format!(
                    "validate and anchor owner-only local model install {}",
                    canonical_install_dir.display()
                )
            })?;
        cleanup_stale_staging(&canonical_install_dir)?;
        for _ in 0..32 {
            let path = canonical_install_dir.join(format!(
                "{STAGING_PREFIX}{}.{}",
                std::process::id(),
                STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            match create_owner_only_directory(&path) {
                Ok(created) => {
                    #[cfg(not(windows))]
                    let () = created;
                    return Ok(Self {
                        #[cfg(not(windows))]
                        install_dir: canonical_install_dir,
                        path,
                        #[cfg(windows)]
                        staging_anchor: Some(created),
                        #[cfg(windows)]
                        _install_anchor: install_anchor,
                        armed: true,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("create download staging {}", path.display()));
                }
            }
        }
        bail!(
            "could not create unique local model download staging under {}",
            canonical_install_dir.display()
        )
    }

    pub(in crate::retrieval::embedding::local_semantic) fn path(&self) -> &Path {
        &self.path
    }

    pub(in crate::retrieval::embedding::local_semantic) fn cleanup(mut self) -> Result<()> {
        #[cfg(windows)]
        let result = self
            .staging_anchor
            .take()
            .context("Windows download staging anchor is missing")
            .and_then(|anchor| windows_cleanup::remove_created_staging(&self.path, anchor));
        #[cfg(not(windows))]
        let result = remove_managed_tree(&self.install_dir, &self.path);
        self.armed = false;
        result
    }
}

impl Drop for DownloadStaging {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        #[cfg(windows)]
        let result = self
            .staging_anchor
            .take()
            .context("refusing Windows staging cleanup without its FileId anchor")
            .and_then(|anchor| windows_cleanup::remove_created_staging(&self.path, anchor));
        #[cfg(not(windows))]
        let result = remove_managed_tree(&self.install_dir, &self.path);
        if let Err(error) = result {
            crate::log::error(
                "embedding",
                &format!(
                    "remove failed local model download staging {}: {error:#}",
                    self.path.display()
                ),
            );
        }
    }
}

fn cleanup_stale_staging(install_dir: &Path) -> Result<()> {
    for entry in std::fs::read_dir(install_dir)
        .with_context(|| format!("read local model install {}", install_dir.display()))?
    {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(STAGING_PREFIX) {
            continue;
        }
        #[cfg(windows)]
        let result = windows_cleanup::remove_stale_staging(&entry.path());
        #[cfg(not(windows))]
        let result = remove_managed_tree(install_dir, &entry.path());
        result.with_context(|| {
            format!(
                "clean stale local model download staging {}",
                entry.path().display()
            )
        })?;
    }
    Ok(())
}

#[cfg(unix)]
fn create_owner_only_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    let mut builder = std::fs::DirBuilder::new();
    builder.mode(0o700).create(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

#[cfg(windows)]
fn create_owner_only_directory(path: &Path) -> std::io::Result<DirectoryAnchor> {
    windows_security::create_owner_only_cleanup_directory(path, false)
}

#[cfg(not(any(unix, windows)))]
fn create_owner_only_directory(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir(path)
}
