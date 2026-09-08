use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use super::{run_migrations, MIGRATIONS};

fn migrated() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    run_migrations(&conn)?;
    Ok(conn)
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let sql = format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name = ?1");
    let count: i64 = conn.query_row(&sql, [column], |row| row.get(0))?;
    Ok(count == 1)
}

#[test]
fn v092_is_registered_and_named_stably() {
    let migration = MIGRATIONS
        .iter()
        .find(|migration| migration.version == 92)
        .expect("v092 migration must be registered");
    assert_eq!(migration.name, "session_intent_display");
}

#[test]
fn v092_adds_intent_columns_to_summaries_and_workstreams() -> Result<()> {
    let conn = migrated()?;
    for table in ["session_summaries", "workstreams"] {
        for column in [
            "session_intent",
            "session_topic",
            "session_intent_source",
            "session_intent_updated_at_epoch",
        ] {
            assert!(
                column_exists(&conn, table, column)?,
                "{table}.{column} must exist after v092"
            );
        }
    }
    Ok(())
}

#[test]
fn v092_accepts_closed_intent_and_source_or_null() -> Result<()> {
    let conn = migrated()?;
    conn.execute(
        "INSERT INTO session_summaries
         (memory_session_id, project, request, created_at_epoch,
          session_intent, session_topic, session_intent_source,
          session_intent_updated_at_epoch)
         VALUES ('mem-ok', '/repo', 'req', 1, 'fix', 'Batch text display', 'summary', 2)",
        [],
    )
    .context("closed intent/source insert")?;
    conn.execute(
        "INSERT INTO workstreams
         (project, title, status, created_at_epoch, updated_at_epoch,
          session_intent, session_topic, session_intent_source)
         VALUES ('/repo', 'Task', 'active', 1, 1, NULL, NULL, NULL)",
        [],
    )
    .context("nullable intent insert")?;
    Ok(())
}

#[test]
fn v092_rejects_unknown_intent_and_source_on_write() -> Result<()> {
    let conn = migrated()?;
    let bad_intent = conn.execute(
        "INSERT INTO session_summaries
         (memory_session_id, project, request, created_at_epoch, session_intent)
         VALUES ('mem-bad-intent', '/repo', 'req', 1, 'bugfix')",
        [],
    );
    assert!(
        bad_intent.is_err(),
        "unknown session_intent must fail closed"
    );

    let bad_source = conn.execute(
        "INSERT INTO workstreams
         (project, title, status, created_at_epoch, updated_at_epoch, session_intent_source)
         VALUES ('/repo', 'Task', 'active', 1, 1, 'manual')",
        [],
    );
    assert!(
        bad_source.is_err(),
        "unknown session_intent_source must fail closed"
    );

    // Topic length is enforced at the display/write helper, not by SQL CHECK.
    conn.execute(
        "INSERT INTO session_summaries
         (memory_session_id, project, request, created_at_epoch, session_topic)
         VALUES ('mem-long-topic', '/repo', 'req', 1, ?1)",
        params!["n".repeat(81)],
    )
    .context("oversized topic remains SQL-nullable until a writer abstains")?;
    Ok(())
}
