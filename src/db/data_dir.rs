use std::path::PathBuf;

/// Resolves the remem data directory from `REMEM_DATA_DIR` or the home
/// directory. The database and SQLCipher key live under this directory, so a
/// cwd fallback would silently place key material in an unexpected location
/// and split the store from `~/.remem`. A missing home directory without an
/// explicit override is an unrecoverable misconfiguration: fail closed.
pub(super) fn resolve(env_override: Option<PathBuf>, home_dir: Option<PathBuf>) -> PathBuf {
    if let Some(path) = env_override {
        return path;
    }
    let Some(home) = home_dir else {
        panic!(
            "remem cannot resolve a data directory: the home directory is unavailable \
             and REMEM_DATA_DIR is unset; set REMEM_DATA_DIR to an absolute path"
        );
    };
    home.join(".remem")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_override_wins_over_home() {
        let resolved = resolve(
            Some(PathBuf::from("/custom")),
            Some(PathBuf::from("/home/u")),
        );

        assert_eq!(resolved, PathBuf::from("/custom"));
    }

    #[test]
    fn home_directory_gets_dot_remem_suffix() {
        let resolved = resolve(None, Some(PathBuf::from("/home/u")));

        assert_eq!(resolved, PathBuf::from("/home/u/.remem"));
    }

    #[test]
    fn env_override_is_used_even_without_home() {
        let resolved = resolve(Some(PathBuf::from("/custom")), None);

        assert_eq!(resolved, PathBuf::from("/custom"));
    }

    #[test]
    #[should_panic(expected = "set REMEM_DATA_DIR")]
    fn missing_home_without_override_fails_closed() {
        resolve(None, None);
    }
}
