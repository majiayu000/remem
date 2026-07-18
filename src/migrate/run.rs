use std::io::Write;

use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection};

use super::schema_drift::{install_v031_state_delete_trigger, repair_known_schema_drift};
use super::state::{applied_versions, ensure_migration_table, has_migration_table, mark_applied};
use super::transition::transition_from_old_system;
use super::types::{MIGRATIONS, OLD_BASELINE_VERSION};

const WEB_CONSOLE_GOVERNANCE_VERSION: i64 = 70;

pub fn run_migrations(conn: &Connection) -> Result<()> {
    set_foreign_keys(conn, true).context("enable foreign keys before migration")?;
    ensure_migration_table(conn)?;

    let applied = applied_versions(conn)?;
    let v070_pending = !applied.contains(&WEB_CONSOLE_GOVERNANCE_VERSION);
    if v070_pending {
        set_foreign_keys(conn, false).context("disable foreign keys for v070 table rebuild")?;
    }

    let outcome = run_migration_transaction(conn, v070_pending);
    match outcome {
        MigrationOutcome::Completed(result) => restore_foreign_keys(conn, result),
        MigrationOutcome::DiscardConnection(error) => Err(error),
    }
}

enum MigrationOutcome {
    Completed(Result<()>),
    DiscardConnection(anyhow::Error),
}

fn run_migration_transaction(conn: &Connection, verify_rebuild: bool) -> MigrationOutcome {
    if let Err(error) = conn
        .execute_batch("BEGIN IMMEDIATE")
        .context("begin migration transaction")
    {
        return MigrationOutcome::Completed(Err(error));
    }

    let migration_result = run_migrations_locked(conn).and_then(|()| {
        if verify_rebuild {
            verify_migration_integrity(conn)?;
        }
        Ok(())
    });

    finish_migration_transaction(conn, migration_result)
}

fn finish_migration_transaction(
    conn: &Connection,
    migration_result: Result<()>,
) -> MigrationOutcome {
    match migration_result {
        Ok(()) => match conn.execute_batch("COMMIT") {
            Ok(()) => MigrationOutcome::Completed(Ok(())),
            Err(commit_error) => rollback_after_error(
                conn,
                anyhow!(commit_error).context("commit migration transaction"),
            ),
        },
        Err(error) => rollback_after_error(conn, error),
    }
}

fn rollback_after_error(conn: &Connection, error: anyhow::Error) -> MigrationOutcome {
    match conn.execute_batch("ROLLBACK") {
        Ok(()) => MigrationOutcome::Completed(Err(error)),
        Err(rollback_error) => {
            crate::log::error(
                "migrate",
                &format!(
                    "fatal migration rollback failure; discard connection: {rollback_error}; \
                     original migration error: {error:#}"
                ),
            );
            MigrationOutcome::DiscardConnection(error.context(format!(
                "migration rollback also failed; connection must be discarded: {rollback_error}"
            )))
        }
    }
}

fn restore_foreign_keys(conn: &Connection, result: Result<()>) -> Result<()> {
    let restored = set_foreign_keys(conn, true).context("restore foreign keys after migration");
    match (result, restored) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(restore_error)) => Err(restore_error),
        (Err(error), Err(restore_error)) => Err(error.context(format!(
            "foreign key restoration also failed: {restore_error:#}"
        ))),
    }
}

fn set_foreign_keys(conn: &Connection, enabled: bool) -> Result<()> {
    let statement = if enabled {
        "PRAGMA foreign_keys = ON"
    } else {
        "PRAGMA foreign_keys = OFF"
    };
    conn.execute_batch(statement)?;
    let actual: i64 = conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
    if (actual != 0) != enabled {
        return Err(anyhow!(
            "foreign key mode verification failed: expected {}, got {}",
            if enabled { "ON" } else { "OFF" },
            if actual != 0 { "ON" } else { "OFF" }
        ));
    }
    Ok(())
}

