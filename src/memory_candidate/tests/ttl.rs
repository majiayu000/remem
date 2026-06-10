use anyhow::Result;
use rusqlite::params;

use super::*;

#[tokio::test]
async fn memory_candidate_assigns_ttl_to_current_operational_state() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-ttl")?;
    let text = "Local dev server is currently running at localhost:3000 for remem.";
    insert_source_observation(&conn, &task, text)?;
    let before = chrono::Utc::now().timestamp();

    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok(format!(
            "<memory_candidate>\
                <scope>project</scope>\
                <type>decision</type>\
                <topic_key>repo:/tmp/remem:dev-server</topic_key>\
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
            pending_review: 0,
            to_event_id: task.high_watermark_event_id.expect("task watermark")
        }
    );
    let (candidate_expires, memory_expires, memory_valid_from): (i64, i64, i64) = conn.query_row(
        "SELECT c.expires_at_epoch, m.expires_at_epoch, m.valid_from_epoch
         FROM memory_candidates c
         JOIN memories m ON m.source_candidate_id = c.id",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let min_expected = before + crate::memory::lifecycle::SHORT_CURRENT_TTL_SECONDS;
    let max_expected =
        chrono::Utc::now().timestamp() + crate::memory::lifecycle::SHORT_CURRENT_TTL_SECONDS;
    assert!((min_expected..=max_expected).contains(&candidate_expires));
    assert!((min_expected..=max_expected).contains(&memory_expires));
    assert!(memory_valid_from >= before);
    Ok(())
}

#[tokio::test]
async fn memory_candidate_refreshes_legacy_current_state_without_ttl() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-refresh-ttl")?;
    let text = "Local dev server is currently running at localhost:3000 for remem.";
    let topic_key = "repo-tmp-remem-dev-server";
    insert_source_observation(&conn, &task, text)?;
    let old_epoch = chrono::Utc::now().timestamp() - 3600;

    conn.execute(
        "INSERT INTO memory_candidates
         (project_id, scope, memory_type, topic_key, text, evidence_event_ids,
          confidence, risk_class, review_status, created_at_epoch, updated_at_epoch,
          source_project, target_project, owner_scope, owner_key, context_class)
         VALUES (?1, 'project', 'decision', ?2, ?3, '[]',
                 0.92, 'low', 'auto_promoted', ?4, ?4,
                 ?5, ?5, 'repo', ?5, 'startup_core')",
        params![task.project_id, topic_key, text, old_epoch, task.project],
    )?;
    conn.execute(
        "INSERT INTO memories
         (session_id, project, topic_key, title, content, memory_type,
          created_at_epoch, updated_at_epoch, status, scope,
          source_project, target_project, owner_scope, owner_key, context_class)
         VALUES ('legacy-current', ?1, ?2, 'Dev server', ?3, 'decision',
                 ?4, ?4, 'active', 'project',
                 ?1, ?1, 'repo', ?1, 'startup_core')",
        params![task.project, topic_key, text, old_epoch],
    )?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok(format!(
            "<memory_candidate>\
                <scope>project</scope>\
                <type>decision</type>\
                <topic_key>{topic_key}</topic_key>\
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
            pending_review: 0,
            to_event_id: task.high_watermark_event_id.expect("task watermark")
        }
    );
    let rows = conn
        .prepare(
            "SELECT status, expires_at_epoch
             FROM memories
             WHERE topic_key = ?1
             ORDER BY id ASC",
        )?
        .query_map([topic_key], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<i64>>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0, "stale");
    assert_eq!(rows[0].1, None);
    assert_eq!(rows[1].0, "active");
    assert!(rows[1].1.is_some());
    Ok(())
}
