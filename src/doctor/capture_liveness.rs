use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};

use super::types::{Check, Status};
use crate::db;

const STALE_CAPTURE_HEARTBEAT_SECS: i64 = 7 * 24 * 60 * 60;
const SUMMARY_HEARTBEAT_GRACE_SECS: i64 = 60;
const ADMIN_REQUIRED_ARCHIVED_CANDIDATE_LIMIT: i64 = 5;
const ADMIN_REQUIRED_ARCHIVED_FIELD_DISPLAY_BYTES: usize = 80;

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
    let recoverable_archived_pending =
        match db::pending::admin::count_recoverable_archived_legacy_pending(conn) {
            Ok(count) => count,
            Err(err) => {
                if failures.is_empty() {
                    return Check::new(
                        "Capture liveness",
                        Status::Warn,
                        format!("cannot count recoverable archived pending observations: {err}"),
                    );
                }
                failures.push(format!(
                    "cannot count recoverable archived pending observations: {err}"
                ));
                return Check::new("Capture liveness", Status::Fail, failures.join("; "));
            }
        };
    let admin_required_archived_pending =
        match db::pending::admin::count_admin_required_archived_legacy_pending(conn) {
            Ok(count) => count,
            Err(err) => {
                if failures.is_empty() {
                    return Check::new(
                        "Capture liveness",
                        Status::Warn,
                        format!("cannot count admin-required archived pending observations: {err}"),
                    );
                }
                failures.push(format!(
                    "cannot count admin-required archived pending observations: {err}"
                ));
                return Check::new("Capture liveness", Status::Fail, failures.join("; "));
            }
        };
    let admin_required_archived_candidates = if admin_required_archived_pending > 0 {
        match db::pending::admin::list_admin_required_archived_legacy_pending(
            conn,
            ADMIN_REQUIRED_ARCHIVED_CANDIDATE_LIMIT,
        ) {
            Ok(candidates) => candidates,
            Err(err) => {
                failures.push(format!(
                    "{admin_required_archived_pending} admin-required archived pending observations, but cannot list recovery candidates: {err}"
                ));
                return Check::new("Capture liveness", Status::Fail, failures.join("; "));
            }
        }
    } else {
        Vec::new()
    };

    if stats.failed_pending_observations > 0
        || recoverable_archived_pending > 0
        || admin_required_archived_pending > 0
        || stats.failed_extraction_tasks > 0
    {
        let oldest_age = oldest_actionable_failure_age(&stats.failure_lifecycle)
            .map(|age| format!("; oldest actionable failure age={}s", age))
            .unwrap_or_default();
        let mut recovery: Vec<String> = Vec::new();
        if stats.failed_pending_observations > 0 || recoverable_archived_pending > 0 {
            recovery.push("a running `remem worker` auto-migrates eligible transient rows into the capture pipeline; run `remem worker --once` to drain one known-host batch now, including archived transient rows; for non-archived or unknown-host repair, run `remem pending list-failed --limit 20`, then preview/apply `remem pending retry-failed --dry-run` and `remem pending retry-failed`, followed by `remem pending migrate-legacy --dry-run --host claude-code` and `remem pending migrate-legacy --host claude-code` (or the corresponding `remem pending migrate-legacy --dry-run --host codex-cli` and `remem pending migrate-legacy --host codex-cli` commands)".to_string());
        }
        if admin_required_archived_pending > 0 {
            recovery.push(admin_required_archived_recovery_detail(
                &admin_required_archived_candidates,
                admin_required_archived_pending,
            ));
        }
        if stats.failed_extraction_tasks > 0 {
            recovery.push("run `remem worker --once` for failed extraction tasks".to_string());
        }
        failures.push(format!(
            "failed-observation backlog: {} actionable failed pending observations, {} recoverable archived transient pending observations, {} admin-required archived pending observations, {} actionable failed extraction tasks{}; {}",
            stats.failed_pending_observations,
            recoverable_archived_pending,
            admin_required_archived_pending,
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

fn admin_required_archived_recovery_detail(
    candidates: &[db::pending::admin::AdminRequiredArchivedLegacyPendingRow],
    total: usize,
) -> String {
    let details = candidates
        .iter()
        .map(admin_required_archived_candidate_detail)
        .collect::<Vec<_>>()
        .join("; ");
    format!(
        "admin-required archived candidates (showing {} of {total}, oldest first): {details}",
        candidates.len()
    )
}

fn admin_required_archived_candidate_detail(
    candidate: &db::pending::admin::AdminRequiredArchivedLegacyPendingRow,
) -> String {
    let failure_class = candidate
        .failure_class
        .as_deref()
        .map(bounded_debug_field)
        .unwrap_or_else(|| "<null>".to_string());
    let metadata = format!(
        "candidate id={} host={} failure_class={} archived_at_epoch={}",
        candidate.id,
        bounded_debug_field(&candidate.host),
        failure_class,
        candidate.archived_at_epoch
    );
    if matches!(
        candidate.host.as_str(),
        crate::runtime_config::CLAUDE_HOST | crate::runtime_config::CODEX_HOST
    ) {
        return format!(
            "{metadata}; preview `remem pending recover-archived --id {} --dry-run`; apply `remem pending recover-archived --id {}`",
            candidate.id, candidate.id
        );
    }
    format!(
        "{metadata}; unknown host requires explicit `--host`; preview `remem pending recover-archived --id {} --host claude-code --dry-run`; apply `remem pending recover-archived --id {} --host claude-code`; alternatively preview `remem pending recover-archived --id {} --host codex-cli --dry-run`; apply `remem pending recover-archived --id {} --host codex-cli`",
        candidate.id, candidate.id, candidate.id, candidate.id
    )
}

fn bounded_debug_field(value: &str) -> String {
    let truncated = db::truncate_str(value, ADMIN_REQUIRED_ARCHIVED_FIELD_DISPLAY_BYTES);
    if truncated.len() == value.len() {
        return format!("{truncated:?}");
    }
    let displayed = format!("{truncated}…");
    format!("{displayed:?}")
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
        assert!(check.detail.contains("`remem worker --once`"));
        assert!(check.detail.contains("`remem pending retry-failed`"));
        assert!(check
            .detail
            .contains("`remem pending migrate-legacy --host claude-code`"));
        assert!(check
            .detail
            .contains("`remem pending migrate-legacy --host codex-cli`"));
        Ok(())
    }

    #[test]
    fn capture_liveness_fails_on_recoverable_archived_observation_backlog() -> anyhow::Result<()> {
        let conn = setup_liveness_conn()?;
        let id = crate::db::test_support::insert_legacy_pending_fixture(
            &conn,
            "codex-cli",
            "sess-archived-transient",
            "/tmp/remem",
            "Bash",
            Some("{}"),
            Some("{}"),
            Some("/tmp/remem"),
        )?;
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "UPDATE pending_observations
             SET status = 'failed',
                 failure_class = 'transient',
                 failed_at_epoch = ?1,
                 archived_at_epoch = ?2
             WHERE id = ?3",
            params![now - 20 * 86_400, now - 86_400, id],
        )?;

        let check = check_capture_liveness(Some(&conn), &[]);

        assert!(matches!(check.status, Status::Fail));
        assert!(check.detail.contains("failed-observation backlog"));
        assert!(check
            .detail
            .contains("0 actionable failed pending observations"));
        assert!(check
            .detail
            .contains("1 recoverable archived transient pending observations"));
        assert!(check
            .detail
            .contains("a running `remem worker` auto-migrates"));
        assert!(check.detail.contains("run `remem worker --once`"));
        Ok(())
    }

    #[test]
    fn capture_liveness_fails_on_admin_required_archived_observation() -> anyhow::Result<()> {
        let conn = setup_liveness_conn()?;
        let unsafe_host = format!("unknown;\n{}", "h".repeat(120));
        let unsafe_failure_class = format!("permanent;\r{}", "f".repeat(120));
        let id = crate::db::test_support::insert_legacy_pending_fixture(
            &conn,
            &unsafe_host,
            "sess-archived-admin",
            "/tmp/remem",
            "Bash",
            Some("{}"),
            Some("{}"),
            Some("/tmp/remem"),
        )?;
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "UPDATE pending_observations
             SET status = 'failed',
                 failure_class = ?1,
                 failed_at_epoch = ?2,
                 archived_at_epoch = ?3,
                 updated_at_epoch = ?4
             WHERE id = ?5",
            params![
                unsafe_failure_class,
                now - 20 * 86_400,
                now - 86_400,
                now - 21 * 86_400,
                id
            ],
        )?;
        for index in 0..20 {
            let newer_id = crate::db::test_support::insert_legacy_pending_fixture(
                &conn,
                crate::runtime_config::CODEX_HOST,
                &format!("sess-newer-failed-{index}"),
                "/tmp/remem",
                "Bash",
                Some("{}"),
                Some("{}"),
                Some("/tmp/remem"),
            )?;
            conn.execute(
                "UPDATE pending_observations
                 SET status = 'failed',
                     failure_class = 'transient',
                     failed_at_epoch = ?1,
                     updated_at_epoch = ?1
                 WHERE id = ?2",
                params![now - index, newer_id],
            )?;
        }
        let mut additional_admin_ids = Vec::new();
        for index in 0..5 {
            let host = if index == 0 {
                crate::runtime_config::CODEX_HOST
            } else {
                "unknown"
            };
            let candidate_id = crate::db::test_support::insert_legacy_pending_fixture(
                &conn,
                host,
                &format!("sess-newer-admin-{index}"),
                "/tmp/remem",
                "Bash",
                Some("{}"),
                Some("{}"),
                Some("/tmp/remem"),
            )?;
            conn.execute(
                "UPDATE pending_observations
                 SET status = 'failed',
                     failure_class = 'permanent',
                     archived_at_epoch = ?1,
                     updated_at_epoch = ?1
                 WHERE id = ?2",
                params![now - 43_200 + index, candidate_id],
            )?;
            additional_admin_ids.push(candidate_id);
        }

        let global_recent = crate::db::pending::admin::list_failed(&conn, None, 20)?;
        assert_eq!(global_recent.len(), 20);
        assert!(
            global_recent.iter().all(|row| row.id != id),
            "the legacy global list must reproduce the hidden-candidate regression"
        );

        let check = check_capture_liveness(Some(&conn), &[]);

        assert!(matches!(check.status, Status::Fail));
        assert!(check
            .detail
            .contains("20 actionable failed pending observations"));
        assert!(check
            .detail
            .contains("6 admin-required archived pending observations"));
        assert!(check
            .detail
            .contains("admin-required archived candidates (showing 5 of 6, oldest first)"));
        assert!(check
            .detail
            .contains(&format!("candidate id={id} host=\"unknown;\\n")));
        assert!(check.detail.contains("failure_class=\"permanent;\\r"));
        assert!(check
            .detail
            .contains(&format!("archived_at_epoch={}", now - 86_400)));
        assert!(!check.detail.contains('\n'));
        assert!(!check.detail.contains(&unsafe_host));
        assert!(check
            .detail
            .contains("unknown host requires explicit `--host`"));
        assert!(check.detail.contains(&format!(
            "`remem pending recover-archived --id {id} --host claude-code --dry-run`"
        )));
        assert!(check.detail.contains(&format!(
            "`remem pending recover-archived --id {id} --host claude-code`"
        )));
        assert!(check.detail.contains(&format!(
            "`remem pending recover-archived --id {id} --host codex-cli --dry-run`"
        )));
        assert!(check.detail.contains(&format!(
            "`remem pending recover-archived --id {id} --host codex-cli`"
        )));
        let known_host_id = additional_admin_ids[0];
        assert!(check.detail.contains(&format!(
            "candidate id={known_host_id} host=\"codex-cli\" failure_class=\"permanent\""
        )));
        assert!(check.detail.contains(&format!(
            "`remem pending recover-archived --id {known_host_id} --dry-run`"
        )));
        assert!(check.detail.contains(&format!(
            "`remem pending recover-archived --id {known_host_id}`"
        )));
        assert!(!check
            .detail
            .contains(&format!("candidate id={}", additional_admin_ids[4])));
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
