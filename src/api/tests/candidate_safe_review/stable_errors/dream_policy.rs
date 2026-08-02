use axum::http::StatusCode;
use rusqlite::{params, Connection};
use serde_json::json;

use crate::db;
use crate::db::test_support::ScopedTestDataDir;
use crate::memory::state_key::StateKeyDecision;
use crate::memory::suppression::{SuppressRequest, SuppressionTarget};

use super::super::{
    candidate_version, insert_safe_dream_review_candidate, insert_safe_review_candidate,
    response_json, send_safe_review,
};

const MEMORY_TYPE: &str = "decision";
const DREAM_TITLE: &str = "Reviewed Dream title";
const DREAM_CONTENT: &str = "Reviewed Dream content";

fn insert_state_keyed_dream_review_candidate(
    fixture: &str,
) -> anyhow::Result<(i64, Vec<i64>, i64, StateKeyDecision)> {
    let review_text = format!("{DREAM_TITLE}\n{DREAM_CONTENT}");
    let (candidate_id, _) = insert_safe_review_candidate(
        fixture,
        "file_edit",
        "Dream source evidence is represented by memory provenance.",
        &review_text,
    )?;
    let conn = db::open_db()?;
    let topic_key: String = conn.query_row(
        "SELECT topic_key FROM memory_candidates WHERE id = ?1",
        params![candidate_id],
        |row| row.get(0),
    )?;
    let now = chrono::Utc::now().timestamp();
    let mut member_ids = Vec::new();
    for index in 1..=2 {
        conn.execute(
            "INSERT INTO memories
             (project, topic_key, title, content, memory_type, created_at_epoch,
              updated_at_epoch, status, scope, source_project, target_project,
              owner_scope, owner_key, source_trust_class)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, 'active', 'project',
                     ?1, ?1, 'repo', ?1, 'local_tool_output')",
            params![
                fixture,
                format!("dream-source-{fixture}-{index}"),
                format!("Dream source {index}"),
                format!("Dream source content {index}"),
                MEMORY_TYPE,
                now
            ],
        )?;
        member_ids.push(conn.last_insert_rowid());
    }
    conn.execute(
        "INSERT INTO memories
         (project, topic_key, title, content, memory_type, created_at_epoch,
          updated_at_epoch, status, scope, source_project, target_project,
          owner_scope, owner_key, source_trust_class)
         VALUES (?1, ?2, 'Replacement current source',
                 'Replacement current source content', ?3, ?4, ?4,
                 'active', 'project', ?1, ?1, 'repo', ?1, 'local_tool_output')",
        params![
            fixture,
            format!("dream-replacement-{fixture}"),
            MEMORY_TYPE,
            now
        ],
    )?;
    let replacement_id = conn.last_insert_rowid();
    let state_key = StateKeyDecision {
        state_key: format!("reviewed-dream-source-{fixture}"),
        confidence: 1.0,
        reason: "stable_topic_key".to_string(),
    };
    crate::memory::state_key::attach_current_memory(
        &conn,
        member_ids[0],
        "repo",
        fixture,
        MEMORY_TYPE,
        &state_key,
        now,
    )?;
    conn.execute(
        "UPDATE memory_candidates
         SET evidence_event_ids = '[]', review_status = 'quarantined',
             memory_type = ?1, topic_key = ?2,
             source_kind = 'dream_model_output', source_project = ?3,
             target_project = ?3, owner_scope = 'repo', owner_key = ?3,
             source_trust_class = 'external_content',
             quarantine_pattern_id = 'override_previous_instructions',
             quarantine_pattern_version = ?4
         WHERE id = ?5",
        params![
            MEMORY_TYPE,
            topic_key,
            fixture,
            crate::memory::poisoning::INSTRUCTION_PATTERN_SET_VERSION,
            candidate_id
        ],
    )?;
    let member_snapshots = member_ids
        .iter()
        .map(|id| {
            conn.query_row(
                "SELECT id, version, updated_at_epoch, topic_key, title, content
                 FROM memories WHERE id = ?1",
                params![id],
                |row| {
                    Ok(crate::dream::DreamClusterMemberSnapshot {
                        id: row.get(0)?,
                        version: row.get(1)?,
                        updated_at_epoch: row.get(2)?,
                        topic_key: row.get(3)?,
                        title: row.get(4)?,
                        content: row.get(5)?,
                    })
                },
            )
        })
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let cluster_signature =
        crate::dream::cluster_signature_sha256(fixture, MEMORY_TYPE, &member_snapshots);
    let decision_payload_sha256 =
        crate::dream::decision_payload_sha256(crate::dream::DreamDecisionPayload::Merge {
            topic_key: &topic_key,
            memory_type: MEMORY_TYPE,
            title: DREAM_TITLE,
            content: DREAM_CONTENT,
            intended_superseded_ids: &member_ids,
        });
    let member_ids_json = serde_json::to_string(&member_ids)?;
    conn.execute(
        "INSERT INTO dream_quarantine_artifacts
         (version, project, cluster_signature, member_ids_json,
          source_candidate_id, decision_kind, decision_ids_json,
          decision_payload_sha256, intended_superseded_ids_json,
          generated_topic_key, generated_memory_type, generated_title,
          generated_content, generated_field, pattern_id, pattern_version,
          source_operation, source_trust_class, created_at_epoch,
          updated_at_epoch)
         VALUES (1, ?1, ?2, ?3, ?4, 'merge', ?3, ?5, ?3, ?6, ?7, ?8, ?9,
                 'dream.title', 'override_previous_instructions', ?10, 'dream',
                 'external_content', ?11, ?11)",
        params![
            fixture,
            cluster_signature,
            member_ids_json,
            candidate_id,
            decision_payload_sha256,
            topic_key,
            MEMORY_TYPE,
            DREAM_TITLE,
            DREAM_CONTENT,
            crate::memory::poisoning::INSTRUCTION_PATTERN_SET_VERSION,
            now
        ],
    )?;
    Ok((candidate_id, member_ids, replacement_id, state_key))
}

