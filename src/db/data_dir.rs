use std::cell::RefCell;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

thread_local! {
    static DATA_DIR_OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

pub(crate) fn with_data_dir<T>(dir: &Path, f: impl FnOnce() -> T) -> T {
    let _guard = DataDirOverrideGuard::set(dir.to_path_buf());
    f()
}

pub fn data_dir() -> PathBuf {
    try_data_dir().unwrap_or_else(|error| panic!("{error}"))
}

pub fn try_data_dir() -> Result<PathBuf> {
    resolve_data_dir(
        DATA_DIR_OVERRIDE.with(|slot| slot.borrow().clone()),
        std::env::var("REMEM_DATA_DIR").ok(),
        dirs::home_dir(),
    )
}

/// Resolves the remem data directory from an explicit override,
/// `REMEM_DATA_DIR`, or the home directory. The database and SQLCipher key live
/// under this directory, so a cwd fallback would silently place key material in
/// an unexpected location and split the store from `~/.remem`. A missing home
/// directory without an explicit override is an unrecoverable misconfiguration:
/// fail closed.
pub(crate) fn resolve_data_dir(
    override_path: Option<PathBuf>,
    remem_data_dir: Option<String>,
    home_dir: Option<PathBuf>,
) -> Result<PathBuf> {
    if let Some(path) = override_path {
        return Ok(path);
    }
    if let Some(path) = remem_data_dir.filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    home_dir
        .map(|home| home.join(".remem"))
        .ok_or_else(|| anyhow!("cannot resolve remem data dir: HOME is unset; set REMEM_DATA_DIR"))
}

struct DataDirOverrideGuard {
    previous: Option<PathBuf>,
}

impl DataDirOverrideGuard {
    fn set(path: PathBuf) -> Self {
        let previous = DATA_DIR_OVERRIDE.with(|slot| slot.replace(Some(path)));
        Self { previous }
    }
}

impl Drop for DataDirOverrideGuard {
    fn drop(&mut self) {
        let previous = self.previous.take();
        DATA_DIR_OVERRIDE.with(|slot| {
            slot.replace(previous);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_data_dir_prefers_override_then_env_then_home() {
        let override_path = PathBuf::from("/tmp/override");
        assert_eq!(
            resolve_data_dir(
                Some(override_path.clone()),
                Some("/tmp/env".into()),
                Some(PathBuf::from("/home/user")),
            )
            .unwrap(),
            override_path
        );
        assert_eq!(
            resolve_data_dir(
                None,
                Some("/tmp/env".into()),
                Some(PathBuf::from("/home/user"))
            )
            .unwrap(),
            PathBuf::from("/tmp/env")
        );
        assert_eq!(
            resolve_data_dir(None, None, Some(PathBuf::from("/home/user"))).unwrap(),
            PathBuf::from("/home/user/.remem")
        );
    }

    #[test]
    fn resolve_data_dir_uses_env_even_without_home() {
        assert_eq!(
            resolve_data_dir(None, Some("/tmp/env".into()), None).unwrap(),
            PathBuf::from("/tmp/env")
        );
    }

    #[test]
    fn resolve_data_dir_ignores_empty_env_and_falls_back_to_home() {
        assert_eq!(
            resolve_data_dir(None, Some(String::new()), Some(PathBuf::from("/home/user"))).unwrap(),
            PathBuf::from("/home/user/.remem")
        );
    }

    #[test]
    fn resolve_data_dir_fails_closed_without_home_or_env() {
        let error = resolve_data_dir(None, None, None).unwrap_err().to_string();
        assert!(error.contains("HOME is unset"));
        assert!(error.contains("REMEM_DATA_DIR"));
        assert!(!error.contains(".remem"));
    }
}
