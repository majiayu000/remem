use std::io::Write;
use std::path::Path;

use super::config::{log_max_bytes, log_path, rotated_log_path, LOG_ROTATION_KEEP};

pub(crate) fn rotate_if_needed(path: &Path, max_bytes: u64) {
    let size = match std::fs::metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(_) => 0,
    };
    if size < max_bytes {
        return;
    }

    for index in (1..=LOG_ROTATION_KEEP).rev() {
        let dst = rotated_log_path(path, index);
        if index == LOG_ROTATION_KEEP {
            if let Err(error) = std::fs::remove_file(&dst) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    eprintln!("[remem] log rotate: remove {:?} failed: {}", dst, error);
                }
            }
        }
        let src = if index == 1 {
            path.to_path_buf()
        } else {
            rotated_log_path(path, index - 1)
        };
        if src.exists() {
            if let Err(error) = std::fs::rename(&src, &dst) {
                eprintln!(
                    "[remem] log rotate: rename {:?} → {:?} failed: {}",
                    src, dst, error
                );
            }
        }
    }
}

fn write_log(level: &str, component: &str, msg: &str) {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let line = format!("[{}] [{}] [{}] {}", now, level, component, msg);
    if should_mirror_to_stderr(level, component) {
        eprintln!("{}", line);
    }
    if let Some(path) = log_path() {
        if let Some(parent) = path.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                eprintln!("[remem] log dir create failed: {}", error);
                return;
            }
        }
        rotate_if_needed(&path, log_max_bytes());
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            Ok(mut file) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
                }
                if let Err(error) = writeln!(file, "{}", line) {
                    eprintln!("[remem] log write failed: {}", error);
                }
            }
            Err(error) => {
                eprintln!("[remem] log open failed: {}", error);
            }
        }
    }
}

fn should_mirror_to_stderr(level: &str, component: &str) -> bool {
    if std::env::var_os("REMEM_STDERR_TO_LOG").is_some() {
        return false;
    }
    if level == "INFO" && component == "migrate" {
        return debug_enabled();
    }
    true
}

pub fn open_log_append() -> Option<std::fs::File> {
    let path = log_path()?;
    if let Some(parent) = path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            eprintln!("[remem] log dir create failed: {}", error);
            return None;
        }
    }
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(file) => Some(file),
        Err(error) => {
            eprintln!("[remem] open log for child stderr failed: {}", error);
            None
        }
    }
}

pub fn debug_enabled() -> bool {
    std::env::var("REMEM_DEBUG").is_ok()
}

pub fn debug(component: &str, msg: &str) {
    if debug_enabled() {
        write_log("DEBUG", component, msg);
    }
}

pub fn info(component: &str, msg: &str) {
    write_log("INFO", component, msg);
}

pub fn warn(component: &str, msg: &str) {
    write_log("WARN", component, msg);
}

pub fn error(component: &str, msg: &str) {
    write_log("ERROR", component, msg);
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_log_env<T>(
        debug: Option<&str>,
        stderr_to_log: Option<&str>,
        f: impl FnOnce() -> T,
    ) -> T {
        let _guard = match ENV_LOCK.lock() {
            Ok(guard) => guard,
            Err(error) => panic!("log env lock should acquire: {error}"),
        };
        let previous_debug = std::env::var("REMEM_DEBUG").ok();
        let previous_stderr_to_log = std::env::var("REMEM_STDERR_TO_LOG").ok();

        match debug {
            Some(value) => unsafe { std::env::set_var("REMEM_DEBUG", value) },
            None => unsafe { std::env::remove_var("REMEM_DEBUG") },
        }
        match stderr_to_log {
            Some(value) => unsafe { std::env::set_var("REMEM_STDERR_TO_LOG", value) },
            None => unsafe { std::env::remove_var("REMEM_STDERR_TO_LOG") },
        }

        let result = f();

        match previous_debug {
            Some(value) => unsafe { std::env::set_var("REMEM_DEBUG", value) },
            None => unsafe { std::env::remove_var("REMEM_DEBUG") },
        }
        match previous_stderr_to_log {
            Some(value) => unsafe { std::env::set_var("REMEM_STDERR_TO_LOG", value) },
            None => unsafe { std::env::remove_var("REMEM_STDERR_TO_LOG") },
        }

        result
    }

    #[test]
    fn migrate_info_is_not_mirrored_to_stderr_by_default() {
        with_log_env(None, None, || {
            assert!(!super::should_mirror_to_stderr("INFO", "migrate"));
            assert!(super::should_mirror_to_stderr("INFO", "install"));
            assert!(super::should_mirror_to_stderr("ERROR", "migrate"));
        });
    }

    #[test]
    fn migrate_info_is_mirrored_to_stderr_when_debug_enabled() {
        with_log_env(Some("1"), None, || {
            assert!(super::should_mirror_to_stderr("INFO", "migrate"));
        });
    }

    #[test]
    fn stderr_to_log_disables_stderr_mirroring() {
        with_log_env(Some("1"), Some("1"), || {
            assert!(!super::should_mirror_to_stderr("INFO", "migrate"));
            assert!(!super::should_mirror_to_stderr("ERROR", "migrate"));
        });
    }
}
