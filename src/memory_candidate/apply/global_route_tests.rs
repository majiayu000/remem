use anyhow::Result;
use rusqlite::Connection;

use super::*;
use crate::db::{record_captured_event, CaptureEventInput, ExtractionTaskKind};

#[test]
fn global_candidate_replacement_reuses_the_existing_cross_project_route() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    conn.execute(
        "INSERT INTO memories
         (project, topic_key, title, content, memory_type, created_at_epoch,
          updated_at_epoch, status, scope, source_project, owner_scope, owner_key,
          context_class, source_trust_class)
         VALUES ('project-a', 'shared-global-candidate', 'Old global decision',
                 'Old shared value.', 'decision', 1, 1, 'active', 'global',
                 'project-a', 'user', 'user:default', 'startup_core', 'user_prompt')",
        [],
    )?;
    let old_id = conn.last_insert_rowid();
    let captured = record_captured_event(
        &conn,
        &CaptureEventInput {
            host: "codex-cli",
            session_id: "global-route",
            project: "project-b",
            cwd: None,
            event_type: "user_prompt",
            role: Some("user"),
            tool_name: None,
            content: "Use the new shared global value.",
            task_kind: Some(ExtractionTaskKind::MemoryCandidate),
        },
    )?;
    let evidence_json = serde_json::to_string(&vec![captured.event_row_id])?;
    conn.execute(
        "INSERT INTO memory_candidates
         (id, scope, memory_type, topic_key, text, evidence_event_ids, confidence,
          risk_class, review_status, created_at_epoch, updated_at_epoch)
         VALUES (701, 'global', 'decision', 'shared-global-candidate',
                 'Use the new shared global value.', ?1, 0.95, 'low',
                 'approved', 2, 2)",
        [evidence_json.as_str()],
    )?;
    let candidate = ParsedMemoryCandidate {
        scope: "global".to_string(),
        memory_type: "decision".to_string(),
        topic_key: "shared-global-candidate".to_string(),
        title_override: Some("New global decision".to_string()),
        text: "Use the new shared global value.".to_string(),
        confidence: 0.95,
        risk_class: "low".to_string(),
        outcome: None,
        facts: Vec::new(),
    };
    let route = CandidateRoute {
        owner_scope: "user".to_string(),
        owner_key: "user:default".to_string(),
        target_project: None,
        topic_domain: Some("user-preference".to_string()),
        routing_confidence: 0.95,
        routing_reason: "global candidate route".to_string(),
        context_class: "startup_core".to_string(),
    };

    let outcome = promote_candidate_to_memory_with_route(
        &conn,
        Some("global-route"),
        "project-b",
        701,
        &candidate,
        &evidence_json,
        &route,
        SourceTrustClass::UserPrompt,
    )?;
    let new_id = outcome
        .memory_id
        .context("global candidate was not promoted")?;
    assert_ne!(new_id, old_id);
    assert_eq!(outcome.superseded_ids, vec![old_id]);
    assert_eq!(
        conn.query_row(
            "SELECT project, source_project FROM memories WHERE id = ?1",
            [new_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )?,
        ("project-a".to_string(), "project-b".to_string())
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM memories
             WHERE topic_key = 'shared-global-candidate' AND status = 'active'",
            [],
            |row| row.get::<_, i64>(0),
        )?,
        1
    );
    Ok(())
}
