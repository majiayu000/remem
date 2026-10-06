use anyhow::Result;
use rusqlite::{params, Connection};

use remem::{migrate, retrieval::vector};

#[test]
fn vector_search_10k_exact_fallback_preserves_oldest_best_match() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    migrate::run_migrations(&conn)?;
    let mut query = vec![0.0_f32; vector::EMBEDDING_DIMENSIONS];
    query[0] = 1.0;
    let mut other = vec![0.0_f32; vector::EMBEDDING_DIMENSIONS];
    other[1] = 1.0;

    conn.execute("BEGIN IMMEDIATE", [])?;
    for id in 1..=10_000_i64 {
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status)
             VALUES (?1, '/repo', 'Vector bench', 'Exact vector scan candidate', 'decision', ?1, ?1, 'active')",
            params![id],
        )?;
        let embedding = if id == 1 { &query } else { &other };
        vector::upsert_embedding(&conn, id, embedding)?;
    }
    // Make ID 1 oldest in both canonical and embedding recency order.
    conn.execute(
        "UPDATE memory_embeddings SET updated_at_epoch = memory_id",
        [],
    )?;
    conn.execute("COMMIT", [])?;
    assert_eq!(vector::embedding_count(&conn)?, 10_000);

    // Missing readiness state forces exact fallback even if sqlite-vec is
    // registered on this connection. Only derived state in this fixture is removed.
    conn.execute_batch("DROP TABLE IF EXISTS memory_embedding_vec_state_v2")?;

    let start = std::time::Instant::now();
    let outcome = vector::vector_search_filtered(
        &conn,
        &query,
        vector::VectorSearchFilters {
            project: Some("/repo"),
            ..vector::VectorSearchFilters::default()
        },
        10,
    )?;
    let elapsed = start.elapsed();
    eprintln!(
        "[VectorExactFallback] corpus=10000 scanned={} returned={} elapsed_ms={}",
        outcome.candidates_scanned,
        outcome.hits.len(),
        elapsed.as_millis()
    );

    assert!(outcome.disabled_reason.is_none(), "{outcome:?}");
    assert!(outcome
        .timings
        .iter()
        .any(|timing| timing.phase == "vector_exact_scan"));
    assert_eq!(outcome.candidates_scanned, 10_000);
    assert_eq!(outcome.hits.len(), 10);
    assert_eq!(outcome.hits[0].memory_id, 1);
    assert_eq!(outcome.hits[0].distance, 0.0);
    assert!(outcome.hits[1..]
        .iter()
        .all(|hit| hit.memory_id != 1 && hit.distance == 1.0));
    assert_eq!(
        outcome
            .hits
            .iter()
            .map(|hit| hit.memory_id)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        10
    );
    Ok(())
}