fn current_review_token(conn: &Connection, candidate_id: i64) -> anyhow::Result<String> {
    crate::memory_candidate::review::load_dream_quarantine_provenance(conn, candidate_id)?
        .and_then(|provenance| provenance.review_token)
        .ok_or_else(|| anyhow::anyhow!("Dream review token missing"))
}

fn assert_stale_provenance(conn: &Connection, candidate_id: i64) -> anyhow::Result<()> {
    let provenance =
        crate::memory_candidate::review::load_dream_quarantine_provenance(conn, candidate_id)?
            .ok_or_else(|| anyhow::anyhow!("Dream provenance missing"))?;
    assert!(provenance
        .blocked_reasons
        .iter()
        .any(|reason| reason == "dream_provenance_stale"));
    assert!(provenance.review_token.is_none());
    Ok(())
}

fn assert_no_review_side_effects(
    conn: &Connection,
    candidate_id: i64,
    expected_version: i64,
    active_memory_ids: &[i64],
) -> anyhow::Result<()> {
    let state: (String, i64, i64, i64, i64, i64, i64) = conn.query_row(
        "SELECT c.review_status, c.version,
                (SELECT COUNT(*) FROM memories WHERE source_candidate_id = c.id),
                (SELECT COUNT(*) FROM events
                 WHERE event_type IN ('candidate_review', 'candidate_dream_review')),
                (SELECT COUNT(*) FROM api_mutation_requests),
                (SELECT COUNT(*) FROM memory_operation_log
                 WHERE source_candidate_id = c.id),
                (SELECT COUNT(*) FROM memory_edges WHERE source_candidate_id = c.id)
         FROM memory_candidates c WHERE c.id = ?1",
        params![candidate_id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        },
    )?;
    assert_eq!(state.0, "quarantined");
    assert_eq!(state.1, expected_version);
    assert_eq!(
        (state.2, state.3, state.4, state.5, state.6),
        (0, 0, 0, 0, 0)
    );
    for memory_id in active_memory_ids {
        let status: String = conn.query_row(
            "SELECT status FROM memories WHERE id = ?1",
            params![memory_id],
            |row| row.get(0),
        )?;
        assert_eq!(status, "active");
    }
    Ok(())
}

