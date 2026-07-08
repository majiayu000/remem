use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};

use super::types::{Check, Status};
use crate::db;

const STALE_CAPTURE_HEARTBEAT_SECS: i64 = 7 * 24 * 60 * 60;
const SUMMARY_HEARTBEAT_GRACE_SECS: i64 = 60;

pub(super) fn check_capture_liveness(conn: Option<&Connection>, setup_checks: &[Check]) -> Check {
    let setup_findings = capture_setup_findings(setup_checks);
    let mut failures: Vec<String> = setup_findings
        .iter()
        .filter(|finding| matches!(finding.status, Status::Fail))
        .map(|finding| finding.detail.clone())
        .collect();
    let warnings: Vec<String> = setup_findings
        .iter()
        .filter(|finding| matches!(finding.status, Status::Warn))
        .map(|finding| finding.detail.clone())
        .collect();

    let Some(conn) = conn else {
        if failures.is_empty() {
            return Check::new(
                "Capture liveness",
                Status::Warn,
                join_detail(&warnings, "cannot open database"),
            );
        }
        return Check::new("Capture liveness", Status::Fail, failures.join("; "));
    };

    let stats = match db::query_system_stats(conn) {
        Ok(stats) => stats,
        Err(err) => {
            if failures.is_empty() {
                return Check::new(
                    "Capture liveness",
                    Status::Warn,
                    format!("cannot load capture stats: {}", err),
                );
            }
            failures.push(format!("cannot load capture stats: {err}"));
            return Check::new("Capture liveness", Status::Fail, failures.join("; "));
        }
    };

    if stats.failed_pending_observations > 0 || stats.failed_extraction_tasks > 0 {
        let oldest_age = oldest_actionable_failure_age(&stats.failure_lifecycle)
            .map(|age| format!("; oldest actionable failure age={}s", age))
            .unwrap_or_default();
        let mut recovery = Vec::new();
        if stats.failed_pending_observations > 0 {
            recovery.push("run `remem pending list-failed --limit 20`; preview and apply migration prep with `remem pending retry-failed --dry-run` then `remem pending retry-failed`; replay with `remem pending migrate-legacy --dry-run` then `remem pending migrate-legacy`; if replay reports host='unknown', rerun with `remem pending migrate-legacy --host claude-code` or `remem pending migrate-legacy --host codex-cli`");
        }
        if stats.failed_extraction_tasks > 0 {
            recovery.push("run `remem worker --once` for failed extraction tasks");
        }
        failures.push(format!(
            "failed-observation backlog: {} actionable failed pending observations, {} actionable failed extraction tasks{}; {}",
            stats.failed_pending_observations,
            stats.failed_extraction_tasks,
            oldest_age,
            recovery.join("; ")
        ));
    }
    if stats.actionable_capture_drops > 0 {
        failures.push(format!(
            "{} actionable capture drop(s); latest reason={}",
            stats.actionable_capture_drops,
            stats
                .latest_capture_drop_reason
                .as_deref()
                .unwrap_or("unknown")
        ));
    }

    let liveness = match query_liveness_rows(conn) {
        Ok(liveness) => liveness,
        Err(err) => {
            if failures.is_empty() {
                return Check::new(
                    "Capture liveness",
                    Status::Warn,
                    format!("cannot load capture liveness rows: {}", err),
                );
            }
            failures.push(format!("cannot load capture liveness rows: {err}"));
            return Check::new("Capture liveness", Status::Fail, failures.join("; "));
        }
    };
    if let Some(gap) = liveness.hosted_summary_without_capture {
        failures.push(format!(
            "{} hosted session summary row(s) have no captured_events heartbeat; latest session_row_id={} host={}",
            gap.count,
            gap.latest_session_row_id.unwrap_or_default(),
            gap.latest_host.unwrap_or_else(|| "unknown".to_string())
        ));
    }
    if stats.session_summaries > 0 && stats.latest_capture_activity_epoch.is_none() {
        failures.push(format!(
            "{} completed session summary row(s), but no captured_events/raw_messages/capture_drop_events heartbeat; hooks are not recording capture activity",
            stats.session_summaries
        ));
    }
    if let (Some(summary_epoch), Some(heartbeat_epoch)) = (
        liveness.latest_session_summary_epoch,
        stats.latest_capture_activity_epoch,
    ) {
        if summary_epoch.saturating_sub(heartbeat_epoch) > SUMMARY_HEARTBEAT_GRACE_SECS {
            failures.push(format!(
                "completed session summary is newer than latest capture heartbeat by {}s; hooks may be missing or stale",
                summary_epoch.saturating_sub(heartbeat_epoch)
            ));
        }
    }

    if !failures.is_empty() {
        return Check::new("Capture liveness", Status::Fail, failures.join("; "));
    }
    if stats.latest_capture_activity_epoch.is_none() {
        return Check::new(
            "Capture liveness",
            Status::Warn,
            join_detail(
                &warnings,
                "no capture heartbeat yet; run one host session and re-run doctor",
            ),
        );
    }

    let heartbeat_age_secs = chrono::Utc::now()
        .timestamp()
        .saturating_sub(stats.latest_capture_activity_epoch.unwrap_or_default());
    let detail = format!(
        "latest capture heartbeat {}s ago; captured_events={}, raw_messages={}, expected drops={}",
        heartbeat_age_secs, stats.captured_events, stats.raw_messages, stats.capture_drop_events
    );
    if heartbeat_age_secs > STALE_CAPTURE_HEARTBEAT_SECS {
        return Check::new(
            "Capture liveness",
            Status::Warn,
            join_detail(
                &warnings,
                &format!(
                    "{detail}; stale capture heartbeat exceeds {}s, run one host session and re-run doctor",
                    STALE_CAPTURE_HEARTBEAT_SECS
                ),
            ),
        );
    }
    if !warnings.is_empty() {
        return Check::new(
            "Capture liveness",
            Status::Warn,
            join_detail(&warnings, &detail),
        );
    }

    Check::new("Capture liveness", Status::Ok, detail)
}

