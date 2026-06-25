use rusqlite::params;

use crate::db::test_support::ScopedTestDataDir;
use crate::{db, memory};

use super::super::database::check_memory_usage_feedback;

#[test]
fn check_memory_usage_feedback_reports_parse_rate() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("doctor-memory-usage-feedback");
    let conn = db::open_db()?;
    let memory_id = memory::insert_memory(
        &conn,
        Some("session-1"),
        "proj-a",
        None,
        "source memory",
        "A cited injected memory.",
        "decision",
        None,
    )?;
    conn.execute(
        "INSERT INTO context_injection_items
         (injection_run_id, host, project, session_id, injection_key, output_mode,
          decision, item_kind, item_id, memory_id, channel, render_order, status,
          title, provenance, staleness, injected_at_epoch)
         VALUES ('run-1', 'codex-cli', 'proj-a', 'session-1', 'key-1', 'full',
                 'emitted', 'memory', ?1, ?1, 'core', 1, 'injected',
                 'source memory', 'src=memory', 'current', 100)",
        params![memory_id],
    )?;
    crate::memory::usage::record_stop_memory_citations(
        &conn,
        "codex-cli",
        "proj-a",
        "session-1",
        "hash-1",
        &format!("Used the injected memory.\nMemory citations: memory:#{memory_id}"),
    )?;

    let check = check_memory_usage_feedback(Some(&conn));
    assert_eq!(check.icon(), "ok");
    assert!(
        check.detail.contains("1 citation event"),
        "{}",
        check.detail
    );
    assert!(check.detail.contains("parsed=1"), "{}", check.detail);
    assert!(check.detail.contains("matched=1"), "{}", check.detail);
    assert!(check.detail.contains("no_citation=0"), "{}", check.detail);
    assert!(check.detail.contains("unmatched=0"), "{}", check.detail);
    assert!(check.detail.contains("usage_events=1"), "{}", check.detail);
    Ok(())
}
