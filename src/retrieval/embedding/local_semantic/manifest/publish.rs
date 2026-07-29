use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{bail, Context, Result};

use super::super::{LocalModelManifest, MANIFEST_FILE};

static MANIFEST_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(in crate::retrieval::embedding::local_semantic) fn write_manifest(
    install_dir: &Path,
    manifest: &LocalModelManifest,
) -> Result<()> {
    let canonical_install_dir = std::fs::canonicalize(install_dir)
        .with_context(|| format!("canonicalize local model dir {}", install_dir.display()))?;
    let install_metadata =
        std::fs::symlink_metadata(&canonical_install_dir).with_context(|| {
            format!(
                "stat canonical local model dir {}",
                canonical_install_dir.display()
            )
        })?;
    if !install_metadata.file_type().is_dir() {
        bail!(
            "local model manifest parent is not a directory: {}",
            canonical_install_dir.display()
        );
    }
    let path = canonical_install_dir.join(MANIFEST_FILE);
    let existing_permissions = validate_manifest_destination(&path)?;
    let content = serde_json::to_vec_pretty(manifest).context("serialize local model manifest")?;
    let (temp_path, mut temp_file) = create_manifest_temp(&path)?;
    let mut cleanup = ManifestTempCleanup::new(temp_path.clone());

    temp_file
        .write_all(&content)
        .with_context(|| format!("write local model manifest temp {}", temp_path.display()))?;
    if let Some(permissions) = existing_permissions {
        temp_file.set_permissions(permissions).with_context(|| {
            format!(
                "preserve local model manifest permissions {}",
                path.display()
            )
        })?;
    }
    temp_file
        .sync_all()
        .with_context(|| format!("sync local model manifest temp {}", temp_path.display()))?;
    drop(temp_file);

    // Recheck after writing the temp file. An attacker can still race this check, but the
    // atomic replacement below replaces the directory entry itself and never follows it.
    validate_manifest_destination(&path)?;
    replace_file(&temp_path, &path)?;
    cleanup.disarm();
    sync_parent_dir(&canonical_install_dir)?;
    Ok(())
}

fn validate_manifest_destination(path: &Path) -> Result<Option<std::fs::Permissions>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(Some(metadata.permissions())),
        Ok(metadata) if metadata.file_type().is_symlink() => bail!(
            "refusing to publish local model manifest through symlink {}",
            path.display()
        ),
        Ok(_) => bail!(
            "local model manifest destination is not a regular file: {}",
            path.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => {
            Err(error).with_context(|| format!("stat manifest destination {}", path.display()))
        }
    }
}

fn create_manifest_temp(path: &Path) -> Result<(PathBuf, File)> {
    let parent = path
        .parent()
        .context("local model manifest path should have a parent")?;
    let file_name = path
        .file_name()
        .context("local model manifest path should have a file name")?;
    let mut last_collision = None;
    for _ in 0..16 {
        let mut temp_name = OsString::from(".");
        temp_name.push(file_name);
        temp_name.push(format!(
            ".tmp.{}.{}",
            std::process::id(),
            MANIFEST_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let temp_path = parent.join(temp_name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => return Ok((temp_path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                last_collision = Some(temp_path);
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("create manifest temp {}", temp_path.display()));
            }
        }
    }
    bail!(
        "create unique manifest temp for {} failed; last collision {}",
        path.display(),
        last_collision
            .as_deref()
            .map_or_else(|| "<none>".to_string(), |path| path.display().to_string())
    )
}

struct ManifestTempCleanup {
    path: PathBuf,
    armed: bool,
}

impl ManifestTempCleanup {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ManifestTempCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Err(error) = fs::remove_file(&self.path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                crate::log::error(
                    "embedding",
                    &format!(
                        "remove failed local model manifest temp {}: {error}",
                        self.path.display()
                    ),
                );
            }
        }
    }
}

#[cfg(not(windows))]
fn replace_file(temp_path: &Path, path: &Path) -> Result<()> {
    fs::rename(temp_path, path)
        .with_context(|| format!("atomically publish local model manifest {}", path.display()))
}

#[cfg(windows)]
fn replace_file(temp_path: &Path, path: &Path) -> Result<()> {
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;

    #[link(name = "Kernel32")]
    unsafe extern "system" {
        #[link_name = "MoveFileExW"]
        fn move_file_ex_w(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }

    let existing = wide_null(temp_path);
    let new = wide_null(path);
    let ok = unsafe {
        move_file_ex_w(
            existing.as_ptr(),
            new.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error()).with_context(|| {
            format!(
                "atomically publish local model manifest {} from {}",
                path.display(),
                temp_path.display()
            )
        });
    }
    Ok(())
}

#[cfg(windows)]
fn wide_null(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;

    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(unix)]
fn sync_parent_dir(parent: &Path) -> Result<()> {
    File::open(parent)
        .with_context(|| format!("open manifest parent {}", parent.display()))?
        .sync_all()
        .with_context(|| format!("sync manifest parent {}", parent.display()))
}

#[cfg(not(unix))]
fn sync_parent_dir(_parent: &Path) -> Result<()> {
    Ok(())
}
