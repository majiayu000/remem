use super::{
    apply_compression_response, CompressionOutcome, INVALID_REPLACEMENTS_REASON,
    NO_REPLACEMENTS_REASON,
};
use crate::db;
use anyhow::Result;
use rusqlite::{params, Connection};

fn setup_observation_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE observations (
            id INTEGER PRIMARY KEY,
            memory_session_id TEXT NOT NULL,
            project TEXT,
            type TEXT NOT NULL,
            title TEXT,
            subtitle TEXT,
            narrative TEXT,
            facts TEXT,
            concepts TEXT,
            files_read TEXT,
            files_modified TEXT,
            prompt_number INTEGER,
            created_at TEXT,
            created_at_epoch INTEGER,
            discovery_tokens INTEGER DEFAULT 0,
            status TEXT DEFAULT 'active',
            last_accessed_epoch INTEGER,
            branch TEXT,
            commit_sha TEXT
        );
        CREATE TABLE sdk_sessions (
            id INTEGER PRIMARY KEY,
            content_session_id TEXT UNIQUE NOT NULL,
            memory_session_id TEXT NOT NULL
        );
        CREATE TABLE compressed_observation_sources (
            id INTEGER PRIMARY KEY,
            compressed_observation_id INTEGER NOT NULL,
            source_observation_id INTEGER NOT NULL,
            source_hash TEXT NOT NULL,
            source_snapshot_json TEXT NOT NULL,
            source_created_at_epoch INTEGER NOT NULL,
            compression_session_id TEXT NOT NULL,
            created_at_epoch INTEGER NOT NULL,
            UNIQUE(compressed_observation_id, source_observation_id),
            FOREIGN KEY(compressed_observation_id) REFERENCES observations(id) ON DELETE CASCADE
        );",
    )?;
    Ok(())
}

fn insert_source_observation(conn: &Connection, status: &str) -> Result<i64> {
    let id = db::insert_observation(
        conn,
        "source-session",
        "proj",
        "discovery",
        Some("Source"),
        None,
        Some("Source observation"),
        None,
        None,
        None,
        None,
        None,
        0,
    )?;
    conn.execute(
        "UPDATE observations SET status = ?1 WHERE id = ?2",
        params![status, id],
    )?;
    Ok(id)
}

fn statuses_for(conn: &Connection, ids: &[i64]) -> Result<Vec<String>> {
    let placeholders = ids
        .iter()
        .enumerate()
        .map(|(idx, _)| format!("?{}", idx + 1))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("SELECT status FROM observations WHERE id IN ({placeholders}) ORDER BY id");
    let params = ids
        .iter()
        .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
        .collect::<Vec<_>>();
    let refs = crate::db::to_sql_refs(&params);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| row.get::<_, String>(0))?;
    crate::db::query::collect_rows(rows)
}

fn compressed_titles(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(title, '')
         FROM observations
         WHERE memory_session_id LIKE 'compressed-%'
         ORDER BY id",
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    crate::db::query::collect_rows(rows)
}

fn source_observations(conn: &Connection, ids: &[i64]) -> Result<Vec<db::Observation>> {
    db::get_observations_by_ids(conn, ids, Some("proj"))
}

fn compressed_source_links(conn: &Connection) -> Result<Vec<(i64, i64, String)>> {
    let mut stmt = conn.prepare(
        "SELECT compressed_observation_id, source_observation_id, source_hash
         FROM compressed_observation_sources
         ORDER BY compressed_observation_id, source_observation_id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    crate::db::query::collect_rows(rows)
}

fn valid_response(title: &str) -> String {
    format!(
        "<observation>
            <type>decision</type>
            <title>{title}</title>
            <narrative>Compressed narrative</narrative>
         </observation>"
    )
}

#[test]
fn empty_compression_response_leaves_sources_active() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_observation_schema(&conn)?;
    let ids = vec![
        insert_source_observation(&conn, "active")?,
        insert_source_observation(&conn, "stale")?,
    ];

    let sources = source_observations(&conn, &ids)?;
    let outcome = apply_compression_response(&conn, "proj", &sources, "")?;

    assert_eq!(
        outcome,
        CompressionOutcome::Skipped {
            reason: NO_REPLACEMENTS_REASON,
            source_count: 2,
        }
    );
    assert_eq!(statuses_for(&conn, &ids)?, vec!["active", "stale"]);
    assert!(compressed_titles(&conn)?.is_empty());
    Ok(())
}

#[test]
fn malformed_compression_response_leaves_sources_active() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_observation_schema(&conn)?;
    let ids = vec![insert_source_observation(&conn, "active")?];

    let sources = source_observations(&conn, &ids)?;
    let outcome = apply_compression_response(
        &conn,
        "proj",
        &sources,
        "<observation><type>decision</type><title>broken",
    )?;

    assert_eq!(
        outcome,
        CompressionOutcome::Skipped {
            reason: NO_REPLACEMENTS_REASON,
            source_count: 1,
        }
    );
    assert_eq!(statuses_for(&conn, &ids)?, vec!["active"]);
    assert!(compressed_titles(&conn)?.is_empty());
    Ok(())
}

