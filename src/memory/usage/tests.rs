use rusqlite::params;

use super::*;

fn setup_memory_usage_conn() -> Result<rusqlite::Connection> {
    let conn = rusqlite::Connection::open_in_memory()?;
    crate::memory::tests_helper::setup_memory_schema(&conn);
    Ok(conn)
}

fn insert_usage_memory(conn: &rusqlite::Connection, title: &str) -> Result<i64> {
    crate::memory::insert_memory(
        conn,
        Some("seed-session"),
        "/repo",
        None,
        title,
        "memory body",
        "decision",
        None,
    )
}

fn insert_injected_item(conn: &rusqlite::Connection, memory_id: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO context_injection_items
         (injection_run_id, host, project, session_id, injection_key, output_mode,
          decision, item_kind, item_id, memory_id, channel, render_order, status,
          title, provenance, staleness, injected_at_epoch)
         VALUES ('run-1', 'codex-cli', '/repo', 'sess-1', 'key-1', 'full',
                 'emitted', 'memory', ?1, ?1, 'core', 1, 'injected',
                 'title', 'src=memory', 'current', 100)",
        params![memory_id],
    )?;
    Ok(())
}

#[test]
fn parses_only_dedicated_memory_citation_line() {
    let text = "src=memory:#44 is metadata\nMemory citations: memory:#7, memory:#7 memory:#9\n";
    let parsed = parse_memory_citations(text);

    assert!(parsed.line_present);
    assert_eq!(parsed.ids, vec![7, 9]);
}

#[test]
fn recognizes_none_citation_line_without_memory_ids() {
    let parsed = parse_memory_citations("No memory was needed.\nMemory citations: none");

    assert!(parsed.line_present);
    assert!(parsed.ids.is_empty());
}

#[test]
fn records_only_cited_injected_memories_once() -> Result<()> {
    let conn = setup_memory_usage_conn()?;
    let cited = insert_usage_memory(&conn, "cited")?;
    let not_injected = insert_usage_memory(&conn, "not injected")?;
    insert_injected_item(&conn, cited)?;

    let message = format!(
        "I used the durable context.\nMemory citations: memory:#{cited} memory:#{not_injected}"
    );
    let report =
        record_stop_memory_citations(&conn, "codex-cli", "/repo", "sess-1", "hash-a", &message)?;
    let duplicate =
        record_stop_memory_citations(&conn, "codex-cli", "/repo", "sess-1", "hash-a", &message)?;

    assert_eq!(
        report,
        MemoryUsageReport {
            parsed_count: 2,
            matched_count: 1,
            inserted_count: 1,
            duplicate_event: false,
        }
    );
    assert!(duplicate.duplicate_event);
    let cited_count: i64 = conn.query_row(
        "SELECT access_count FROM memories WHERE id = ?1",
        [cited],
        |row| row.get(0),
    )?;
    let not_injected_count: i64 = conn.query_row(
        "SELECT access_count FROM memories WHERE id = ?1",
        [not_injected],
        |row| row.get(0),
    )?;
    let usage_events: i64 =
        conn.query_row("SELECT COUNT(*) FROM memory_usage_events", [], |row| {
            row.get(0)
        })?;
    assert_eq!(cited_count, 1);
    assert_eq!(not_injected_count, 0);
    assert_eq!(usage_events, 1);
    Ok(())
}

#[test]
fn usage_feedback_stats_expose_no_citation_and_unmatched_categories() -> Result<()> {
    let conn = setup_memory_usage_conn()?;
    let cited = insert_usage_memory(&conn, "cited")?;
    let not_injected = insert_usage_memory(&conn, "not injected")?;
    insert_injected_item(&conn, cited)?;

    record_stop_memory_citations(
        &conn,
        "codex-cli",
        "/repo",
        "sess-1",
        "hash-matched",
        &format!("Used memory.\nMemory citations: memory:#{cited}"),
    )?;
    record_stop_memory_citations(
        &conn,
        "codex-cli",
        "/repo",
        "sess-1",
        "hash-none",
        "No memory was needed.\nMemory citations: none",
    )?;
    record_stop_memory_citations(
        &conn,
        "codex-cli",
        "/repo",
        "sess-1",
        "hash-unmatched",
        &format!("Referenced a missing injection.\nMemory citations: memory:#{not_injected}"),
    )?;

    let stats = query_memory_usage_feedback_stats(&conn)?;

    assert_eq!(stats.total_events, 3);
    assert_eq!(stats.parsed_events, 3);
    assert_eq!(stats.matched_events, 1);
    assert_eq!(stats.inserted_events, 1);
    assert_eq!(stats.no_citation_events, 1);
    assert_eq!(stats.unmatched_events, 1);
    assert_eq!(stats.usage_events, 1);
    Ok(())
}
