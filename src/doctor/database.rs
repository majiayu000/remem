use super::types::{Check, Status};
use crate::db;

pub(super) fn check_database() -> Check {
    let db_path = db::db_path();
    if !db_path.exists() {
        return Check {
            name: "Database",
            status: Status::Fail,
            detail: format!("{} (not found)", db_path.display()),
        };
    }

    let size = std::fs::metadata(&db_path)
        .map(|meta| meta.len())
        .unwrap_or(0);
    match db::open_db() {
        Ok(conn) => match db::query_system_stats(&conn) {
            Ok(stats) => Check {
                name: "Database",
                status: Status::Ok,
                detail: format!(
                    "{} ({:.1} MB, {} memories)",
                    db_path.display(),
                    size as f64 / 1_048_576.0,
                    stats.active_memories
                ),
            },
            Err(err) => Check {
                name: "Database",
                status: Status::Fail,
                detail: format!("{} (stats error: {})", db_path.display(), err),
            },
        },
        Err(err) => Check {
            name: "Database",
            status: Status::Fail,
            detail: format!("{} (open error: {})", db_path.display(), err),
        },
    }
}

pub(super) fn check_pending_queue() -> Check {
    let conn = match db::open_db() {
        Ok(conn) => conn,
        Err(_) => {
            return Check {
                name: "Pending queue",
                status: Status::Warn,
                detail: "cannot open database".to_string(),
            };
        }
    };

    let stats = match db::query_system_stats(&conn) {
        Ok(stats) => stats,
        Err(err) => {
            return Check {
                name: "Pending queue",
                status: Status::Warn,
                detail: format!("cannot load queue stats: {}", err),
            };
        }
    };
    let pending = stats.pending_observations;
    let failed_pending = stats.failed_pending_observations;
    let stuck_jobs = stats.stuck_jobs;

    if stuck_jobs > 0 {
        Check {
            name: "Pending queue",
            status: Status::Warn,
            detail: format!(
                "{} pending, {} failed, {} stuck jobs (will auto-recover)",
                pending, failed_pending, stuck_jobs
            ),
        }
    } else if failed_pending > 0 {
        Check {
            name: "Pending queue",
            status: Status::Warn,
            detail: format!(
                "{} pending, {} failed (inspect parsing/AI failures)",
                pending, failed_pending
            ),
        }
    } else if pending > 100 {
        Check {
            name: "Pending queue",
            status: Status::Warn,
            detail: format!(
                "{} pending, {} failed (backlog building up)",
                pending, failed_pending
            ),
        }
    } else {
        Check {
            name: "Pending queue",
            status: Status::Ok,
            detail: format!("{} pending, {} failed", pending, failed_pending),
        }
    }
}

pub(super) fn check_disk_space() -> Check {
    let db_path = db::db_path();
    let db_size = std::fs::metadata(&db_path)
        .map(|meta| meta.len())
        .unwrap_or(0);
    let log_path = db_path.parent().map(|parent| parent.join("remem.log"));
    let log_size = log_path
        .and_then(|path| std::fs::metadata(&path).ok())
        .map(|meta| meta.len())
        .unwrap_or(0);
    let total_mb = (db_size + log_size) as f64 / 1_048_576.0;

    if total_mb > 500.0 {
        Check {
            name: "Disk usage",
            status: Status::Warn,
            detail: format!(
                "{:.1} MB total (DB: {:.1} MB, logs: {:.1} MB) — consider `remem cleanup`",
                total_mb,
                db_size as f64 / 1_048_576.0,
                log_size as f64 / 1_048_576.0
            ),
        }
    } else {
        Check {
            name: "Disk usage",
            status: Status::Ok,
            detail: format!(
                "{:.1} MB total (DB: {:.1} MB, logs: {:.1} MB)",
                total_mb,
                db_size as f64 / 1_048_576.0,
                log_size as f64 / 1_048_576.0
            ),
        }
    }
}
