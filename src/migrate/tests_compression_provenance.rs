use anyhow::Result;
use rusqlite::{params, Connection};

use super::run_migrations;

fn link_count(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM compressed_observation_sources",
        [],
        |row| row.get(0),
    )?)
}

#[test]
fn compressed_source_links_survive_source_delete_but_cascade_from_compressed_row() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    run_migrations(&conn)?;

    let source_id = crate::db::insert_observation(
        &conn,
        "source-session",
        "proj",
        "discovery",
        Some("Source"),
        None,
        Some("Original source observation"),
        None,
        None,
        None,
        None,
        None,
        0,
    )?;
    let compressed_id = crate::db::insert_observation(
        &conn,
        "compressed-test",
        "proj",
        "decision",
        Some("Compressed"),
        None,
        Some("Compressed observation"),
        None,
        None,
        None,
        None,
        None,
        0,
    )?;
    let sources = crate::db::get_observations_by_ids(&conn, &[source_id], Some("proj"))?;
    crate::db::insert_compressed_observation_sources(
        &conn,
        &[compressed_id],
        &sources,
        "compressed-test",
    )?;
    assert_eq!(link_count(&conn)?, 1);

    conn.execute("DELETE FROM observations WHERE id = ?1", params![source_id])?;
    assert_eq!(
        link_count(&conn)?,
        1,
        "retention deleting source rows must preserve provenance"
    );

    conn.execute(
        "DELETE FROM observations WHERE id = ?1",
        params![compressed_id],
    )?;
    assert_eq!(
        link_count(&conn)?,
        0,
        "deleting a compressed row should clean its provenance"
    );
    Ok(())
}
