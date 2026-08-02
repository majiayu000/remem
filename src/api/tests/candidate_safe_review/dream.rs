use axum::http::{Method, StatusCode};
use rusqlite::params;
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::api::DbState;
use crate::db;
use crate::db::test_support::ScopedTestDataDir;

use super::{candidate_version, insert_safe_review_candidate, response_json, send_safe_review};

pub(in crate::api::tests) fn insert_safe_dream_review_candidate(
    fixture: &str,
) -> anyhow::Result<(i64, Vec<i64>)> {
    insert_safe_dream_review_candidate_with_decision(fixture, "merge", &[0, 1])
}

pub(in crate::api::tests) fn insert_safe_dream_review_candidate_with_decision(
    fixture: &str,
    decision_kind: &str,
    intended_member_indexes: &[usize],
) -> anyhow::Result<(i64, Vec<i64>)> {
    insert_safe_dream_review_candidate_internal(
        fixture,
        decision_kind,
        intended_member_indexes,
        None,
        "decision",
        "Reviewed Dream title",
        "Reviewed Dream content",
    )
}

pub(in crate::api::tests) fn insert_safe_dream_review_candidate_with_payload(
    fixture: &str,
    topic_key: &str,
    memory_type: &str,
    title: &str,
    content: &str,
) -> anyhow::Result<(i64, Vec<i64>)> {
    insert_safe_dream_review_candidate_internal(
        fixture,
        "merge",
        &[0, 1],
        Some(topic_key),
        memory_type,
        title,
        content,
    )
}

fn insert_safe_dream_review_candidate_internal(
    fixture: &str,
    decision_kind: &str,
    intended_member_indexes: &[usize],
    topic_key_override: Option<&str>,
    memory_type: &str,
    title: &str,
    content: &str,
) -> anyhow::Result<(i64, Vec<i64>)> {
    let review_text = format!("{title}\n{content}");
    let (candidate_id, _) = insert_safe_review_candidate(
        fixture,
        "file_edit",
        "Dream source evidence is represented by memory provenance.",
        &review_text,
    )?;
    let conn = db::open_db()?;
    let default_topic_key: String = conn.query_row(
        "SELECT topic_key FROM memory_candidates WHERE id = ?1",
        params![candidate_id],
        |row| row.get(0),
    )?;
    let topic_key = topic_key_override.unwrap_or(&default_topic_key);
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
                memory_type,
                now
            ],
        )?;
        member_ids.push(conn.last_insert_rowid());
    }
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
            memory_type,
            topic_key,
            fixture,
            crate::memory::poisoning::INSTRUCTION_PATTERN_SET_VERSION,
            candidate_id
        ],
    )?;
    let intended_superseded_ids = intended_member_indexes
        .iter()
        .map(|index| {
            member_ids
                .get(*index)
                .copied()
                .ok_or_else(|| anyhow::anyhow!("invalid intended member index {index}"))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let generated_field = match decision_kind {
        "merge" => "dream.title",
        "no_merge" => "dream.no_merge_reason",
        "conflict" => "dream.conflict_reason",
        _ => "dream.unknown",
    };
    let decision_ids = match decision_kind {
        "merge" => intended_superseded_ids.clone(),
        "conflict" => member_ids.clone(),
        _ => Vec::new(),
    };
    let (generated_topic_key, generated_memory_type, generated_title, generated_content) =
        if decision_kind == "merge" {
            (
                Some(topic_key),
                Some(memory_type),
                Some(title),
                Some(content),
            )
        } else {
            (None, None, None, None)
        };
    let decision_payload_sha256 = match decision_kind {
        "merge" => {
            crate::dream::decision_payload_sha256(crate::dream::DreamDecisionPayload::Merge {
                topic_key,
                memory_type,
                title,
                content,
                intended_superseded_ids: &decision_ids,
            })
        }
        "conflict" => {
            crate::dream::decision_payload_sha256(crate::dream::DreamDecisionPayload::Conflict {
                conflicting_ids: &decision_ids,
                reason: &review_text,
            })
        }
        _ => crate::dream::decision_payload_sha256(crate::dream::DreamDecisionPayload::NoMerge {
            reason: &review_text,
        }),
    };
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
        crate::dream::cluster_signature_sha256(fixture, memory_type, &member_snapshots);
    conn.execute(
        "INSERT INTO dream_quarantine_artifacts
         (version, project, cluster_signature, member_ids_json,
          source_candidate_id, decision_kind, decision_ids_json,
          decision_payload_sha256, intended_superseded_ids_json,
          generated_topic_key, generated_memory_type, generated_title,
          generated_content, generated_field, pattern_id, pattern_version,
          source_operation, source_trust_class, created_at_epoch,
          updated_at_epoch)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                 ?13, 'override_previous_instructions', ?14, 'dream',
                 'external_content', ?15, ?15)",
        params![
            fixture,
            cluster_signature,
            serde_json::to_string(&member_ids)?,
            candidate_id,
            decision_kind,
            serde_json::to_string(&decision_ids)?,
            decision_payload_sha256,
            serde_json::to_string(&intended_superseded_ids)?,
            generated_topic_key,
            generated_memory_type,
            generated_title,
            generated_content,
            generated_field,
            crate::memory::poisoning::INSTRUCTION_PATTERN_SET_VERSION,
            now
        ],
    )?;
    Ok((candidate_id, member_ids))
}

