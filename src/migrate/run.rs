use std::io::Write;

use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection};

use super::schema_drift::{install_v031_state_delete_trigger, repair_known_schema_drift};
use super::state::{applied_versions, ensure_migration_table, has_migration_table, mark_applied};
use super::transition::transition_from_old_system;
use super::types::{MIGRATIONS, OLD_BASELINE_VERSION};

pub fn run_migrations(conn: &Connection) -> Result<()> {
    ensure_migration_table(conn)?;

    conn.execute_batch("BEGIN IMMEDIATE")
        .context("begin migration transaction")?;
    let result = run_migrations_locked(conn);
    match result {
        Ok(()) => {
            conn.execute_batch("COMMIT")
                .context("commit migration transaction")?;
            Ok(())
        }
        Err(error) => {
            if let Err(rollback_error) = conn.execute_batch("ROLLBACK") {
                // A failed rollback can leave the database in a partially-migrated
                // state, so this is never a benign warning (U-29). Surface it at
                // error level and keep both error chains.
                crate::log::error(
                    "migrate",
                    &format!(
                        "rollback failed after migration error: {rollback_error}; \
                         original migration error: {error:#}"
                    ),
                );
                return Err(
                    error.context(format!("migration rollback also failed: {rollback_error}"))
                );
            }
            Err(error)
        }
    }
}

pub(crate) fn ensure_schema_current(conn: &Connection) -> Result<()> {
    if !has_migration_table(conn) {
        return Err(anyhow!(
            "database schema is not initialized; run a foreground remem command before hook context"
        ));
    }
    let applied = applied_versions(conn)?;
    let binary_latest = super::latest_schema_version();
    let db_latest = applied.iter().copied().max().unwrap_or(0);
    if db_latest > binary_latest {
        return Err(anyhow!(
            "database is at schema v{db_latest} but this binary ({}) only knows up to v{binary_latest}; please upgrade remem and verify `remem --version` reports schema v{db_latest} or newer",
            crate::build_info::version_label()
        ));
    }
    if db_latest < binary_latest {
        return Err(anyhow!(
            "database is at schema v{db_latest} but this binary ({}) requires schema v{binary_latest}; run a foreground remem command to migrate before hook context",
            crate::build_info::version_label()
        ));
    }
    if let Some(missing) = MIGRATIONS
        .iter()
        .find(|migration| !applied.contains(&migration.version))
    {
        return Err(anyhow!(
            "database schema is missing migration v{:03}_{}; run a foreground remem command to migrate before hook context",
            missing.version,
            missing.name
        ));
    }
    let invariant_errors = super::validate_schema_invariants(conn)?;
    if !invariant_errors.is_empty() {
        return Err(anyhow!(
            "database schema drift requires foreground migration: {}",
            invariant_errors.join("; ")
        ));
    }
    Ok(())
}

fn run_migrations_locked(conn: &Connection) -> Result<()> {
    transition_from_old_system(conn)?;

    let applied = applied_versions(conn)?;
    let binary_latest = super::latest_schema_version();
    let db_latest = applied.iter().copied().max().unwrap_or(0);
    if db_latest > binary_latest {
        return Err(anyhow!(
            "database is at schema v{db_latest} but this binary ({}) only knows up to v{binary_latest}; please upgrade remem and verify `remem --version` reports schema v{db_latest} or newer",
            crate::build_info::version_label()
        ));
    }
    for repair in repair_known_schema_drift(conn, &applied)? {
        crate::log::info("migrate", &format!("repaired schema drift: {repair}"));
    }
    for migration in MIGRATIONS {
        if applied.contains(&migration.version) {
            continue;
        }
        crate::log::info(
            "migrate",
            &format!("applying v{:03}_{}", migration.version, migration.name),
        );
        conn.execute_batch(migration.sql).with_context(|| {
            format!(
                "migration v{:03}_{} failed",
                migration.version, migration.name
            )
        })?;
        run_post_migration_hook(conn, migration.version, migration.name)?;
        mark_applied(conn, migration.version, migration.name)?;
        crate::log::info(
            "migrate",
            &format!("applied v{:03}_{}", migration.version, migration.name),
        );
    }

    // Keep PRAGMA user_version consistent with what `_schema_migrations`
    // actually records: derive it from the highest applied migration, not from
    // the binary's latest known version. Using the binary version here would
    // claim a schema level the database may not have reached if a later
    // migration was never applied (#244).
    let max_applied = applied_versions(conn)?
        .into_iter()
        .max()
        .unwrap_or(OLD_BASELINE_VERSION);
    let current_user_version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap_or(0);
    let user_version = current_user_version.max(OLD_BASELINE_VERSION - 1 + max_applied);
    conn.execute_batch(&format!("PRAGMA user_version = {}", user_version))?;
    Ok(())
}

pub(super) fn run_post_migration_hook(conn: &Connection, version: i64, name: &str) -> Result<()> {
    if version == 15 {
        let rebuilt = crate::memory::search_context::rebuild_all(conn).with_context(|| {
            format!("migration v{version:03}_{name} failed to rebuild memory search contexts")
        })?;
        crate::log::info(
            "migrate",
            &format!("rebuilt search_context for {rebuilt} memories"),
        );
    }
    if version == 31 || version == 33 || version == 34 {
        install_v031_state_delete_trigger(conn).with_context(|| {
            format!("migration v{version:03}_{name} failed to install graph edge cleanup triggers")
        })?;
    }
    if version == 41 {
        super::content_identity::backfill_content_identity_hashes(conn).with_context(|| {
            format!("migration v{version:03}_{name} failed to backfill content identity hashes")
        })?;
    }
    if version == 53 {
        let updated = crate::workstream::backfill_workstream_alias_normalized_titles(conn)
            .with_context(|| {
                format!(
                    "migration v{version:03}_{name} failed to normalize workstream alias titles"
                )
            })?;
        crate::log::info(
            "migrate",
            &format!("normalized {updated} workstream alias title(s)"),
        );
    }
    if version == 69 {
        log_v069_reconciliation_counts(conn).with_context(|| {
            format!("migration v{version:03}_{name} failed to log reconciliation counts")
        })?;
    }
    Ok(())
}

fn log_v069_reconciliation_counts(conn: &Connection) -> Result<()> {
    let mut counts = Vec::with_capacity(3);
    for identity_kind in ["ordinary", "dream", "compile_rules"] {
        let (reconciled, manual_review): (i64, i64) = conn.query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(CASE WHEN manual_review = 1 THEN 1 ELSE 0 END), 0)
             FROM temp._v069_reconciled
             WHERE identity_kind = ?1",
            params![identity_kind],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        counts.push((reconciled, manual_review));
    }

    let message = format!(
        "v069 job queue reconciliation ordinary={} manual_review={} dream={} manual_review={} compile_rules={} manual_review={}",
        counts[0].0,
        counts[0].1,
        counts[1].0,
        counts[1].1,
        counts[2].0,
        counts[2].1,
    );
    let mut log_file =
        crate::log::open_log_append().ok_or_else(|| anyhow!("prepare migration log output"))?;
    writeln!(
        log_file,
        "[{}] [INFO] [migrate] {message}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    )
    .context("write migration reconciliation log")?;
    log_file
        .flush()
        .context("flush migration reconciliation log")?;

    conn.execute_batch("DROP TABLE temp._v069_reconciled")
        .context("drop v069 reconciliation evidence after logging")?;
    Ok(())
}