#[test]
fn contentless_replacements_do_not_retire_sources() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_observation_schema(&conn)?;
    let ids = vec![insert_source_observation(&conn, "active")?];

    let sources = source_observations(&conn, &ids)?;
    let missing_type =
        apply_compression_response(&conn, "proj", &sources, "<observation></observation>")?;
    assert_eq!(
        missing_type,
        CompressionOutcome::Skipped {
            reason: NO_REPLACEMENTS_REASON,
            source_count: 1,
        }
    );

    let contentless = apply_compression_response(
        &conn,
        "proj",
        &sources,
        "<observation><type>decision</type></observation>",
    )?;
    assert_eq!(
        contentless,
        CompressionOutcome::Skipped {
            reason: INVALID_REPLACEMENTS_REASON,
            source_count: 1,
        }
    );
    assert_eq!(statuses_for(&conn, &ids)?, vec!["active"]);
    assert!(compressed_titles(&conn)?.is_empty());
    Ok(())
}

#[test]
fn valid_compression_inserts_replacement_then_marks_sources() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_observation_schema(&conn)?;
    let ids = vec![
        insert_source_observation(&conn, "active")?,
        insert_source_observation(&conn, "stale")?,
    ];

    let sources = source_observations(&conn, &ids)?;
    let outcome =
        apply_compression_response(&conn, "proj", &sources, &valid_response("Compressed"))?;

    assert_eq!(
        outcome,
        CompressionOutcome::Compressed {
            source_count: 2,
            replacement_count: 1,
            marked_count: 2,
        }
    );
    assert_eq!(statuses_for(&conn, &ids)?, vec!["compressed", "compressed"]);
    assert_eq!(compressed_titles(&conn)?, vec!["Compressed"]);
    let links = compressed_source_links(&conn)?;
    assert_eq!(links.len(), 2);
    for (_, source_id, source_hash) in links {
        let Some(source) = sources.iter().find(|source| source.id == source_id) else {
            panic!("linked source should be in original batch");
        };
        assert_eq!(source_hash, db::observation_source_hash(source));
    }
    Ok(())
}

#[test]
fn multi_replacement_compression_links_every_replacement_to_every_source() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_observation_schema(&conn)?;
    let ids = vec![
        insert_source_observation(&conn, "active")?,
        insert_source_observation(&conn, "stale")?,
    ];
    let sources = source_observations(&conn, &ids)?;
    let response = format!(
        "{}\n{}",
        valid_response("Compressed one"),
        valid_response("Compressed two")
    );

    let outcome = apply_compression_response(&conn, "proj", &sources, &response)?;

    assert_eq!(
        outcome,
        CompressionOutcome::Compressed {
            source_count: 2,
            replacement_count: 2,
            marked_count: 2,
        }
    );
    let links = compressed_source_links(&conn)?;
    assert_eq!(links.len(), 4);
    for source in &sources {
        let matching = links
            .iter()
            .filter(|(_, source_id, source_hash)| {
                *source_id == source.id && *source_hash == db::observation_source_hash(source)
            })
            .count();
        assert_eq!(matching, 2);
    }
    Ok(())
}

#[test]
fn partial_source_mark_rolls_back_sources_and_replacements() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_observation_schema(&conn)?;
    let ids = vec![
        insert_source_observation(&conn, "active")?,
        insert_source_observation(&conn, "compressed")?,
    ];

    let sources = source_observations(&conn, &ids)?;
    let error = apply_compression_response(&conn, "proj", &sources, &valid_response("Compressed"))
        .expect_err("partial mark should roll back");

    assert!(error
        .to_string()
        .contains("marked 1 of 2 source observations compressed"));
    assert_eq!(statuses_for(&conn, &ids)?, vec!["active", "compressed"]);
    assert!(compressed_titles(&conn)?.is_empty());
    assert!(compressed_source_links(&conn)?.is_empty());
    Ok(())
}

#[test]
fn source_mark_failure_rolls_back_replacements() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_observation_schema(&conn)?;
    conn.execute_batch(
        "CREATE TRIGGER fail_source_compression
         BEFORE UPDATE OF status ON observations
         WHEN NEW.status = 'compressed' AND OLD.status = 'active'
         BEGIN
             SELECT RAISE(FAIL, 'source compression failed');
         END;",
    )?;
    let ids = vec![insert_source_observation(&conn, "active")?];

    let sources = source_observations(&conn, &ids)?;
    let error = apply_compression_response(&conn, "proj", &sources, &valid_response("Compressed"))
        .expect_err("update trigger should fail");

    assert!(error.to_string().contains("source compression failed"));
    assert_eq!(statuses_for(&conn, &ids)?, vec!["active"]);
    assert!(compressed_titles(&conn)?.is_empty());
    assert!(compressed_source_links(&conn)?.is_empty());
    Ok(())
}

#[test]
fn replacement_insert_failure_rolls_back_sources_and_replacements() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_observation_schema(&conn)?;
    conn.execute_batch(
        "CREATE TRIGGER fail_bad_compression
         BEFORE INSERT ON observations
         WHEN NEW.memory_session_id LIKE 'compressed-%'
              AND NEW.title = 'Bad replacement'
         BEGIN
             SELECT RAISE(FAIL, 'bad compressed insert');
         END;",
    )?;
    let ids = vec![
        insert_source_observation(&conn, "active")?,
        insert_source_observation(&conn, "stale")?,
    ];
    let response = format!(
        "{}\n{}",
        valid_response("Good replacement"),
        valid_response("Bad replacement")
    );

    let sources = source_observations(&conn, &ids)?;
    let error = apply_compression_response(&conn, "proj", &sources, &response)
        .expect_err("insert trigger should fail");

    assert!(error.to_string().contains("bad compressed insert"));
    assert_eq!(statuses_for(&conn, &ids)?, vec!["active", "stale"]);
    assert!(compressed_titles(&conn)?.is_empty());
    assert!(compressed_source_links(&conn)?.is_empty());
    Ok(())
}
