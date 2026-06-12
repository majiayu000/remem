use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{bail, Context, Result};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
static FAIL_RENAME_PATH: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);
#[cfg(test)]
static FAILPOINT_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) fn write_atomic(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> Result<()> {
    let path = resolve_final_path(path.as_ref())?;
    let parent = parent_dir(&path);
    fs::create_dir_all(parent)
        .with_context(|| format!("create parent directory {}", parent.display()))?;

    let (temp_path, file) = create_temp_file(&path)?;
    let result = write_via_temp(&path, parent, &temp_path, file, contents.as_ref());
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn create_temp_file(path: &Path) -> Result<(PathBuf, File)> {
    let mut last_collision = None;
    for _ in 0..16 {
        let temp_path = temp_path_for(path)?;
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => return Ok((temp_path, file)),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                last_collision = Some(temp_path);
            }
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("create temp file {}", temp_path.display()));
            }
        }
    }

    let last = last_collision
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<none>".to_string());
    bail!(
        "create unique temp file for {} failed after repeated collisions; last collision {}",
        path.display(),
        last
    );
}

fn write_via_temp(
    path: &Path,
    parent: &Path,
    temp_path: &Path,
    mut file: File,
    contents: &[u8],
) -> Result<()> {
    file.write_all(contents)
        .with_context(|| format!("write temp file {}", temp_path.display()))?;
    copy_permissions_if_present(path, &file)?;
    file.sync_all()
        .with_context(|| format!("sync temp file {}", temp_path.display()))?;
    drop(file);

    #[cfg(test)]
    if should_fail_rename_for_test(path) {
        bail!("injected atomic write failure before rename");
    }

    replace_file(temp_path, path)?;
    sync_parent_dir(parent)?;
    Ok(())
}

fn copy_permissions_if_present(path: &Path, temp_file: &File) -> Result<()> {
    match fs::metadata(path) {
        Ok(metadata) => temp_file
            .set_permissions(metadata.permissions())
            .with_context(|| format!("copy permissions from {}", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("read metadata {}", path.display())),
    }
}

fn resolve_final_path(path: &Path) -> Result<PathBuf> {
    let mut current = path.to_path_buf();
    for _ in 0..32 {
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let target = fs::read_link(&current)
                    .with_context(|| format!("read symlink {}", current.display()))?;
                current = if target.is_absolute() {
                    target
                } else {
                    parent_dir(&current).join(target)
                };
            }
            Ok(_) => return Ok(current),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(current),
            Err(err) => {
                return Err(err).with_context(|| format!("read metadata {}", current.display()));
            }
        }
    }
    anyhow::bail!(
        "too many symlink hops while resolving atomic write path {}",
        path.display()
    );
}

fn temp_path_for(path: &Path) -> Result<PathBuf> {
    let parent = parent_dir(path);
    let file_name = path
        .file_name()
        .with_context(|| format!("atomic write path has no file name: {}", path.display()))?;
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut nonce = [0u8; 8];
    getrandom::fill(&mut nonce)
        .map_err(|err| anyhow::anyhow!("generate atomic temp file nonce: {err}"))?;
    let nonce = u64::from_ne_bytes(nonce);
    let mut temp_name = OsString::from(".");
    temp_name.push(file_name);
    temp_name.push(format!(
        ".tmp.{}.{}.{nonce:016x}",
        std::process::id(),
        counter
    ));
    Ok(parent.join(temp_name))
}

fn parent_dir(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

#[cfg(unix)]
fn sync_parent_dir(parent: &Path) -> Result<()> {
    let dir =
        File::open(parent).with_context(|| format!("open parent dir {}", parent.display()))?;
    dir.sync_all()
        .with_context(|| format!("sync parent dir {}", parent.display()))
}

#[cfg(not(unix))]
fn sync_parent_dir(_parent: &Path) -> Result<()> {
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(temp_path: &Path, path: &Path) -> Result<()> {
    fs::rename(temp_path, path)
        .with_context(|| format!("rename {} to {}", temp_path.display(), path.display()))
}

#[cfg(windows)]
fn replace_file(temp_path: &Path, path: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;

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
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("replace {} with {}", path.display(), temp_path.display()));
    }
    Ok(())
}

#[cfg(windows)]
fn wide_null(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(test)]
pub(crate) fn fail_next_rename_for_path_for_test(path: impl AsRef<Path>) {
    let mut fail_path = match FAIL_RENAME_PATH.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    *fail_path = Some(path.as_ref().to_path_buf());
}

#[cfg(test)]
pub(crate) fn clear_failpoints_for_test() {
    let mut fail_path = match FAIL_RENAME_PATH.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    *fail_path = None;
}

#[cfg(test)]
fn should_fail_rename_for_test(path: &Path) -> bool {
    let mut fail_path = match FAIL_RENAME_PATH.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if fail_path.as_deref() == Some(path) {
        *fail_path = None;
        return true;
    }
    false
}

#[cfg(test)]
pub(crate) fn failpoint_test_lock() -> std::sync::MutexGuard<'static, ()> {
    match FAILPOINT_TEST_LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injected_failure_preserves_existing_file() -> Result<()> {
        let _guard = failpoint_test_lock();
        let path = std::env::temp_dir().join(format!(
            "remem-atomic-write-{}-{}.txt",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, "old")?;
        fail_next_rename_for_path_for_test(&path);

        let err = write_atomic(&path, "new").expect_err("injected failure must abort");
        assert!(err.to_string().contains("injected atomic write failure"));
        assert_eq!(fs::read_to_string(&path)?, "old");
        let _ = fs::remove_file(path);
        Ok(())
    }

    #[test]
    fn successful_write_replaces_existing_file() -> Result<()> {
        let _guard = failpoint_test_lock();
        let path = std::env::temp_dir().join(format!(
            "remem-atomic-write-success-{}-{}.txt",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, "old")?;

        write_atomic(&path, "new")?;

        assert_eq!(fs::read_to_string(&path)?, "new");
        let _ = fs::remove_file(path);
        Ok(())
    }

    #[test]
    fn rename_failpoint_is_path_scoped() -> Result<()> {
        let _guard = failpoint_test_lock();
        let path = std::env::temp_dir().join(format!(
            "remem-atomic-write-scoped-{}-{}.txt",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let other = std::env::temp_dir().join(format!(
            "remem-atomic-write-other-{}-{}.txt",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, "old")?;
        fs::write(&other, "old")?;
        fail_next_rename_for_path_for_test(&path);

        write_atomic(&other, "new")?;
        let err = write_atomic(&path, "new").expect_err("targeted failure must abort");

        assert!(err.to_string().contains("injected atomic write failure"));
        assert_eq!(fs::read_to_string(&other)?, "new");
        assert_eq!(fs::read_to_string(&path)?, "old");
        clear_failpoints_for_test();
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(other);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn symlink_target_is_updated_without_replacing_link() -> Result<()> {
        let _guard = failpoint_test_lock();
        let dir = std::env::temp_dir().join(format!(
            "remem-atomic-write-symlink-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir)?;
        let target = dir.join("target.toml");
        let link = dir.join("config.toml");
        fs::write(&target, "old")?;
        std::os::unix::fs::symlink(&target, &link)?;

        write_atomic(&link, "new")?;

        assert!(fs::symlink_metadata(&link)?.file_type().is_symlink());
        assert_eq!(fs::read_to_string(&target)?, "new");
        let _ = fs::remove_file(link);
        let _ = fs::remove_file(target);
        let _ = fs::remove_dir(dir);
        Ok(())
    }
}
