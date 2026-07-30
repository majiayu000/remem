use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::super::super::{sha256_file, LocalModelFile};

#[derive(Clone, Debug)]
pub(super) struct PrivateRuntimeFile {
    pub(super) relative_path: PathBuf,
    pub(super) expected: LocalModelFile,
}

#[derive(Clone, Debug)]
pub(super) struct PrivateRuntimeLayout {
    pub(super) root: PathBuf,
    pub(super) repo_dir: PathBuf,
    pub(super) revision: String,
    pub(super) files: Vec<PrivateRuntimeFile>,
}

impl PrivateRuntimeLayout {
    pub(super) fn verify(&self) -> Result<()> {
        verify_root(&self.root)?;
        let ref_relative = self.repo_dir.join("refs/main");
        let mut expected_files = self
            .files
            .iter()
            .map(|file| file.relative_path.clone())
            .collect::<HashSet<_>>();
        expected_files.insert(ref_relative.clone());
        verify_exact_tree(&self.root, &expected_files)?;

        let ref_path = self.root.join(&ref_relative);
        let active_revision = std::fs::read_to_string(&ref_path)
            .with_context(|| format!("read private runtime ref {}", ref_path.display()))?;
        if active_revision != self.revision {
            bail!(
                "private local embedding runtime ref changed: expected {}, got {}",
                self.revision,
                active_revision
            );
        }

        for file in &self.files {
            let path = self.root.join(&file.relative_path);
            let metadata = std::fs::symlink_metadata(&path)
                .with_context(|| format!("stat private runtime artifact {}", path.display()))?;
            if metadata.len() != file.expected.bytes {
                bail!(
                    "private runtime artifact {} size changed: expected {}, got {}",
                    path.display(),
                    file.expected.bytes,
                    metadata.len()
                );
            }
            let actual = sha256_file(&path)?;
            if actual != file.expected.sha256 {
                bail!(
                    "private runtime artifact {} checksum changed: expected {}, got {}",
                    path.display(),
                    file.expected.sha256,
                    actual
                );
            }
        }
        Ok(())
    }
}

fn verify_root(root: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(root)
        .with_context(|| format!("stat private runtime cache {}", root.display()))?;
    if !metadata.file_type().is_dir() {
        bail!(
            "private local embedding runtime cache is not a directory: {}",
            root.display()
        );
    }
    let canonical = std::fs::canonicalize(root)
        .with_context(|| format!("canonicalize private runtime cache {}", root.display()))?;
    if canonical != root {
        bail!(
            "private local embedding runtime cache path changed: expected {}, got {}",
            root.display(),
            canonical.display()
        );
    }
    Ok(())
}

fn verify_exact_tree(root: &Path, expected_files: &HashSet<PathBuf>) -> Result<()> {
    let expected_dirs = expected_directories(expected_files);
    let mut seen_files = HashSet::new();
    let mut seen_dirs = HashSet::new();
    visit_tree(
        root,
        root,
        expected_files,
        &expected_dirs,
        &mut seen_files,
        &mut seen_dirs,
    )?;
    if seen_files != *expected_files {
        bail!("private runtime cache is missing one or more expected files");
    }
    if seen_dirs != expected_dirs {
        bail!("private runtime cache directory layout changed");
    }
    Ok(())
}

fn expected_directories(expected_files: &HashSet<PathBuf>) -> HashSet<PathBuf> {
    let mut directories = HashSet::new();
    for file in expected_files {
        let mut current = file.parent();
        while let Some(parent) = current {
            if parent.as_os_str().is_empty() {
                break;
            }
            directories.insert(parent.to_path_buf());
            current = parent.parent();
        }
    }
    directories
}

fn visit_tree(
    root: &Path,
    current: &Path,
    expected_files: &HashSet<PathBuf>,
    expected_dirs: &HashSet<PathBuf>,
    seen_files: &mut HashSet<PathBuf>,
    seen_dirs: &mut HashSet<PathBuf>,
) -> Result<()> {
    for entry in
        std::fs::read_dir(current).with_context(|| format!("read {}", current.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .with_context(|| format!("make {} relative to {}", path.display(), root.display()))?
            .to_path_buf();
        let metadata = std::fs::symlink_metadata(&path)
            .with_context(|| format!("stat private runtime path {}", path.display()))?;
        let canonical = std::fs::canonicalize(&path)
            .with_context(|| format!("canonicalize private runtime path {}", path.display()))?;
        if canonical != path || !canonical.starts_with(root) {
            bail!(
                "private runtime path escapes private cache: {}",
                canonical.display()
            );
        }
        if metadata.file_type().is_dir() {
            if !expected_dirs.contains(&relative) {
                bail!(
                    "private runtime cache has unexpected directory {}",
                    relative.display()
                );
            }
            seen_dirs.insert(relative);
            visit_tree(
                root,
                &path,
                expected_files,
                expected_dirs,
                seen_files,
                seen_dirs,
            )?;
        } else if metadata.file_type().is_file() {
            if !expected_files.contains(&relative) {
                bail!(
                    "private runtime cache has unexpected file {}",
                    relative.display()
                );
            }
            if !metadata.permissions().readonly() {
                bail!(
                    "private runtime cache file is writable: {}",
                    relative.display()
                );
            }
            seen_files.insert(relative);
        } else {
            bail!(
                "private runtime cache path is not a regular file or directory: {}",
                relative.display()
            );
        }
    }
    Ok(())
}
