use super::types::{Check, Status};
use crate::db;
use crate::doctor::health_action::{
    queue_actions, render_inline_hints, worker_once_fallback_detail,
};

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
    match db::open_db_read_only() {
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
    let conn = match db::open_db_read_only() {
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
    let detail = format!(
        "{} ready, {} delayed, {} processing ({} expired), {} failed pending; {} jobs pending, {} processing, {} failed, {} stuck",
        stats.ready_pending_observations,
        stats.delayed_pending_observations,
        stats.processing_pending_observations,
        stats.expired_processing_pending_observations,
        stats.failed_pending_observations,
        stats.pending_jobs,
        stats.processing_jobs,
        stats.failed_jobs,
        stats.stuck_jobs,
    );

    let actions = queue_actions(
        stats.failed_pending_observations,
        stats.expired_processing_pending_observations,
        stats.failed_jobs,
        stats.stuck_jobs,
    );
    let action_suffix = render_inline_hints(&actions)
        .map(|hints| format!("; actions: {hints}"))
        .unwrap_or_default();

    if stats.expired_processing_pending_observations > 0 || stats.stuck_jobs > 0 {
        Check {
            name: "Pending queue",
            status: Status::Warn,
            detail: format!("{detail} (will auto-recover{action_suffix})"),
        }
    } else if stats.failed_pending_observations > 0 || stats.failed_jobs > 0 {
        Check {
            name: "Pending queue",
            status: Status::Warn,
            detail: format!("{detail} (inspect failures{action_suffix})"),
        }
    } else if stats.ready_pending_observations > 100 {
        Check {
            name: "Pending queue",
            status: Status::Warn,
            detail: format!("{detail} (backlog building up; action: `remem worker --once`)"),
        }
    } else {
        Check {
            name: "Pending queue",
            status: Status::Ok,
            detail,
        }
    }
}

pub(super) fn check_worker_daemon() -> Check {
    let conn = match db::open_db_read_only() {
        Ok(conn) => conn,
        Err(_) => {
            return Check {
                name: "Worker daemon",
                status: Status::Warn,
                detail: "cannot open database".to_string(),
            };
        }
    };

    let stats = match db::query_system_stats(&conn) {
        Ok(stats) => stats,
        Err(err) => {
            return Check {
                name: "Worker daemon",
                status: Status::Warn,
                detail: format!("cannot load heartbeat stats: {}", err),
            };
        }
    };

    match (
        stats.worker_daemon_healthy,
        stats.worker_heartbeat_owner,
        stats.worker_heartbeat_age_secs,
    ) {
        (true, Some(owner), Some(age_secs)) => Check {
            name: "Worker daemon",
            status: Status::Ok,
            detail: format!("healthy, last heartbeat {}s ago ({})", age_secs, owner),
        },
        (false, Some(owner), Some(age_secs)) => Check {
            name: "Worker daemon",
            status: Status::Warn,
            detail: format!(
                "stale, last heartbeat {}s ago ({}); {}",
                age_secs,
                owner,
                worker_once_fallback_detail()
            ),
        },
        _ => Check {
            name: "Worker daemon",
            status: Status::Ok,
            detail: format!("not running; {}", worker_once_fallback_detail()),
        },
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
