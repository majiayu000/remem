use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{bail, Context, Result};

use super::{capture_control_file, StoredPermissions};

static TRANSACTION_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) fn atomic_replace_control_file(
    path: &Path,
    content: &[u8],
    expected_current: Option<&[u8]>,
    permissions: Option<&StoredPermissions>,
) -> Result<()> {
    let (temp_path, mut temp_file) = create_transaction_temp(path)?;
    let mut cleanup = TempFileCleanup::new(temp_path.clone());
    temp_file
        .write_all(content)
        .with_context(|| format!("write control temp {}", temp_path.display()))?;
    if let Some(permissions) = permissions {
        permissions.apply(&temp_file)?;
    }
    temp_file
        .sync_all()
        .with_context(|| format!("sync control temp {}", temp_path.display()))?;
    drop(temp_file);
    validate_expected_control_file(path, expected_current)?;
    replace_file(&temp_path, path)?;
    cleanup.disarm();
    sync_parent(
        path.parent()
            .context("active revision ref must have a parent")?,
    )
}

fn validate_expected_control_file(path: &Path, expected: Option<&[u8]>) -> Result<()> {
    match (capture_control_file(path)?, expected) {
        (None, None) => Ok(()),
        (Some(actual), Some(expected)) if actual.content == expected => Ok(()),
        (Some(_), _) => bail!(
            "active revision ref changed during transaction: {}",
            path.display()
        ),
        (None, Some(_)) => bail!(
            "active revision ref disappeared during transaction: {}",
            path.display()
        ),
    }
}

pub(super) fn remove_candidate_control_file(path: &Path, candidate: &[u8]) -> Result<()> {
    validate_expected_control_file(path, Some(candidate))?;
    std::fs::remove_file(path)
        .with_context(|| format!("remove candidate active revision {}", path.display()))?;
    sync_parent(
        path.parent()
            .context("active revision ref must have a parent")?,
    )
}

pub(super) fn create_transaction_temp(path: &Path) -> Result<(PathBuf, File)> {
    let parent = path
        .parent()
        .context("transaction path must have a parent")?;
    let file_name = path
        .file_name()
        .context("transaction path must have a file name")?;
    for _ in 0..32 {
        let mut name = OsString::from(".");
        name.push(file_name);
        name.push(format!(
            ".remem-txn.{}.{}",
            std::process::id(),
            TRANSACTION_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let temp_path = parent.join(name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => return Ok((temp_path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("create activation transaction temp {}", temp_path.display())
                });
            }
        }
    }
    bail!("could not create transaction temp for {}", path.display())
}

pub(super) struct TempFileCleanup {
    path: PathBuf,
    armed: bool,
}

impl TempFileCleanup {
    pub(super) fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    pub(super) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TempFileCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Err(error) = std::fs::remove_file(&self.path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                crate::log::error(
                    "embedding",
                    &format!(
                        "remove failed activation temp {}: {error}",
                        self.path.display()
                    ),
                );
            }
        }
    }
}

#[cfg(not(windows))]
pub(super) fn replace_file(temp_path: &Path, path: &Path) -> Result<()> {
    std::fs::rename(temp_path, path)
        .with_context(|| format!("atomically replace control file {}", path.display()))
}

#[cfg(windows)]
pub(super) fn replace_file(temp_path: &Path, path: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;

    #[link(name = "Kernel32")]
    unsafe extern "system" {
        #[link_name = "MoveFileExW"]
        fn move_file_ex_w(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }

    let existing = temp_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let new = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let ok = unsafe {
        move_file_ex_w(
            existing.as_ptr(),
            new.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("atomically replace control file {}", path.display()));
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn sync_parent(parent: &Path) -> Result<()> {
    File::open(parent)
        .with_context(|| format!("open activation parent {}", parent.display()))?
        .sync_all()
        .with_context(|| format!("sync activation parent {}", parent.display()))
}

#[cfg(not(unix))]
pub(super) fn sync_parent(_parent: &Path) -> Result<()> {
    Ok(())
}
