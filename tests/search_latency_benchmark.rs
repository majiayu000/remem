use anyhow::Result;
use rusqlite::{params, Connection};

use remem::{
    migrate,
    perf::format_phase_timings,
    retrieval::{search, vector},
};

#[test]
#[ignore = "large-corpus latency harness; run explicitly with --ignored --nocapture"]
fn query_search_10k_corpus_reports_phase_timings() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    migrate::run_migrations(&conn)?;

    conn.execute("BEGIN IMMEDIATE", [])?;
    for id in 1..=10_000_i64 {
        let title = if id % 500 == 0 {
            format!("FTS5 search latency target {id}")
        } else {
            format!("Noise memory {id}")
        };
        let content = if id % 500 == 0 {
            "FTS5 search should remain measurable on a large in-memory corpus"
        } else {
            "Unrelated memory body for latency benchmark noise"
        };
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status)
             VALUES (?1, '/repo', ?2, ?3, 'decision', ?1, ?1, 'active')",
            params![id, title, content],
        )?;
        vector::upsert_memory_embedding(&conn, id, &title, content, "decision", None)?;
    }
    conn.execute("COMMIT", [])?;

    let start = std::time::Instant::now();
    let (results, explain) = search::search_with_branch_explain(
        &conn,
        Some("FTS5 search"),
        Some("/repo"),
        None,
        10,
        0,
        true,
        None,
    )?;
    let elapsed = start.elapsed();
    let explain = explain.expect("query search should include explain details");
    eprintln!(
        "[SearchLatency] corpus=10000 returned={} elapsed_ms={} timings=[{}]",
        results.len(),
        elapsed.as_millis(),
        format_phase_timings(&explain.timings)
    );

    assert!(!results.is_empty());
    assert!(explain.timings.iter().any(|timing| timing.phase == "fts"));
    assert!(
        explain
            .timings
            .iter()
            .any(|timing| timing.phase == "vector_load_embeddings"),
        "integrated latency harness must exercise vector embedding load: {:#?}",
        explain.timings
    );
    let vector = explain
        .channels
        .iter()
        .find(|channel| channel.name == "vector" && channel.enabled)
        .expect("integrated latency harness must exercise enabled vector channel");
    assert!(
        vector.candidates_scanned.unwrap_or_default() > 0,
        "integrated latency harness must report vector candidate scan count: {vector:#?}"
    );
    assert!(
        elapsed.as_secs_f64() < 5.0,
        "10k in-memory query search exceeded 5s: {:?}",
        elapsed
    );
    Ok(())
}
