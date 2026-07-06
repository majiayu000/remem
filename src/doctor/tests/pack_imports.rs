use rusqlite::params;

use crate::db::test_support::ScopedTestDataDir;
use crate::{db, memory};

use super::super::pack_imports::check_pack_imports;

#[test]
fn check_pack_imports_reports_origin_memory_and_review_counts() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("doctor-pack-imports");
    let conn = db::open_db()?;
    let memory_id = memory::insert_memory(
        &conn,
        None,
        "/repo",
        Some("pack-state"),
        "Imported decision",
        "Keep imported pack rows attributable.",
        "decision",
        None,
    )?;
    conn.execute(
        "UPDATE memories
         SET source_trust_class = 'pack',
             topic_domain = 'pack:abc123def456'
         WHERE id = ?1",
        params![memory_id],
    )?;
    conn.execute(
        "INSERT INTO memory_candidates
         (scope, memory_type, topic_key, text, evidence_event_ids, confidence,
          risk_class, review_status, created_at_epoch, updated_at_epoch,
          source_kind, source_trust_class, topic_domain)
         VALUES
         ('project', 'decision', 'pending-pack', 'pending', '[]', 0.7,
          'medium', 'pending_review', 1, 1, 'pack', 'pack', 'pack:abc123def456'),
         ('project', 'decision', 'quarantine-pack', 'quarantine', '[]', 0.7,
          'high', 'quarantined', 1, 1, 'pack', 'pack', 'pack:abc123def456')",
        [],
    )?;

    let check = check_pack_imports(Some(&conn));

    assert_eq!(check.icon(), "ok");
    assert!(check.detail.contains("pack:abc123def456 memories=1"));
    assert!(check.detail.contains("pending_review=1"));
    assert!(check.detail.contains("quarantined=1"));
    Ok(())
}