fn oldest_actionable_failure_age(stats: &db::FailureLifecycleStats) -> Option<i64> {
    let now = chrono::Utc::now().timestamp();
    [
        stats.pending_observation.oldest_actionable_epoch,
        stats.extraction_task.oldest_actionable_epoch,
        stats.extraction_replay_range.oldest_actionable_epoch,
        stats.job.oldest_actionable_epoch,
    ]
    .into_iter()
    .flatten()
    .min()
    .map(|epoch| now.saturating_sub(epoch))
}

#[derive(Clone, PartialEq, Eq)]
struct SetupFinding {
    status: Status,
    detail: String,
}

fn capture_setup_findings(checks: &[Check]) -> Vec<SetupFinding> {
    checks
        .iter()
        .filter_map(|check| {
            if check.name.starts_with("Hooks") {
                return hook_setup_finding(check);
            }
            if check.name == "Install paths" {
                return install_path_setup_finding(check);
            }
            None
        })
        .collect()
}

fn hook_setup_finding(check: &Check) -> Option<SetupFinding> {
    match check.status {
        Status::Fail => Some(SetupFinding {
            status: Status::Fail,
            detail: format!("{} failed: {}", check.name, check.detail),
        }),
        Status::Warn if hook_warning_blocks_capture(&check.detail) => Some(SetupFinding {
            status: Status::Fail,
            detail: format!("{} stale or incomplete: {}", check.name, check.detail),
        }),
        _ => None,
    }
}

fn hook_warning_blocks_capture(detail: &str) -> bool {
    detail.contains(" registered (run `remem install --target")
}

fn install_path_setup_finding(check: &Check) -> Option<SetupFinding> {
    match check.status {
        Status::Fail => Some(SetupFinding {
            status: Status::Fail,
            detail: format!("Install paths failed: {}", check.detail),
        }),
        Status::Warn => Some(SetupFinding {
            status: Status::Warn,
            detail: format!("Install paths warning: {}", check.detail),
        }),
        Status::Ok => None,
    }
}

