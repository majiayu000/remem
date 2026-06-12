use anyhow::Result;
use rusqlite::{params, Connection};

use remem::{migrate, retrieval::vector};

#[test]
fn vector_search_10k_candidate_gate() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    migrate::run_migrations(&conn)?;
    let embedding = vec![0.0_f32; vector::EMBEDDING_DIMENSIONS];

    conn.execute("BEGIN IMMEDIATE", [])?;
    for id in 1..=10_000_i64 {
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status)
             VALUES (?1, '/repo', 'Vector bench', 'Bounded vector scan candidate', 'decision', ?1, ?1, 'active')",
            params![id],
        )?;
        vector::upsert_embedding(&conn, id, &embedding)?;
    }
    conn.execute("COMMIT", [])?;

    let query = vector::embed_query_text("bounded vector scan");
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
        "[VectorBound] corpus=10000 scanned={} returned={} elapsed_ms={}",
        outcome.candidates_scanned,
        outcome.hits.len(),
        elapsed.as_millis()
    );

    assert!(outcome.candidates_scanned <= vector::VECTOR_SEARCH_CANDIDATE_LIMIT);
    assert!(outcome.candidates_scanned < 10_000);
    assert!(outcome.hits.len() <= 10);
    Ok(())
}
