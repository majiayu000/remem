use std::io::Write;

fn log_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|d| d.join(".claude-mem/remem.log"))
}

fn write_log(level: &str, component: &str, msg: &str) {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let line = format!("[{}] [{}] [{}] {}", now, level, component, msg);
    eprintln!("{}", line);
    if let Some(path) = log_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = writeln!(f, "{}", line);
        }
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
