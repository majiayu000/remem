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
    if std::env::var_os("REMEM_STDERR_TO_LOG").is_none() {
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

pub fn debug(component: &str, msg: &str) {
    if std::env::var("REMEM_DEBUG").is_ok() {
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
