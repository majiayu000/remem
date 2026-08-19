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
const LEGACY_PENDING_HALT_NOTICE: &str = "automatic drain halted (no residual actionable rows)";

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
    } else {
        detail.push_str(&format!("; {LEGACY_PENDING_HALT_NOTICE}"));
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
