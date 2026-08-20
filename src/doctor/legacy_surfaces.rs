use super::types::{Check, Status};
use crate::db;
use crate::db::pending::admin::LEGACY_PENDING_REMOVAL_VERSION;
use rusqlite::Connection;

fn legacy_pending_deprecation_notice() -> String {
    format!(
        "pending_observations is deprecated in remem 0.6.0 and scheduled for guarded removal no earlier than remem {LEGACY_PENDING_REMOVAL_VERSION}"
    )
}
const LEGACY_PENDING_MIGRATION_NOTICE: &str = "actionable pending_observations: preview with `remem pending migrate-legacy --dry-run`, then apply with `remem pending migrate-legacy`; if the legacy host is unknown, apply explicitly with `remem pending migrate-legacy --host claude-code` or `remem pending migrate-legacy --host codex-cli`";
const LEGACY_PENDING_HALT_NOTICE: &str =
    "automatic drain halted (no auto-recoverable residual rows)";
const LEGACY_PENDING_ACTIVE_NOTICE: &str =
    "automatic drain active (auto-recoverable residual rows remain)";

pub(super) fn check_legacy_surfaces(conn: Option<&Connection>) -> Check {
    let deprecation = legacy_pending_deprecation_notice();
    let Some(conn) = conn else {
        return Check::new(
            "Legacy surfaces",
            Status::Warn,
            format!("cannot open database; {deprecation}"),
        );
    };
    let stats = match db::query_system_stats(conn) {
        Ok(stats) => stats,
        Err(err) => {
            return Check::new(
                "Legacy surfaces",
                Status::Warn,
                format!("cannot load legacy surface stats: {err}; {deprecation}"),
            );
        }
    };
    let actionable_pending_rows =
        match db::pending::admin::count_legacy_migration_candidates(conn, None, i64::MAX) {
            Ok(count) => count,
            Err(err) => {
                return Check::new(
                    "Legacy surfaces",
                    Status::Warn,
                    format!("cannot count actionable legacy pending rows: {err}; {deprecation}"),
                )
            }
        };
    let auto_bridge_exhausted =
        match db::pending::admin::legacy_pending_auto_bridge_is_exhausted(conn) {
            Ok(exhausted) => exhausted,
            Err(err) => {
                return Check::new(
                    "Legacy surfaces",
                    Status::Warn,
                    format!("cannot load automatic legacy bridge state: {err}; {deprecation}"),
                )
            }
        };
    let mut detail = stats
        .legacy_surfaces
        .iter()
        .map(|surface| {
            let last_write = surface
                .last_write_epoch
                .map(|epoch| epoch.to_string())
                .unwrap_or_else(|| "none".to_string());
            format!(
                "{} rows={} disposition={} last_write_epoch={} frozen_write_violations={}",
                surface.surface,
                surface.row_count,
                surface.disposition,
                last_write,
                surface.frozen_write_violations
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    detail.push_str(&format!("; {deprecation}"));
    if actionable_pending_rows > 0 {
        detail.push_str(&format!("; {LEGACY_PENDING_MIGRATION_NOTICE}"));
    }
    if auto_bridge_exhausted {
        detail.push_str(&format!("; {LEGACY_PENDING_HALT_NOTICE}"));
    } else {
        detail.push_str(&format!("; {LEGACY_PENDING_ACTIVE_NOTICE}"));
    }
    let violations: i64 = stats
        .legacy_surfaces
        .iter()
        .map(|surface| surface.frozen_write_violations)
        .sum();
    if violations > 0 {
        Check::new(
            "Legacy surfaces",
            Status::Fail,
            format!("{detail}; retire/freeze blockers={violations}"),
        )
    } else {
        Check::new("Legacy surfaces", Status::Ok, detail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_transient_row_reports_active_auto_bridge() -> anyhow::Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        let id = crate::db::test_support::insert_legacy_pending_fixture(
            &conn,
            crate::runtime_config::CODEX_HOST,
            "doctor-auto-bridge",
            "alpha",
            "Bash",
            None,
            None,
            None,
        )?;
        conn.execute(
            "UPDATE pending_observations
             SET status = 'failed', failure_class = 'transient',
                 next_retry_epoch = 0, failed_at_epoch = 1
             WHERE id = ?1",
            [id],
        )?;

        let check = check_legacy_surfaces(Some(&conn));

        assert!(check.detail.contains(LEGACY_PENDING_ACTIVE_NOTICE));
        assert!(!check.detail.contains(LEGACY_PENDING_HALT_NOTICE));
        Ok(())
    }
}
