use super::{Check, Status};

pub(in crate::doctor) fn check_disk_space() -> Check {
    let db_path = match crate::db::try_db_path() {
        Ok(path) => path,
        Err(error) => return Check::new("Disk usage", Status::Fail, error.to_string()),
    };
    let db_size = std::fs::metadata(&db_path)
        .map(|meta| meta.len())
        .unwrap_or(0);
    let log_size = crate::log::log_health_snapshot()
        .map(|snapshot| snapshot.total_bytes)
        .unwrap_or(0);
    let total_mb = (db_size + log_size) as f64 / 1_048_576.0;

    if total_mb > 500.0 {
        Check::new(
            "Disk usage",
            Status::Warn,
            format!(
                "{:.1} MB total (DB: {:.1} MB, logs: {:.1} MB) — consider `remem cleanup`",
                total_mb,
                db_size as f64 / 1_048_576.0,
                log_size as f64 / 1_048_576.0
            ),
        )
    } else {
        Check::new(
            "Disk usage",
            Status::Ok,
            format!(
                "{:.1} MB total (DB: {:.1} MB, logs: {:.1} MB)",
                total_mb,
                db_size as f64 / 1_048_576.0,
                log_size as f64 / 1_048_576.0
            ),
        )
    }
}
