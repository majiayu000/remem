use super::types::{Check, Status};
use crate::db;
use crate::doctor::health_action::{
    queue_actions, queue_actions_with_replay, render_inline_hints, worker_once_fallback_detail,
};
use rusqlite::{Connection, OptionalExtension};

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
    let replay_detail = if stats.retryable_extraction_replay_ranges > 0
        || stats.active_extraction_replay_ranges > 0
        || stats.quarantined_extraction_replay_ranges > 0
    {
        format!(
            "; {} extraction replay ranges retryable, {} active, {} quarantined",
            stats.retryable_extraction_replay_ranges,
            stats.active_extraction_replay_ranges,
            stats.quarantined_extraction_replay_ranges
        )
    } else {
        String::new()
    };
    let detail = format!("{detail}{replay_detail}");

    let actions = if stats.retryable_extraction_replay_ranges > 0 {
        queue_actions_with_replay(
            stats.failed_pending_observations,
            stats.expired_processing_pending_observations,
            stats.expired_processing_extraction_tasks,
            stats.failed_jobs,
            stats.stuck_jobs,
            stats.failed_extraction_tasks,
            stats.retryable_extraction_replay_ranges,
        )
    } else {
        queue_actions(
            stats.failed_pending_observations,
            stats.expired_processing_pending_observations,
            stats.expired_processing_extraction_tasks,
            stats.failed_jobs,
            stats.stuck_jobs,
            stats.failed_extraction_tasks,
        )
    };
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
        || stats.retryable_extraction_replay_ranges > 0
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

pub(super) fn check_declared_empty_surfaces(conn: Option<&Connection>) -> Check {
    let Some(conn) = conn else {
        return Check::new(
            "Declared-empty surfaces",
            Status::Warn,
            "cannot open database",
        );
    };

    let stats = match db::query_system_stats(conn) {
        Ok(stats) => stats,
        Err(err) => {
            return Check::new(
                "Declared-empty surfaces",
                Status::Warn,
                format!("cannot load stats: {}", err),
            );
        }
    };
    let memory_facts = match table_count(conn, "memory_facts") {
        Ok(count) => count,
        Err(err) => {
            return Check::new(
                "Declared-empty surfaces",
                Status::Warn,
                format!("cannot count memory_facts: {err}"),
            );
        }
    };
    let graph_edges = match table_count(conn, "graph_edges") {
        Ok(count) => count,
        Err(err) => {
            return Check::new(
                "Declared-empty surfaces",
                Status::Warn,
                format!("cannot count graph_edges: {err}"),
            );
        }
    };
    let rule_candidates = match table_count(conn, "rule_candidates") {
        Ok(count) => count,
        Err(err) => {
            return Check::new(
                "Declared-empty surfaces",
                Status::Warn,
                format!("cannot count rule_candidates: {err}"),
            );
        }
    };

    let mut findings = Vec::new();
    if memory_facts == 0 && (stats.active_memories > 0 || stats.captured_events > 0) {
        findings.push("memory_facts=0 despite memory/event source data");
    }
    if graph_edges == 0 && (stats.active_memories > 0 || stats.captured_events > 0) {
        findings.push("graph_edges=0 despite graph schema/read path");
    }
    if rule_candidates == 0 && stats.captured_events > 0 {
        findings.push("rule_candidates=0 despite captured_events source data");
    }

    if findings.is_empty() {
        Check::new(
            "Declared-empty surfaces",
            Status::Ok,
            "no declared-but-empty production surface found",
        )
    } else {
        Check::new("Declared-empty surfaces", Status::Warn, findings.join("; "))
    }
}

pub(super) fn check_promotion_funnel(conn: Option<&Connection>) -> Check {
    let Some(conn) = conn else {
        return Check::new("Promotion funnel", Status::Warn, "cannot open database");
    };

    let stats = match db::query_system_stats(conn) {
        Ok(stats) => stats,
        Err(err) => {
            return Check::new(
                "Promotion funnel",
                Status::Warn,
                format!("cannot load funnel stats: {}", err),
            );
        }
    };
    let detail = format!(
        "captured_events={} -> observations={} ({:.1}%) -> candidates={} ({:.1}%) -> promoted={} ({:.1}%), pending_review={} ({:.1}%)",
        stats.captured_events,
        stats.total_observations,
        percent(stats.total_observations, stats.captured_events),
        stats.total_memory_candidates,
        percent(stats.total_memory_candidates, stats.total_observations),
        stats.promoted_memory_candidates,
        percent(stats.promoted_memory_candidates, stats.total_memory_candidates),
        stats.pending_review_memory_candidates,
        percent(stats.pending_review_memory_candidates, stats.total_memory_candidates),
    );

    if stats.captured_events > 0 && stats.total_observations == 0 {
        Check::new(
            "Promotion funnel",
            Status::Warn,
            format!("{detail}; extraction is not producing observations"),
        )
    } else if stats.total_observations > 0 && stats.total_memory_candidates == 0 {
        Check::new(
            "Promotion funnel",
            Status::Warn,
            format!("{detail}; promotion is not producing candidates"),
        )
    } else {
        Check::new("Promotion funnel", Status::Ok, detail)
    }
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

    if stats.total == 0
        && stats.retrieval_eligible == 0
        && stats.active_memories == 0
        && stats.captured_events == 0
    {
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

fn percent(numerator: i64, denominator: i64) -> f64 {
    if denominator <= 0 {
        0.0
    } else {
        (numerator as f64 * 100.0) / denominator as f64
    }
}

fn table_count(conn: &Connection, table: &str) -> Result<i64, rusqlite::Error> {
    let exists: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table],
            |row| row.get(0),
        )
        .optional()?;
    if exists.is_none() {
        return Ok(0);
    }
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_conn() -> anyhow::Result<Connection> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        Ok(conn)
    }

    fn record_capture(conn: &Connection) -> anyhow::Result<()> {
        crate::db::record_captured_event(
            conn,
            &crate::db::CaptureEventInput {
                host: "codex-cli",
                session_id: "sess-doctor",
                project: "/tmp/remem-doctor",
                cwd: Some("/tmp/remem-doctor"),
                event_type: "message",
                role: Some("user"),
                tool_name: None,
                content: "captured event",
                task_kind: None,
            },
        )?;
        Ok(())
    }

    #[test]
    fn declared_empty_surfaces_warns_when_source_data_exists_without_rows() -> anyhow::Result<()> {
        let conn = setup_conn()?;
        crate::memory::insert_memory(
            &conn,
            Some("sess"),
            "/tmp/remem",
            None,
            "decision",
            "source memory",
            "decision",
            None,
        )?;
        record_capture(&conn)?;

        let check = check_declared_empty_surfaces(Some(&conn));

        assert!(matches!(check.status, Status::Warn));
        assert!(check.detail.contains("memory_facts=0"));
        assert!(check.detail.contains("graph_edges=0"));
        assert!(check.detail.contains("rule_candidates=0"));
        Ok(())
    }

    #[test]
    fn promotion_funnel_warns_when_observations_do_not_create_candidates() -> anyhow::Result<()> {
        let conn = setup_conn()?;
        record_capture(&conn)?;
        crate::db::insert_observation(
            &conn,
            "sess-doctor",
            "/tmp/remem-doctor",
            "feature",
            Some("observed feature"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            1,
        )?;

        let check = check_promotion_funnel(Some(&conn));

        assert!(matches!(check.status, Status::Warn));
        assert!(check.detail.contains("captured_events=1 -> observations=1"));
        assert!(check
            .detail
            .contains("promotion is not producing candidates"));
        Ok(())
    }
}
