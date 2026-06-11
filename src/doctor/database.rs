use super::types::{Check, Status};
use crate::db;
use crate::doctor::health_action::{
    queue_actions, render_inline_hints, worker_once_fallback_detail,
};
use rusqlite::Connection;

pub(super) fn check_database(conn: Option<&Connection>, open_error: Option<&str>) -> Check {
    let db_path = db::db_path();
    if !db_path.exists() {
        return Check::new(
            "Database",
            Status::Fail,
            format!("{} (not found)", db_path.display()),
        );
    }

    let size = std::fs::metadata(&db_path)
        .map(|meta| meta.len())
        .unwrap_or(0);
    let Some(conn) = conn else {
        return Check::new(
            "Database",
            Status::Fail,
            format!(
                "{} (open error: {})",
                db_path.display(),
                open_error.unwrap_or("database connection unavailable")
            ),
        );
    };

    match db::query_system_stats(conn) {
        Ok(stats) => Check::new(
            "Database",
            Status::Ok,
            format!(
                "{} ({:.1} MB, {} memories)",
                db_path.display(),
                size as f64 / 1_048_576.0,
                stats.active_memories
            ),
        ),
        Err(err) => Check::new(
            "Database",
            Status::Fail,
            format!("{} (stats error: {})", db_path.display(), err),
        ),
    }
}

pub(super) fn check_pending_queue(conn: Option<&Connection>) -> Check {
    let Some(conn) = conn else {
        return Check::new("Pending queue", Status::Warn, "cannot open database");
    };

    let stats = match db::query_system_stats(conn) {
        Ok(stats) => stats,
        Err(err) => {
            return Check::new(
                "Pending queue",
                Status::Warn,
                format!("cannot load queue stats: {}", err),
            );
        }
    };
    let detail = format!(
        "{} ready, {} delayed, {} processing ({} expired), {} failed pending; {} extraction tasks pending, {} processing ({} expired), {} failed; {} jobs pending, {} processing, {} failed, {} stuck",
        stats.ready_pending_observations,
        stats.delayed_pending_observations,
        stats.processing_pending_observations,
        stats.expired_processing_pending_observations,
        stats.failed_pending_observations,
        stats.pending_extraction_tasks,
        stats.processing_extraction_tasks,
        stats.expired_processing_extraction_tasks,
        stats.failed_extraction_tasks,
        stats.pending_jobs,
        stats.processing_jobs,
        stats.failed_jobs,
        stats.stuck_jobs,
    );

    let actions = queue_actions(
        stats.failed_pending_observations,
        stats.expired_processing_pending_observations,
        stats.expired_processing_extraction_tasks,
        stats.failed_jobs,
        stats.stuck_jobs,
        stats.failed_extraction_tasks,
    );
    let action_suffix = render_inline_hints(&actions)
        .map(|hints| format!("; actions: {hints}"))
        .unwrap_or_default();

    if stats.expired_processing_pending_observations > 0
        || stats.expired_processing_extraction_tasks > 0
        || stats.stuck_jobs > 0
    {
        Check::new(
            "Pending queue",
            Status::Warn,
            format!("{detail} (will auto-recover{action_suffix})"),
        )
    } else if stats.failed_pending_observations > 0
        || stats.failed_jobs > 0
        || stats.failed_extraction_tasks > 0
    {
        Check::new(
            "Pending queue",
            Status::Warn,
            format!("{detail} (inspect failures{action_suffix})"),
        )
    } else if stats.ready_pending_observations > 100 || stats.pending_extraction_tasks > 100 {
        Check::new(
            "Pending queue",
            Status::Warn,
            format!("{detail} (backlog building up; action: `remem worker --once`)"),
        )
    } else {
        Check::new("Pending queue", Status::Ok, detail)
    }
}

pub(super) fn check_raw_archive_ingest(conn: Option<&Connection>) -> Check {
    let Some(conn) = conn else {
        return Check::new("Raw archive ingest", Status::Warn, "cannot open database");
    };

    let stats = match db::query_system_stats(conn) {
        Ok(stats) => stats,
        Err(err) => {
            return Check::new(
                "Raw archive ingest",
                Status::Warn,
                format!("cannot load raw ingest stats: {}", err),
            );
        }
    };

    if stats.raw_ingest_failures == 0 {
        return Check::new(
            "Raw archive ingest",
            Status::Ok,
            format!("{} raw messages, no ingest failures", stats.raw_messages),
        );
    }

    let mut detail = format!(
        "{} failure(s), parse_errors={}, insert_errors={}",
        stats.raw_ingest_failures, stats.raw_ingest_parse_errors, stats.raw_ingest_insert_errors
    );
    if let Some(kind) = stats.latest_raw_ingest_failure_kind {
        detail.push_str(&format!("; latest={kind}"));
    }
    if let Some(path) = stats.latest_raw_ingest_failure_path {
        detail.push_str(&format!(" path={path}"));
    }
    if let Some(message) = stats.latest_raw_ingest_failure_message {
        detail.push_str(&format!(" ({})", crate::db::truncate_str(&message, 160)));
    }

    Check::new("Raw archive ingest", Status::Warn, detail)
}

