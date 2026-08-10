//! Explicit production-shaped proof helpers for isolated test fixtures.
//!
//! These helpers are deliberately opt-in at each fixture construction site.
//! Generic memory test writers remain proofless so tests continue to exercise
//! the fail-closed legacy-unverified path.

use anyhow::Result;
use rusqlite::{params, Connection};

pub(crate) fn seed_current_memory_proof(conn: &Connection, memory_id: i64) -> Result<()> {
    seed_current_memory_proof_inner(conn, memory_id, true)
}

/// Opt-in proof for fixtures whose candidate inventory is part of the assertion.
/// A real captured event supplies provenance without adding a memory candidate.
pub(crate) fn seed_current_memory_direct_evidence_proof(
    conn: &Connection,
    memory_id: i64,
) -> Result<()> {
    seed_current_memory_proof_inner(conn, memory_id, false)
}

fn seed_current_memory_proof_inner(
    conn: &Connection,
    memory_id: i64,
    with_candidate: bool,
) -> Result<()> {
    let (project, topic_key, content, memory_type, created_at): (
        String,
        Option<String>,
        String,
        String,
        i64,
    ) = conn.query_row(
        "SELECT project, topic_key, content, memory_type, created_at_epoch
         FROM memories WHERE id = ?1",
        [memory_id],
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
    let topic_key = topic_key.unwrap_or_else(|| format!("g2-fixture-{memory_id}"));
    let event_id_override = format!("g2-fixture-event-{memory_id}");
    let session_id = format!("g2-fixture-session-{memory_id}");
    let event = crate::db::record_captured_event_with_id_and_created_at(
        conn,
        &crate::db::CaptureEventInput {
            host: "codex-cli",
            session_id: &session_id,
            project: &project,
            cwd: None,
            event_type: "message",
            role: Some("assistant"),
            tool_name: None,
            content: &content,
            task_kind: None,
        },
        Some(&event_id_override),
        created_at,
    )?;
    let canonical_project =
        crate::project_alias::resolve_project_identity(conn, &project)?.canonical_path;
    let evidence_event_ids = serde_json::to_string(&[event.event_row_id])?;
    let candidate_id = if with_candidate {
        let project_id: i64 = conn.query_row(
            "SELECT id FROM projects WHERE project_path = ?1 ORDER BY id DESC LIMIT 1",
            [&canonical_project],
            |row| row.get(0),
        )?;
        conn.execute(
            "INSERT INTO memory_candidates
             (project_id, scope, memory_type, topic_key, text, evidence_event_ids,
              confidence, risk_class, review_status, created_at_epoch, updated_at_epoch)
             VALUES (?1, 'project', ?2, ?3, ?4, ?5, 0.95, 'low', 'accepted', ?6, ?6)",
            params![
                project_id,
                memory_type,
                topic_key,
                content,
                evidence_event_ids,
                created_at
            ],
        )?;
        Some(conn.last_insert_rowid())
    } else {
        None
    };
    let state_key_id = if matches!(
        memory_type.as_str(),
        "decision" | "architecture" | "preference"
    ) {
        conn.execute(
            "INSERT INTO memory_state_keys
             (owner_scope, owner_key, memory_type, state_key, current_memory_id,
              created_at_epoch, updated_at_epoch)
            VALUES ('repo', ?1, ?2, ?3, ?4, ?5, ?5)",
            params![
                canonical_project,
                memory_type,
                format!("g2-fixture-state-{memory_id}"),
                memory_id,
                created_at
            ],
        )?;
        Some(conn.last_insert_rowid())
    } else {
        None
    };
    conn.execute(
        "UPDATE memories
         SET source_candidate_id = ?1, evidence_event_ids = ?2,
             confidence = 0.95, valid_from_epoch = ?3, state_key_id = ?4,
             source_project = COALESCE(source_project, ?5),
             target_project = COALESCE(target_project, ?5),
             owner_scope = COALESCE(owner_scope, 'repo'),
             owner_key = COALESCE(owner_key, ?5),
             context_class = COALESCE(context_class, 'startup_core')
         WHERE id = ?6",
        params![
            candidate_id,
            evidence_event_ids,
            created_at,
            state_key_id,
            canonical_project,
            memory_id
        ],
    )?;
    let updated = conn.changes();
    anyhow::ensure!(updated == 1, "fixture memory {memory_id} was not updated");
    let visibility =
        crate::truth::classify_memory(conn, memory_id, chrono::Utc::now().timestamp())?;
    anyhow::ensure!(
        visibility.current_context_eligible,
        "fixture memory {memory_id} proof classified as {:?}/{:?}",
        visibility.classification,
        visibility.reason
    );
    Ok(())
}