async fn dream_review_token(candidate_id: i64, api_token: &str) -> anyhow::Result<String> {
    let app = super::super::super::build_router(0).with_state(DbState);
    let response = app
        .oneshot(super::super::authorized_json_request(
            Method::GET,
            &format!("/api/v1/candidates/{candidate_id}"),
            api_token,
            "",
        ))
        .await?;
    anyhow::ensure!(
        response.status() == StatusCode::OK,
        "candidate detail failed"
    );
    let payload = response_json(response).await?;
    payload["provenance"]["review_token"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("Dream review token missing"))
}

#[tokio::test]
async fn safe_dream_approval_requires_exact_review_token_without_side_effects() -> anyhow::Result<()>
{
    let _test_dir = ScopedTestDataDir::new("api-safe-dream-token-required");
    let (candidate_id, _) = insert_safe_dream_review_candidate("safe-dream-token-required")?;
    crate::api::ensure_api_token()?;
    let api_token = crate::api::load_api_token()?;
    let version = candidate_version(candidate_id)?;

    for (key, acknowledged_token, expected_code) in [
        ("dream-token-missing", None, "dream_provenance_ack_required"),
        (
            "dream-token-wrong",
            Some("sha256:wrong"),
            "dream_provenance_changed",
        ),
    ] {
        let mut body = json!({
            "reason": "reviewed Dream provenance",
            "expected_version": version,
            "idempotency_key": key,
            "acknowledge_pattern": "override_previous_instructions"
        });
        if let Some(acknowledged_token) = acknowledged_token {
            body["acknowledge_dream_review_token"] = json!(acknowledged_token);
        }
        let response = send_safe_review(candidate_id, "approve", &api_token, body).await?;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(
            response_json(response).await?["error"]["code"],
            expected_code
        );
    }

    let conn = db::open_db()?;
    let (status, promoted, audit, ledger): (String, i64, i64, i64) = conn.query_row(
        "SELECT c.review_status,
                (SELECT COUNT(*) FROM memories WHERE source_candidate_id = c.id),
                (SELECT COUNT(*) FROM events WHERE event_type = 'candidate_review'),
                (SELECT COUNT(*) FROM api_mutation_requests)
         FROM memory_candidates c WHERE c.id = ?1",
        params![candidate_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(status, "quarantined");
    assert_eq!((promoted, audit, ledger), (0, 0, 0));
    Ok(())
}

#[tokio::test]
async fn safe_dream_approval_supersedes_only_intended_members_and_audits_ids() -> anyhow::Result<()>
{
    let _test_dir = ScopedTestDataDir::new("api-safe-dream-supersede");
    let (candidate_id, member_ids) = insert_safe_dream_review_candidate("safe-dream-supersede")?;
    crate::api::ensure_api_token()?;
    let api_token = crate::api::load_api_token()?;
    let review_token = dream_review_token(candidate_id, &api_token).await?;
    let response = send_safe_review(
        candidate_id,
        "approve",
        &api_token,
        json!({
            "reason": "reviewed exact Dream merge decision",
            "expected_version": candidate_version(candidate_id)?,
            "idempotency_key": "dream-supersede-authorized",
            "acknowledge_pattern": "override_previous_instructions",
            "acknowledge_dream_review_token": review_token
        }),
    )
    .await?;
    let response_status = response.status();
    let response_payload = response_json(response).await?;
    assert_eq!(
        response_status,
        StatusCode::OK,
        "Dream approval response: {response_payload}"
    );
    let promoted_memory_id = response_payload["memory_id"]
        .as_i64()
        .ok_or_else(|| anyhow::anyhow!("Dream approval response missing memory_id"))?;

    let conn = db::open_db()?;
    for member_id in &member_ids {
        let status: String = conn.query_row(
            "SELECT status FROM memories WHERE id = ?1",
            params![member_id],
            |row| row.get(0),
        )?;
        assert_eq!(status, "stale");
    }
    let (topic_key, title, content): (String, String, String) = conn.query_row(
        "SELECT topic_key, title, content FROM memories WHERE id = ?1",
        params![promoted_memory_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(title, "Reviewed Dream title");
    assert_eq!(content, "Reviewed Dream content");
    let candidate_payload: (String, String) = conn.query_row(
        "SELECT topic_key, text FROM memory_candidates WHERE id = ?1",
        params![candidate_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(topic_key, candidate_payload.0);
    assert_eq!(
        candidate_payload.1,
        "Reviewed Dream title\nReviewed Dream content"
    );
    let provenance =
        crate::memory_candidate::review::load_dream_quarantine_provenance(&conn, candidate_id)?
            .ok_or_else(|| anyhow::anyhow!("approved Dream provenance missing"))?;
    assert!(!provenance.blocked_reasons.iter().any(|reason| matches!(
        reason.as_str(),
        "dream_provenance_malformed" | "dream_provenance_payload_mismatch"
    )));
    let audit_detail: String = conn.query_row(
        "SELECT detail FROM events WHERE event_type = 'candidate_review'",
        [],
        |row| row.get(0),
    )?;
    let audit: Value = serde_json::from_str(&audit_detail)?;
    assert_eq!(audit["authorized_supersede_ids"], json!(member_ids));
    assert_eq!(audit["actual_superseded_ids"], json!(member_ids));
    assert!(audit["dream_review_token"]
        .as_str()
        .is_some_and(|token| token.starts_with("sha256:")));
    let core_audit_detail: String = conn.query_row(
        "SELECT detail FROM events WHERE event_type = 'candidate_dream_review'",
        [],
        |row| row.get(0),
    )?;
    let core_audit: Value = serde_json::from_str(&core_audit_detail)?;
    assert_eq!(core_audit["authorized_supersede_ids"], json!(member_ids));
    assert_eq!(core_audit["actual_superseded_ids"], json!(member_ids));
    assert!(core_audit["dream_review_token"]
        .as_str()
        .is_some_and(|token| token.starts_with("sha256:")));
    Ok(())
}

#[tokio::test]
async fn safe_dream_approval_supersedes_exact_intended_subset_only() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-safe-dream-supersede-subset");
    let fixture = "safe-dream-supersede-subset";
    let (candidate_id, member_ids) =
        insert_safe_dream_review_candidate_with_decision(fixture, "merge", &[0])?;
    crate::api::ensure_api_token()?;
    let api_token = crate::api::load_api_token()?;
    let review_token = dream_review_token(candidate_id, &api_token).await?;

    let response = send_safe_review(
        candidate_id,
        "approve",
        &api_token,
        json!({
            "reason": "approve the exact reviewed subset",
            "expected_version": candidate_version(candidate_id)?,
            "idempotency_key": "dream-supersede-exact-subset",
            "acknowledge_pattern": "override_previous_instructions",
            "acknowledge_dream_review_token": review_token
        }),
    )
    .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let conn = db::open_db()?;
    let statuses = member_ids
        .iter()
        .map(|id| {
            conn.query_row(
                "SELECT status FROM memories WHERE id = ?1",
                params![id],
                |row| row.get::<_, String>(0),
            )
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    assert_eq!(statuses, vec!["stale", "active"]);
    let audit_detail: String = conn.query_row(
        "SELECT detail FROM events WHERE event_type = 'candidate_review'",
        [],
        |row| row.get(0),
    )?;
    let audit: Value = serde_json::from_str(&audit_detail)?;
    assert_eq!(audit["authorized_supersede_ids"], json!([member_ids[0]]));
    assert_eq!(audit["actual_superseded_ids"], json!([member_ids[0]]));
    Ok(())
}

#[tokio::test]
async fn safe_dream_approval_rolls_back_for_active_topic_outside_intended_set() -> anyhow::Result<()>
{
    let _test_dir = ScopedTestDataDir::new("api-safe-dream-supersede-blocked");
    let fixture = "safe-dream-supersede-blocked";
    let (candidate_id, member_ids) =
        insert_safe_dream_review_candidate_with_decision(fixture, "merge", &[0])?;
    let conn = db::open_db()?;
    let candidate_topic: String = conn.query_row(
        "SELECT topic_key FROM memory_candidates WHERE id = ?1",
        params![candidate_id],
        |row| row.get(0),
    )?;
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO memories
         (project, topic_key, title, content, memory_type, created_at_epoch,
          updated_at_epoch, status, scope, source_project, target_project,
          owner_scope, owner_key, source_trust_class)
         VALUES (?1, ?2, 'Unreviewed current target', 'Outside reviewed cluster',
                 'decision', ?3, ?3, 'active', 'project', ?1, ?1, 'repo', ?1,
                 'local_tool_output')",
        params![fixture, candidate_topic, now],
    )?;
    let unexpected_id = conn.last_insert_rowid();
    drop(conn);
    crate::api::ensure_api_token()?;
    let api_token = crate::api::load_api_token()?;
    let review_token = dream_review_token(candidate_id, &api_token).await?;

    let response = send_safe_review(
        candidate_id,
        "approve",
        &api_token,
        json!({
            "reason": "must not replace an active target outside reviewed provenance",
            "expected_version": candidate_version(candidate_id)?,
            "idempotency_key": "dream-supersede-unintended-member",
            "acknowledge_pattern": "override_previous_instructions",
            "acknowledge_dream_review_token": review_token
        }),
    )
    .await?;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(response).await?["error"]["code"],
        "candidate_review_rejected"
    );

    let conn = db::open_db()?;
    let candidate_status: String = conn.query_row(
        "SELECT review_status FROM memory_candidates WHERE id = ?1",
        params![candidate_id],
        |row| row.get(0),
    )?;
    assert_eq!(candidate_status, "quarantined");
    for memory_id in member_ids {
        let status: String = conn.query_row(
            "SELECT status FROM memories WHERE id = ?1",
            params![memory_id],
            |row| row.get(0),
        )?;
        assert_eq!(status, "active");
    }
    let unexpected_status: String = conn.query_row(
        "SELECT status FROM memories WHERE id = ?1",
        params![unexpected_id],
        |row| row.get(0),
    )?;
    assert_eq!(unexpected_status, "active");
    let audit_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE event_type = 'candidate_review'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(audit_count, 0);
    Ok(())
}

#[tokio::test]
async fn safe_reason_only_dream_decisions_only_allow_reject() -> anyhow::Result<()> {
    for (decision_kind, expected_code) in [
        ("no_merge", "dream_decision_no_merge_not_approvable"),
        ("conflict", "dream_decision_conflict_not_approvable"),
    ] {
        let fixture = format!("safe-dream-{decision_kind}-only-reject");
        let test_dir = ScopedTestDataDir::new(&fixture);
        let (candidate_id, member_ids) =
            insert_safe_dream_review_candidate_with_decision(&fixture, decision_kind, &[])?;
        crate::api::ensure_api_token()?;
        let api_token = crate::api::load_api_token()?;
        let review_token = dream_review_token(candidate_id, &api_token).await?;

        let approve = send_safe_review(
            candidate_id,
            "approve",
            &api_token,
            json!({
                "reason": "reason-only output cannot be promoted",
                "expected_version": candidate_version(candidate_id)?,
                "idempotency_key": format!("dream-{decision_kind}-approve"),
                "acknowledge_pattern": "override_previous_instructions",
                "acknowledge_dream_review_token": review_token
            }),
        )
        .await?;
        assert_eq!(approve.status(), StatusCode::CONFLICT);
        assert_eq!(
            response_json(approve).await?["error"]["code"],
            expected_code
        );

        let reject = send_safe_review(
            candidate_id,
            "reject",
            &api_token,
            json!({
                "reason": "discard reviewed reason-only output",
                "expected_version": candidate_version(candidate_id)?,
                "idempotency_key": format!("dream-{decision_kind}-reject")
            }),
        )
        .await?;
        assert_eq!(reject.status(), StatusCode::OK);
        let conn = db::open_db()?;
        let status: String = conn.query_row(
            "SELECT review_status FROM memory_candidates WHERE id = ?1",
            params![candidate_id],
            |row| row.get(0),
        )?;
        assert_eq!(status, "discarded");
        for member_id in member_ids {
            let status: String = conn.query_row(
                "SELECT status FROM memories WHERE id = ?1",
                params![member_id],
                |row| row.get(0),
            )?;
            assert_eq!(status, "active");
        }
        drop(conn);
        drop(test_dir);
    }
    Ok(())
}

#[tokio::test]
async fn safe_dream_approval_rejects_token_after_artifact_version_changes() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-safe-dream-token-stale");
    let (candidate_id, _) = insert_safe_dream_review_candidate("safe-dream-token-stale")?;
    crate::api::ensure_api_token()?;
    let api_token = crate::api::load_api_token()?;
    let review_token = dream_review_token(candidate_id, &api_token).await?;
    let conn = db::open_db()?;
    conn.execute(
        "UPDATE dream_quarantine_artifacts
         SET version = version + 1,
             occurrence_count = occurrence_count + 1,
             updated_at_epoch = updated_at_epoch + 1
         WHERE source_candidate_id = ?1",
        params![candidate_id],
    )?;
    drop(conn);

    let response = send_safe_review(
        candidate_id,
        "approve",
        &api_token,
        json!({
            "reason": "stale Dream review",
            "expected_version": candidate_version(candidate_id)?,
            "idempotency_key": "dream-token-stale",
            "acknowledge_pattern": "override_previous_instructions",
            "acknowledge_dream_review_token": review_token
        }),
    )
    .await?;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(response).await?["error"]["code"],
        "dream_provenance_changed"
    );
    Ok(())
}

#[tokio::test]
async fn safe_dream_approval_rejects_source_payload_change_without_epoch_change(
) -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-safe-dream-source-version-stale");
    let (candidate_id, member_ids) =
        insert_safe_dream_review_candidate("safe-dream-source-version-stale")?;
    crate::api::ensure_api_token()?;
    let api_token = crate::api::load_api_token()?;
    let review_token = dream_review_token(candidate_id, &api_token).await?;
    let expected_candidate_version = candidate_version(candidate_id)?;
    let conn = db::open_db()?;
    let (before_version, before_epoch): (i64, i64) = conn.query_row(
        "SELECT version, updated_at_epoch FROM memories WHERE id = ?1",
        params![member_ids[0]],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    conn.execute(
        "UPDATE memories SET content = content || ' changed after review' WHERE id = ?1",
        params![member_ids[0]],
    )?;
    let (after_version, after_epoch): (i64, i64) = conn.query_row(
        "SELECT version, updated_at_epoch FROM memories WHERE id = ?1",
        params![member_ids[0]],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert!(after_version > before_version);
    assert_eq!(after_epoch, before_epoch);
    drop(conn);

    let response = send_safe_review(
        candidate_id,
        "approve",
        &api_token,
        json!({
            "reason": "must reject a token for changed Dream source content",
            "expected_version": expected_candidate_version,
            "idempotency_key": "dream-source-version-stale",
            "acknowledge_pattern": "override_previous_instructions",
            "acknowledge_dream_review_token": review_token
        }),
    )
    .await?;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(response).await?["error"]["code"],
        "dream_provenance_stale"
    );

    let conn = db::open_db()?;
    let (status, version, promoted, audits, ledger): (String, i64, i64, i64, i64) = conn
        .query_row(
            "SELECT c.review_status, c.version,
                    (SELECT COUNT(*) FROM memories WHERE source_candidate_id = c.id),
                    (SELECT COUNT(*) FROM events
                     WHERE event_type IN ('candidate_review', 'candidate_dream_review')),
                    (SELECT COUNT(*) FROM api_mutation_requests)
             FROM memory_candidates c WHERE c.id = ?1",
            params![candidate_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )?;
    assert_eq!(status, "quarantined");
    assert_eq!(version, expected_candidate_version);
    assert_eq!((promoted, audits, ledger), (0, 0, 0));
    for member_id in member_ids {
        let status: String = conn.query_row(
            "SELECT status FROM memories WHERE id = ?1",
            params![member_id],
            |row| row.get(0),
        )?;
        assert_eq!(status, "active");
    }
    Ok(())
}

#[tokio::test]
async fn safe_reject_allows_dream_candidate_with_missing_provenance() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-safe-dream-reject-missing-provenance");
    let fixture = "safe-dream-reject-missing-provenance";
    let (candidate_id, _) = insert_safe_review_candidate(
        fixture,
        "file_edit",
        "Dream review evidence",
        "Malformed Dream quarantine",
    )?;
    let conn = db::open_db()?;
    conn.execute(
        "UPDATE memory_candidates
         SET review_status = 'quarantined', source_kind = 'dream_model_output',
             source_project = ?1, target_project = ?1,
             owner_scope = 'repo', owner_key = ?1,
             source_trust_class = 'external_content',
             quarantine_pattern_id = 'override_previous_instructions',
             quarantine_pattern_version = ?2
         WHERE id = ?3",
        params![
            fixture,
            crate::memory::poisoning::INSTRUCTION_PATTERN_SET_VERSION,
            candidate_id
        ],
    )?;
    drop(conn);
    crate::api::ensure_api_token()?;
    let api_token = crate::api::load_api_token()?;

    let response = send_safe_review(
        candidate_id,
        "reject",
        &api_token,
        json!({
            "reason": "discard malformed quarantine safely",
            "expected_version": candidate_version(candidate_id)?,
            "idempotency_key": "dream-reject-missing-provenance"
        }),
    )
    .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let conn = db::open_db()?;
    let status: String = conn.query_row(
        "SELECT review_status FROM memory_candidates WHERE id = ?1",
        params![candidate_id],
        |row| row.get(0),
    )?;
    assert_eq!(status, "discarded");
    Ok(())
}