#[tokio::test]
async fn safe_dream_state_pointer_change_invalidates_old_token_without_side_effects(
) -> anyhow::Result<()> {
    let fixture = "safe-dream-state-pointer-stale";
    let _test_dir = ScopedTestDataDir::new(fixture);
    let (candidate_id, member_ids, replacement_id, state_key) =
        insert_state_keyed_dream_review_candidate(fixture)?;
    let expected_version = candidate_version(candidate_id)?;
    let conn = db::open_db()?;
    let old_token = current_review_token(&conn, candidate_id)?;
    let source_snapshot_before: (i64, i64) = conn.query_row(
        "SELECT version, updated_at_epoch FROM memories WHERE id = ?1",
        params![member_ids[0]],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    crate::memory::state_key::attach_current_memory(
        &conn,
        replacement_id,
        "repo",
        fixture,
        MEMORY_TYPE,
        &state_key,
        chrono::Utc::now().timestamp() + 1,
    )?;
    let source_snapshot_after: (i64, i64) = conn.query_row(
        "SELECT version, updated_at_epoch FROM memories WHERE id = ?1",
        params![member_ids[0]],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(source_snapshot_after, source_snapshot_before);
    assert_stale_provenance(&conn, candidate_id)?;
    drop(conn);

    crate::api::ensure_api_token()?;
    let api_token = crate::api::load_api_token()?;
    let response = send_safe_review(
        candidate_id,
        "approve",
        &api_token,
        json!({
            "reason": "state-key pointer changed after Dream review",
            "expected_version": expected_version,
            "idempotency_key": "dream-state-pointer-stale-token",
            "acknowledge_pattern": "override_previous_instructions",
            "acknowledge_dream_review_token": old_token
        }),
    )
    .await?;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(response).await?["error"]["code"],
        "dream_provenance_stale"
    );

    let conn = db::open_db()?;
    let mut active_ids = member_ids;
    active_ids.push(replacement_id);
    assert_no_review_side_effects(&conn, candidate_id, expected_version, &active_ids)
}

#[tokio::test]
async fn safe_dream_source_suppression_invalidates_old_token_without_side_effects(
) -> anyhow::Result<()> {
    let fixture = "safe-dream-source-suppressed";
    let _test_dir = ScopedTestDataDir::new(fixture);
    let (candidate_id, member_ids) = insert_safe_dream_review_candidate(fixture)?;
    let expected_version = candidate_version(candidate_id)?;
    let conn = db::open_db()?;
    let old_token = current_review_token(&conn, candidate_id)?;
    let source_snapshot_before: (i64, i64) = conn.query_row(
        "SELECT version, updated_at_epoch FROM memories WHERE id = ?1",
        params![member_ids[0]],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    crate::memory::suppression::create_suppression(
        &conn,
        &SuppressRequest {
            target: SuppressionTarget {
                kind: "memory".to_string(),
                id: Some(member_ids[0]),
                value: None,
            },
            reason: Some("source invalidated after Dream review"),
            actor: Some("test"),
        },
    )?;
    let source_snapshot_after: (i64, i64) = conn.query_row(
        "SELECT version, updated_at_epoch FROM memories WHERE id = ?1",
        params![member_ids[0]],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(source_snapshot_after, source_snapshot_before);
    assert_stale_provenance(&conn, candidate_id)?;
    drop(conn);

    crate::api::ensure_api_token()?;
    let api_token = crate::api::load_api_token()?;
    let response = send_safe_review(
        candidate_id,
        "approve",
        &api_token,
        json!({
            "reason": "source suppression changed after Dream review",
            "expected_version": expected_version,
            "idempotency_key": "dream-source-suppressed-stale-token",
            "acknowledge_pattern": "override_previous_instructions",
            "acknowledge_dream_review_token": old_token
        }),
    )
    .await?;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(response).await?["error"]["code"],
        "dream_provenance_stale"
    );

    let conn = db::open_db()?;
    assert_no_review_side_effects(&conn, candidate_id, expected_version, &member_ids)
}