pub(super) fn check_capture_drops(conn: Option<&Connection>) -> Check {
    let Some(conn) = conn else {
        return Check::new("Capture drops", Status::Warn, "cannot open database");
    };

    let stats = match db::query_system_stats(conn) {
        Ok(stats) => stats,
        Err(err) => {
            return Check::new(
                "Capture drops",
                Status::Warn,
                format!("cannot load capture drop stats: {}", err),
            );
        }
    };

    if stats.actionable_capture_drops == 0 {
        return Check::new(
            "Capture drops",
            Status::Ok,
            format!(
                "{} expected hook skip/drop event(s), no actionable capture drops",
                stats.capture_drop_events
            ),
        );
    }

    let mut detail = format!(
        "{} actionable capture drop(s), {} total recorded hook skip/drop event(s)",
        stats.actionable_capture_drops, stats.capture_drop_events
    );
    if stats.unrecovered_capture_spills > 0 {
        detail.push_str(&format!(
            ", {} unrecovered capture spill(s)",
            stats.unrecovered_capture_spills
        ));
    }
    if let Some(reason) = stats.latest_capture_drop_reason {
        detail.push_str(&format!(", latest reason={reason}"));
    }
    if let Some(drop_detail) = stats.latest_capture_drop_detail {
        detail.push_str(&format!(", detail={}", db::truncate_str(&drop_detail, 160)));
    }

    Check::new("Capture drops", Status::Warn, detail)
}

pub(super) fn check_temporal_facts(conn: Option<&Connection>) -> Check {
    let Some(conn) = conn else {
        return Check::new("Temporal facts", Status::Warn, "cannot open database");
    };

    let stats = match db::query_memory_facts_stats(conn) {
        Ok(stats) => stats,
        Err(err) => {
            return Check::new(
                "Temporal facts",
                Status::Warn,
                format!("cannot load fact stats: {}", err),
            );
        }
    };

    if !stats.table_exists {
        return Check::new(
            "Temporal facts",
            Status::Ok,
            "memory_facts table not present; temporal retrieval uses created_at fallback",
        );
    }

    if stats.retrieval_eligible == 0 && stats.active_memories == 0 && stats.captured_events == 0 {
        return Check::new(
            "Temporal facts",
            Status::Ok,
            "memory_facts table is empty because this store has no memories or captured events yet",
        );
    }

    if stats.retrieval_eligible == 0 {
        return Check::new(
            "Temporal facts",
            Status::Warn,
            format!(
                "temporal retrieval can read memory_facts, but 0 of {} fact row(s) are linked active event-time facts; production fact extraction is not populating retrievable facts yet",
                stats.total
            ),
        );
    }

    Check::new(
        "Temporal facts",
        Status::Ok,
        format!(
            "{} linked active memory fact(s) available for event-time retrieval",
            stats.retrieval_eligible
        ),
    )
}

pub(super) fn check_worker_daemon(conn: Option<&Connection>) -> Check {
    let Some(conn) = conn else {
        return Check::new("Worker daemon", Status::Warn, "cannot open database");
    };

    let stats = match db::query_system_stats(conn) {
        Ok(stats) => stats,
        Err(err) => {
            return Check::new(
                "Worker daemon",
                Status::Warn,
                format!("cannot load heartbeat stats: {}", err),
            );
        }
    };

    match (
        stats.worker_daemon_healthy,
        stats.worker_heartbeat_owner,
        stats.worker_heartbeat_age_secs,
    ) {
        (true, Some(owner), Some(age_secs)) => Check::new(
            "Worker daemon",
            Status::Ok,
            format!("healthy, last heartbeat {}s ago ({})", age_secs, owner),
        ),
        (false, Some(owner), Some(age_secs)) => Check::new(
            "Worker daemon",
            Status::Warn,
            format!(
                "stale, last heartbeat {}s ago ({}); {}",
                age_secs,
                owner,
                worker_once_fallback_detail()
            ),
        ),
        _ => Check::new(
            "Worker daemon",
            Status::Ok,
            format!("not running; {}", worker_once_fallback_detail()),
        ),
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
