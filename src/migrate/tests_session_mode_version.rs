use super::MIGRATIONS;
use rusqlite::Connection;

// Catches backfilling legacy rows as shared, schema constraints or registration omissions.
#[test]
fn v093_preserves_legacy_rows_and_defaults_for_old_writers() -> anyhow::Result<()> {
    let conn = Connection::open_in_memory()?;
    super::state::ensure_migration_table(&conn)?;
    for migration in MIGRATIONS
        .iter()
        .filter(|migration| migration.version <= 92)
    {
        conn.execute_batch(migration.sql)?;
        super::state::mark_applied(&conn, migration.version, migration.name)?;
    }
    let insert = "INSERT INTO raw_session_identities
        (source_root, transcript_path, fallback_session_id, canonical_session_id,
         project, legacy_project, status, observed_mtime_ns, observed_size_bytes,
         first_seen_at_epoch, last_seen_at_epoch, session_mode)
        VALUES ('local', ?1, 's', 's', 'p', 'p', 'active', 10, 20, 30, 40, 'interactive')";
    conn.execute(insert, ["old"])?;
    let migration = MIGRATIONS.iter().find(|m| m.version == 93).unwrap();
    assert_eq!(migration.name, "raw_session_mode_version");
    super::run_migrations(&conn)?;
    super::run_migrations(&conn)?;
    assert_eq!(super::state::applied_versions(&conn)?.last(), Some(&93));
    conn.execute(insert, ["old-writer"])?;
    let rows = conn
        .prepare(
            "SELECT id, session_mode, session_mode_version,
        observed_mtime_ns, observed_size_bytes, first_seen_at_epoch, last_seen_at_epoch
        FROM raw_session_identities ORDER BY id",
        )?
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(
        rows,
        vec![
            (1, "interactive".into(), 0, 10, 20, 30, 40),
            (2, "interactive".into(), 0, 10, 20, 30, 40)
        ]
    );
    for invalid in [-1, 2] {
        assert!(conn
            .execute(
                "UPDATE raw_session_identities SET session_mode_version=?1",
                [invalid]
            )
            .is_err());
    }
    assert!(conn
        .execute(
            "UPDATE raw_session_identities SET session_mode_version=NULL",
            []
        )
        .is_err());
    conn.execute(
        "UPDATE raw_session_identities SET session_mode_version=1 WHERE id=1",
        [],
    )?;
    assert_eq!(
        conn.query_row(
            "SELECT session_mode_version FROM raw_session_identities WHERE id=1",
            [],
            |r| r.get::<_, i64>(0)
        )?,
        1
    );
    // Catches registering the SQL but forgetting the schema-drift invariant.
    conn.execute_batch("ALTER TABLE raw_session_identities DROP COLUMN session_mode_version")?;
    let error = super::run_migrations(&conn).expect_err("missing version column must fail closed");
    assert!(
        format!("{error:#}").contains("session_mode_version"),
        "{error:#}"
    );
    Ok(())
}
