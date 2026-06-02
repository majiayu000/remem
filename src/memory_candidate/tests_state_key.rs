use anyhow::Result;
use rusqlite::params;

use super::tests::{setup_conn, setup_task};
use super::tests_autopromote::insert_source_observation_typed;
use super::{process_with_generator, MemoryCandidateResult};

#[tokio::test]
async fn auto_promote_supersedes_state_key_and_legacy_same_topic_rows() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-state-and-topic")?;
    let text =
        "Use the async worker loop to process extraction tasks after observation extraction.";
    insert_source_observation_typed(&conn, &task, "decision", text)?;
    let keyed_id = crate::memory::insert_memory_full(
        &conn,
        None,
        "/tmp/remem",
        Some("decision-worker-loop"),
        "Old keyed worker loop",
        "Use the queued worker loop to process extraction tasks after observation extraction.",
        "decision",
        None,
        None,
        "project",
        None,
    )?;
    let legacy_id = crate::memory::insert_memory_full(
        &conn,
        None,
        "/tmp/remem",
        Some("decision-worker-loop-legacy"),
        "Legacy worker loop",
        "Use the legacy worker loop to process extraction tasks after observation extraction.",
        "decision",
        None,
        None,
        "project",
        None,
    )?;
    conn.execute(
        "UPDATE memories SET topic_key = ?1, state_key_id = NULL WHERE id = ?2",
        params!["decision-worker-loop", legacy_id],
    )?;
    conn.execute(
        "UPDATE memory_state_keys SET current_memory_id = NULL WHERE current_memory_id = ?1",
        params![legacy_id],
    )?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok(format!(
            "<memory_candidate>\
                <scope>project</scope>\
                <type>decision</type>\
                <topic_key>decision-worker-loop</topic_key>\
                <risk_class>low</risk_class>\
                <confidence>0.92</confidence>\
                <text>{text}</text>\
             </memory_candidate>"
        ))
    })
    .await?;

    assert_eq!(
        result,
        MemoryCandidateResult::Written {
            candidates: 1,
            promoted: 1,
            pending_review: 0
        }
    );
    for old_id in [keyed_id, legacy_id] {
        let status: String = conn.query_row(
            "SELECT status FROM memories WHERE id = ?1",
            params![old_id],
            |row| row.get(0),
        )?;
        assert_eq!(status, "stale");
    }
    let active_rows = conn
        .prepare(
            "SELECT id, content FROM memories
             WHERE topic_key = 'decision-worker-loop' AND status = 'active'
             ORDER BY id",
        )?
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    assert_eq!(active_rows.len(), 1);
    assert_eq!(active_rows[0].1, text);
    Ok(())
}
