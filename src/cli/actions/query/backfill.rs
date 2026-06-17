use anyhow::Result;
use rusqlite::Connection;
use std::time::Instant;

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

    let mut stmt =
        conn.prepare("SELECT id, title, content FROM memories WHERE status = 'active'")?;
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
        let entities = crate::retrieval::entity::extract_entities(&title, &content);
        if !entities.is_empty() {
            crate::retrieval::entity::link_entities(conn, id, &entities)?;
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

pub(in crate::cli) fn run_backfill_embeddings(limit: i64, batch_size: i64) -> Result<()> {
    let conn = db::open_db()?;
    let limit = limit.max(1);
    let batch_size = batch_size.max(1);
    let pending_before = count_missing_embeddings(&conn)?;
    println!(
        "Backfilling/reindexing up to {limit} missing or stale memory embeddings, batch_size={batch_size}, pending_before={pending_before}..."
    );

    let started = Instant::now();
    let mut backfilled = 0usize;
    let mut remaining_limit = limit;
    let mut printed_profile = false;
    while remaining_limit > 0 {
        let batch_limit = remaining_limit.min(batch_size);
        let report =
            crate::retrieval::vector::reindex_memory_embeddings_with_report(&conn, batch_limit)?;
        if !printed_profile && !report.model.is_empty() {
            println!(
                "Embedding profile: model={} dimensions={}",
                report.model, report.dimensions
            );
            printed_profile = true;
        }
        if report.processed == 0 {
            break;
        }

        backfilled += report.processed;
        remaining_limit -= report.processed as i64;
        let remaining = count_missing_embeddings(&conn)?;
        let elapsed_ms = report
            .timings
            .iter()
            .find(|timing| timing.phase == "total")
            .map(|timing| timing.elapsed_ms)
            .unwrap_or(0);
        let rows_per_sec = if elapsed_ms == 0 {
            report.processed as f64
        } else {
            report.processed as f64 * 1000.0 / elapsed_ms as f64
        };
        println!(
            "  batch processed={} selected={} remaining={} rows_per_sec={rows_per_sec:.1} {}",
            report.processed,
            report.selected,
            remaining,
            crate::perf::format_phase_timings(&report.timings)
        );
        if report.processed < batch_limit as usize {
            break;
        }
    }

    let remaining = count_missing_embeddings(&conn)?;
    println!(
        "Done. {backfilled} embeddings backfilled/reindexed, {remaining} remaining, elapsed_ms={}.",
        started.elapsed().as_millis()
    );
    Ok(())
}

fn count_missing_embeddings(conn: &Connection) -> Result<i64> {
    crate::retrieval::vector::pending_memory_embedding_reindex_count(conn)
}