fn verify_migration_integrity(conn: &Connection) -> Result<()> {
    let mut foreign_key_check = conn.prepare("PRAGMA foreign_key_check")?;
    let mut foreign_key_rows = foreign_key_check.query([])?;
    if let Some(row) = foreign_key_rows.next()? {
        let table: String = row.get(0)?;
        let row_id: Option<i64> = row.get(1)?;
        let parent: String = row.get(2)?;
        return Err(anyhow!(
            "foreign key check failed for table {table}, row {row_id:?}, parent {parent}"
        ));
    }

    let mut integrity_check = conn.prepare("PRAGMA integrity_check")?;
    let messages = integrity_check
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if messages.len() != 1 || messages[0] != "ok" {
        return Err(anyhow!("integrity check failed: {}", messages.join("; ")));
    }
    Ok(())
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
        run_pre_migration_hook(conn, migration.version, migration.name)?;
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

pub(super) fn run_pre_migration_hook(conn: &Connection, version: i64, name: &str) -> Result<()> {
    if version == 69 {
        prepare_v069_dream_profile_keys(conn).with_context(|| {
            format!("migration v{version:03}_{name} failed to normalize Dream profiles")
        })?;
    }
    Ok(())
}

fn prepare_v069_dream_profile_keys(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "DROP TABLE IF EXISTS temp._v069_dream_profile_keys;
         CREATE TEMP TABLE _v069_dream_profile_keys(
             id INTEGER PRIMARY KEY,
             profile_key TEXT NOT NULL
         );",
    )
    .context("create v069 Dream profile normalization table")?;
    let rows = {
        let mut statement = conn.prepare(
            "SELECT pending_dream.id, pending_dream.payload_json
             FROM jobs AS pending_dream
             WHERE pending_dream.job_type = 'dream'
               AND pending_dream.state = 'pending'
               AND NOT EXISTS (
                   SELECT 1
                   FROM jobs AS processing_dream
                   WHERE processing_dream.job_type = 'dream'
                     AND processing_dream.project = pending_dream.project
                     AND processing_dream.state = 'processing'
               )
             ORDER BY pending_dream.id ASC",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for (id, payload_json) in rows {
        let profile_key = crate::db::job::dream_profile_key(&payload_json).unwrap_or_default();
        conn.execute(
            "INSERT INTO temp._v069_dream_profile_keys(id, profile_key) VALUES (?1, ?2)",
            params![id, profile_key],
        )?;
    }
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

#[cfg(test)]
mod transaction_failure_tests {
    use anyhow::anyhow;
    use rusqlite::Connection;

    use super::{
        finish_migration_transaction, restore_foreign_keys, rollback_after_error, MigrationOutcome,
    };

    #[test]
    fn commit_failure_rolls_back_and_restores_foreign_keys() -> anyhow::Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE parent(id INTEGER PRIMARY KEY);
             CREATE TABLE child(
                 id INTEGER PRIMARY KEY,
                 parent_id INTEGER REFERENCES parent(id) DEFERRABLE INITIALLY DEFERRED
             );
             BEGIN IMMEDIATE;
             INSERT INTO child(id, parent_id) VALUES (1, 99);",
        )?;

        let outcome = finish_migration_transaction(&conn, Ok(()));
        let result = match outcome {
            MigrationOutcome::Completed(result) => restore_foreign_keys(&conn, result),
            MigrationOutcome::DiscardConnection(error) => Err(error),
        };
        let error = result.expect_err("deferred foreign key must fail COMMIT");
        assert!(format!("{error:#}").contains("commit migration transaction"));
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM child", [], |row| row.get::<_, i64>(0))?,
            0
        );
        assert_eq!(
            conn.query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))?,
            1
        );
        Ok(())
    }

    #[test]
    fn rollback_failure_requires_connection_discard() -> anyhow::Result<()> {
        let conn = Connection::open_in_memory()?;
        let outcome = rollback_after_error(&conn, anyhow!("injected migration failure"));
        let MigrationOutcome::DiscardConnection(error) = outcome else {
            panic!("ROLLBACK without a transaction must require connection discard");
        };
        let message = format!("{error:#}");
        assert!(message.contains("injected migration failure"));
        assert!(message.contains("connection must be discarded"));
        Ok(())
    }
}
