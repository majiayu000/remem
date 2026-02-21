use std::io::Write;

const DEFAULT_LOG_MAX_BYTES: u64 = 10 * 1024 * 1024;
const LOG_ROTATION_KEEP: usize = 3;

fn log_path() -> Option<std::path::PathBuf> {
    let data_dir = std::env::var("REMEM_DATA_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|_| dirs::home_dir().map(|d| d.join(".remem")).ok_or(()))
        .ok()?;
    Some(data_dir.join("remem.log"))
}

fn log_max_bytes() -> u64 {
    std::env::var("REMEM_LOG_MAX_BYTES")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_LOG_MAX_BYTES)
}

fn rotated_log_path(base: &std::path::Path, index: usize) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("{}.{}", base.display(), index))
}

fn rotate_if_needed(path: &std::path::Path, max_bytes: u64) {
    let size = match std::fs::metadata(path) {
        Ok(m) => m.len(),
        Err(_) => 0,
    };
    if size < max_bytes {
        return;
    }

    for i in (1..=LOG_ROTATION_KEEP).rev() {
        let dst = rotated_log_path(path, i);
        if i == LOG_ROTATION_KEEP {
            let _ = std::fs::remove_file(&dst);
        }
        let src = if i == 1 {
            path.to_path_buf()
        } else {
            rotated_log_path(path, i - 1)
        };
        if src.exists() {
            let _ = std::fs::rename(&src, &dst);
        }
    }
}

fn write_log(level: &str, component: &str, msg: &str) {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let line = format!("[{}] [{}] [{}] {}", now, level, component, msg);
    eprintln!("{}", line);
    if let Some(path) = log_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        rotate_if_needed(&path, log_max_bytes());
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = writeln!(f, "{}", line);
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

pub struct Timer {
    component: String,
    start: std::time::Instant,
}

impl Timer {
    pub fn start(component: &str, msg: &str) -> Self {
        info(component, &format!("START {}", msg));
        Self {
            component: component.to_string(),
            start: std::time::Instant::now(),
        }
    }

    pub fn done(self, msg: &str) {
        let ms = self.start.elapsed().as_millis();
        info(&self.component, &format!("DONE {}ms {}", ms, msg));
    }

    pub fn done_with_error(self, err: &str) {
        let ms = self.start.elapsed().as_millis();
        error(&self.component, &format!("FAIL {}ms {}", ms, err));
    }
}
