use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(test)]
use anyhow::bail;
use anyhow::{Context, Result};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
static FAIL_NEXT_RENAME: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[cfg(test)]
static FAILPOINT_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) fn write_atomic(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> Result<()> {
    let path = path.as_ref();
    let parent = parent_dir(path);
    fs::create_dir_all(parent)
        .with_context(|| format!("create parent directory {}", parent.display()))?;

    let temp_path = temp_path_for(path)?;
    let result = write_via_temp(path, parent, &temp_path, contents.as_ref());
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn write_via_temp(path: &Path, parent: &Path, temp_path: &Path, contents: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temp_path)
        .with_context(|| format!("create temp file {}", temp_path.display()))?;
    file.write_all(contents)
        .with_context(|| format!("write temp file {}", temp_path.display()))?;
    copy_permissions_if_present(path, &file)?;
    file.sync_all()
        .with_context(|| format!("sync temp file {}", temp_path.display()))?;
    drop(file);

    #[cfg(test)]
    if FAIL_NEXT_RENAME.swap(false, Ordering::SeqCst) {
        bail!("injected atomic write failure before rename");
    }

    fs::rename(temp_path, path)
        .with_context(|| format!("rename {} to {}", temp_path.display(), path.display()))?;
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

fn temp_path_for(path: &Path) -> Result<PathBuf> {
    let parent = parent_dir(path);
    let file_name = path
        .file_name()
        .with_context(|| format!("atomic write path has no file name: {}", path.display()))?;
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut temp_name = OsString::from(".");
    temp_name.push(file_name);
    temp_name.push(format!(".tmp.{}.{}", std::process::id(), counter));
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

#[cfg(test)]
pub(crate) fn fail_next_rename_for_test() {
    FAIL_NEXT_RENAME.store(true, Ordering::SeqCst);
}

#[cfg(test)]
pub(crate) fn clear_failpoints_for_test() {
    FAIL_NEXT_RENAME.store(false, Ordering::SeqCst);
}

#[cfg(test)]
pub(crate) fn failpoint_test_lock() -> std::sync::MutexGuard<'static, ()> {
    FAILPOINT_TEST_LOCK
        .lock()
        .expect("atomic write failpoint lock should acquire")
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
        fail_next_rename_for_test();

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
}