fn join_detail(prefixes: &[String], detail: &str) -> String {
    if prefixes.is_empty() {
        detail.to_string()
    } else {
        format!("{}; {detail}", prefixes.join("; "))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CaptureLivenessRows {
    latest_session_summary_epoch: Option<i64>,
    hosted_summary_without_capture: Option<HostedSummaryGap>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HostedSummaryGap {
    count: i64,
    latest_session_row_id: Option<i64>,
    latest_host: Option<String>,
}

fn query_liveness_rows(conn: &Connection) -> Result<CaptureLivenessRows> {
    let latest_session_summary_epoch = conn.query_row(
        "SELECT MAX(created_at_epoch)
         FROM session_summaries
         WHERE created_at_epoch IS NOT NULL",
        [],
        |row| row.get::<_, Option<i64>>(0),
    )?;
    let hosted_summary_without_capture = query_hosted_summary_without_capture(conn)?;
    Ok(CaptureLivenessRows {
        latest_session_summary_epoch,
        hosted_summary_without_capture,
    })
}

fn query_hosted_summary_without_capture(conn: &Connection) -> Result<Option<HostedSummaryGap>> {
    let count = conn.query_row(
        "SELECT COUNT(*)
         FROM session_summaries ss
         WHERE ss.session_row_id IS NOT NULL
           AND NOT EXISTS (
               SELECT 1 FROM captured_events ce
               WHERE ce.session_row_id = ss.session_row_id
           )",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    if count == 0 {
        return Ok(None);
    }
    let latest = conn
        .query_row(
            "SELECT ss.session_row_id, h.name
             FROM session_summaries ss
             LEFT JOIN hosts h ON h.id = ss.host_id
             WHERE ss.session_row_id IS NOT NULL
               AND NOT EXISTS (
                   SELECT 1 FROM captured_events ce
                   WHERE ce.session_row_id = ss.session_row_id
               )
             ORDER BY ss.created_at_epoch DESC, ss.id DESC
             LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get::<_, Option<i64>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            },
        )
        .optional()?;
    Ok(Some(HostedSummaryGap {
        count,
        latest_session_row_id: latest.as_ref().and_then(|row| row.0),
        latest_host: latest.and_then(|row| row.1),
    }))
}

#[cfg(test)]
mod tests {
    use rusqlite::params;

    use super::*;

    fn setup_liveness_conn() -> anyhow::Result<Connection> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        Ok(conn)
    }

    fn record_liveness_capture(conn: &Connection) -> anyhow::Result<()> {
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
    fn capture_liveness_fails_on_failed_observation_backlog() -> anyhow::Result<()> {
        let conn = setup_liveness_conn()?;
        let id = crate::db::test_support::insert_legacy_pending_fixture(
            &conn,
            "codex-cli",
            "sess-failed",
            "/tmp/remem",
            "Bash",
            Some("{}"),
            Some("{}"),
            Some("/tmp/remem"),
        )?;
        conn.execute(
            "UPDATE pending_observations SET status = 'failed' WHERE id = ?1",
            [id],
        )?;

        let check = check_capture_liveness(Some(&conn), &[]);

        assert!(matches!(check.status, Status::Fail));
        assert!(check.detail.contains("failed-observation backlog"));
        assert!(check.detail.contains("failed pending observations"));
        assert!(check.detail.contains("`remem pending retry-failed`"));
        assert!(check.detail.contains("`remem pending migrate-legacy`"));
        assert!(check
            .detail
            .contains("`remem pending migrate-legacy --host claude-code`"));
        assert!(check
            .detail
            .contains("`remem pending migrate-legacy --host codex-cli`"));
        Ok(())
    }

    #[test]
    fn capture_liveness_fails_when_summary_is_newer_than_heartbeat() -> anyhow::Result<()> {
        let conn = setup_liveness_conn()?;
        record_liveness_capture(&conn)?;
        conn.execute(
            "INSERT INTO session_summaries
             (memory_session_id, project, request, completed, created_at_epoch)
             VALUES ('legacy-newer-summary', '/tmp/remem-doctor', 'done', 'done', strftime('%s', 'now') + 120)",
            [],
        )?;

        let check = check_capture_liveness(Some(&conn), &[]);

        assert!(matches!(check.status, Status::Fail));
        assert!(check
            .detail
            .contains("summary is newer than latest capture heartbeat"));
        Ok(())
    }

    #[test]
    fn capture_liveness_fails_when_hosted_summary_has_no_capture() -> anyhow::Result<()> {
        let conn = setup_liveness_conn()?;
        conn.execute(
            "INSERT OR IGNORE INTO hosts(name, enabled, created_at_epoch)
             VALUES ('codex-cli', 1, strftime('%s', 'now'))",
            [],
        )?;
        let host_id: i64 =
            conn.query_row("SELECT id FROM hosts WHERE name = 'codex-cli'", [], |row| {
                row.get(0)
            })?;
        conn.execute(
            "INSERT INTO session_summaries
             (memory_session_id, project, request, completed, created_at_epoch, host_id, session_row_id)
             VALUES ('hosted-without-capture', '/tmp/remem-doctor', 'done', 'done',
                     strftime('%s', 'now'), ?1, 9876)",
            params![host_id],
        )?;

        let check = check_capture_liveness(Some(&conn), &[]);

        assert!(matches!(check.status, Status::Fail));
        assert!(check.detail.contains("hosted session summary"));
        assert!(check.detail.contains("session_row_id=9876"));
        Ok(())
    }

    #[test]
    fn capture_liveness_fails_on_missing_hook_setup() {
        let setup = vec![Check::new(
            "Hooks (codex)",
            Status::Fail,
            "no remem hooks (run `remem install --target codex`)",
        )];

        let check = check_capture_liveness(None, &setup);

        assert!(matches!(check.status, Status::Fail));
        assert!(check.detail.contains("Hooks (codex) failed"));
    }

    #[test]
    fn capture_liveness_fails_on_partial_hook_setup() {
        let setup = vec![Check::new(
            "Hooks (codex)",
            Status::Warn,
            "1/2 registered (run `remem install --target codex` to fix)",
        )];

        let check = check_capture_liveness(None, &setup);

        assert!(matches!(check.status, Status::Fail));
        assert!(check.detail.contains("stale or incomplete"));
    }

    #[test]
    fn capture_liveness_warns_on_stale_install_path_setup() -> anyhow::Result<()> {
        let conn = setup_liveness_conn()?;
        let setup = vec![Check::new(
            "Install paths",
            Status::Warn,
            "2 remem executable(s) found; configured /opt/remem; candidates: /usr/local/bin/remem (0.5.1); fix: remove or upgrade stale installs",
        )];

        let check = check_capture_liveness(Some(&conn), &setup);

        assert!(matches!(check.status, Status::Warn));
        assert!(check.detail.contains("Install paths warning"));
        assert!(check.detail.contains("no capture heartbeat yet"));
        Ok(())
    }
}
