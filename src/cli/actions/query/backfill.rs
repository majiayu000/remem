use anyhow::Result;
use rusqlite::Connection;

use crate::db;

pub(in crate::cli) fn run_backfill_entities() -> Result<()> {
    let conn = db::open_db()?;
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE status = 'active'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    println!("Backfilling entities from {} active memories...", count);

    let mut stmt = conn.prepare("SELECT id, title, content FROM memories WHERE status = 'active'")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;

    print_backfill_progress(&conn, rows, count)
}

fn print_backfill_progress<F>(
    conn: &Connection,
    rows: rusqlite::MappedRows<'_, F>,
    count: i64,
) -> Result<()>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<(i64, String, String)>,
{
    let mut total_entities = 0usize;
    let mut memories_processed = 0usize;

    for row in rows {
        let (id, title, content) = row?;
        let entities = crate::entity::extract_entities(&title, &content);
        if !entities.is_empty() {
            crate::entity::link_entities(conn, id, &entities)?;
            total_entities += entities.len();
        }
        memories_processed += 1;
        if memories_processed.is_multiple_of(100) {
            println!("  processed {}/{}", memories_processed, count);
        }
    }

    let unique: i64 = conn
        .query_row("SELECT COUNT(*) FROM entities", [], |row| row.get(0))
        .unwrap_or(0);
    println!(
        "Done. {} entities extracted, {} unique entities, {} memories processed.",
        total_entities, unique, memories_processed
    );
    Ok(())
}
