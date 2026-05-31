use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use super::{
    normalize_memory_type, normalize_scope, normalize_topic_key,
    promote_candidate_to_memory_with_route, route_candidate, update_candidate_after_lifecycle,
    CandidateRoute, ParsedMemoryCandidate,
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReviewCandidate {
    pub id: i64,
    pub project: Option<String>,
    pub scope: String,
    pub memory_type: String,
    pub topic_key: String,
    pub text: String,
    pub evidence_event_ids: String,
    pub evidence_preview: Vec<String>,
    pub confidence: f64,
    pub risk_class: String,
    pub created_at_epoch: i64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct CandidateEdit {
    pub scope: Option<String>,
    pub memory_type: Option<String>,
    pub topic_key: Option<String>,
    pub text: Option<String>,
}

#[derive(Debug, Clone)]
struct CandidateRow {
    id: i64,
    project: Option<String>,
    source_project: Option<String>,
    target_project: Option<String>,
    owner_scope: Option<String>,
    owner_key: Option<String>,
    topic_domain: Option<String>,
    routing_confidence: Option<f64>,
    routing_reason: Option<String>,
    context_class: Option<String>,
    scope: String,
    memory_type: String,
    topic_key: String,
    text: String,
    evidence_event_ids: String,
    confidence: f64,
    risk_class: String,
    review_status: String,
    created_at_epoch: i64,
}

pub(crate) fn list_pending(
    conn: &Connection,
    project: Option<&str>,
    limit: i64,
) -> Result<Vec<ReviewCandidate>> {
    let limit = limit.clamp(1, 200);
    let rows = if let Some(project) = project {
        let mut stmt = conn.prepare(
            "SELECT c.id, p.project_path, c.scope, c.memory_type, c.topic_key,
                    c.text, c.evidence_event_ids, c.confidence, c.risk_class,
                    c.review_status, c.created_at_epoch, c.source_project,
                    c.target_project, c.owner_scope, c.owner_key, c.topic_domain,
                    c.routing_confidence, c.routing_reason, c.context_class
             FROM memory_candidates c
             LEFT JOIN projects p ON p.id = c.project_id
             WHERE c.review_status = 'pending_review'
               AND p.project_path = ?1
             ORDER BY c.created_at_epoch ASC, c.id ASC
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![project, limit], CandidateRow::from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    } else {
        let mut stmt = conn.prepare(
            "SELECT c.id, p.project_path, c.scope, c.memory_type, c.topic_key,
                    c.text, c.evidence_event_ids, c.confidence, c.risk_class,
                    c.review_status, c.created_at_epoch, c.source_project,
                    c.target_project, c.owner_scope, c.owner_key, c.topic_domain,
                    c.routing_confidence, c.routing_reason, c.context_class
             FROM memory_candidates c
             LEFT JOIN projects p ON p.id = c.project_id
             WHERE c.review_status = 'pending_review'
             ORDER BY c.created_at_epoch ASC, c.id ASC
             LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit], CandidateRow::from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };

    rows.into_iter()
        .map(|row| {
            let evidence_preview = evidence_preview(conn, &row.evidence_event_ids)?;
            Ok(ReviewCandidate {
                id: row.id,
                project: row.project,
                scope: row.scope,
                memory_type: row.memory_type,
                topic_key: row.topic_key,
                text: row.text,
                evidence_event_ids: row.evidence_event_ids,
                evidence_preview,
                confidence: row.confidence,
                risk_class: row.risk_class,
                created_at_epoch: row.created_at_epoch,
            })
        })
        .collect()
}

pub(crate) fn approve_candidate(conn: &mut Connection, id: i64) -> Result<Option<i64>> {
    let Some(row) = load_candidate(conn, id)? else {
        return Ok(None);
    };
    ensure_pending(&row)?;
    let tx = conn.transaction()?;
    let memory_id = promote_row(&tx, &row, "approved", None)?;
    tx.commit()?;
    Ok(Some(memory_id))
}

pub(crate) fn discard_candidate(conn: &Connection, id: i64) -> Result<bool> {
    let now = chrono::Utc::now().timestamp();
    let updated = conn.execute(
        "UPDATE memory_candidates
         SET review_status = 'discarded', updated_at_epoch = ?1
         WHERE id = ?2 AND review_status = 'pending_review'",
        params![now, id],
    )?;
    Ok(updated > 0)
}

pub(crate) fn edit_candidate(
    conn: &mut Connection,
    id: i64,
    edit: CandidateEdit,
) -> Result<Option<i64>> {
    if edit.scope.is_none()
        && edit.memory_type.is_none()
        && edit.topic_key.is_none()
        && edit.text.is_none()
    {
        bail!("edit requires at least one changed field");
    }
    let Some(row) = load_candidate(conn, id)? else {
        return Ok(None);
    };
    ensure_pending(&row)?;
    let edited = row.apply_edit(edit)?;
    let tx = conn.transaction()?;
    let memory_id = promote_row(&tx, &row, "edited", Some(&edited))?;
    tx.commit()?;
    Ok(Some(memory_id))
}

impl CandidateRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            project: row.get(1)?,
            scope: row.get(2)?,
            memory_type: row.get(3)?,
            topic_key: row.get(4)?,
            text: row.get(5)?,
            evidence_event_ids: row.get(6)?,
            confidence: row.get(7)?,
            risk_class: row.get(8)?,
            review_status: row.get(9)?,
            created_at_epoch: row.get(10)?,
            source_project: row.get(11)?,
            target_project: row.get(12)?,
            owner_scope: row.get(13)?,
            owner_key: row.get(14)?,
            topic_domain: row.get(15)?,
            routing_confidence: row.get(16)?,
            routing_reason: row.get(17)?,
            context_class: row.get(18)?,
        })
    }

    fn as_candidate(&self) -> ParsedMemoryCandidate {
        ParsedMemoryCandidate {
            scope: self.scope.clone(),
            memory_type: self.memory_type.clone(),
            topic_key: self.topic_key.clone(),
            text: self.text.clone(),
            confidence: self.confidence,
            risk_class: self.risk_class.clone(),
        }
    }

    fn apply_edit(&self, edit: CandidateEdit) -> Result<ParsedMemoryCandidate> {
        let scope = edit
            .scope
            .as_deref()
            .map(normalize_scope)
            .transpose()?
            .unwrap_or_else(|| self.scope.clone());
        let memory_type = edit
            .memory_type
            .as_deref()
            .map(normalize_memory_type)
            .transpose()?
            .unwrap_or_else(|| self.memory_type.clone());
        let topic_key = edit
            .topic_key
            .as_deref()
            .map(normalize_topic_key)
            .transpose()?
            .unwrap_or_else(|| self.topic_key.clone());
        let text = edit
            .text
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| self.text.clone());
        Ok(ParsedMemoryCandidate {
            scope,
            memory_type,
            topic_key,
            text,
            confidence: self.confidence,
            risk_class: self.risk_class.clone(),
        })
    }

    fn route_for(&self, candidate: &ParsedMemoryCandidate) -> CandidateRoute {
        match (
            self.owner_scope.as_ref(),
            self.owner_key.as_ref(),
            self.routing_confidence,
            self.routing_reason.as_ref(),
            self.context_class.as_ref(),
        ) {
            (
                Some(owner_scope),
                Some(owner_key),
                Some(routing_confidence),
                Some(routing_reason),
                Some(context_class),
            ) => CandidateRoute {
                owner_scope: owner_scope.clone(),
                owner_key: owner_key.clone(),
                target_project: self.target_project.clone(),
                topic_domain: self.topic_domain.clone(),
                routing_confidence,
                routing_reason: routing_reason.clone(),
                context_class: context_class.clone(),
            },
            _ => {
                let project = self
                    .source_project
                    .as_deref()
                    .or(self.project.as_deref())
                    .unwrap_or("<unknown>");
                route_candidate(project, None, candidate, std::iter::empty())
            }
        }
    }
}

fn load_candidate(conn: &Connection, id: i64) -> Result<Option<CandidateRow>> {
    conn.query_row(
        "SELECT c.id, p.project_path, c.scope, c.memory_type, c.topic_key,
                c.text, c.evidence_event_ids, c.confidence, c.risk_class,
                c.review_status, c.created_at_epoch, c.source_project,
                c.target_project, c.owner_scope, c.owner_key, c.topic_domain,
                c.routing_confidence, c.routing_reason, c.context_class
         FROM memory_candidates c
         LEFT JOIN projects p ON p.id = c.project_id
         WHERE c.id = ?1",
        params![id],
        CandidateRow::from_row,
    )
    .optional()
    .map_err(Into::into)
}

fn ensure_pending(row: &CandidateRow) -> Result<()> {
    if row.review_status != "pending_review" {
        bail!(
            "candidate {} is {}, expected pending_review",
            row.id,
            row.review_status
        );
    }
    Ok(())
}

fn promote_row(
    conn: &Connection,
    row: &CandidateRow,
    review_status: &str,
    edited: Option<&ParsedMemoryCandidate>,
) -> Result<i64> {
    let project = row
        .source_project
        .as_deref()
        .or(row.project.as_deref())
        .context("candidate is missing source project path")?;
    let candidate = edited.cloned().unwrap_or_else(|| row.as_candidate());
    let route = if edited.is_some() {
        route_candidate(project, None, &candidate, std::iter::empty())
    } else {
        row.route_for(&candidate)
    };
    let outcome = promote_candidate_to_memory_with_route(
        conn,
        None,
        project,
        row.id,
        &candidate,
        &row.evidence_event_ids,
        &route,
    )?;
    let status = outcome.review_status_for(review_status);
    let now = chrono::Utc::now().timestamp();
    update_candidate_after_lifecycle(conn, row.id, &candidate, &route, status)?;
    conn.execute(
        "UPDATE memory_candidates SET updated_at_epoch = ?1 WHERE id = ?2",
        params![now, row.id],
    )?;
    outcome
        .memory_id
        .context("candidate promotion produced no memory id")
}

fn evidence_preview(conn: &Connection, evidence_json: &str) -> Result<Vec<String>> {
    let event_ids: Vec<i64> = serde_json::from_str(evidence_json)
        .with_context(|| "candidate has malformed evidence_event_ids")?;
    let mut previews = Vec::new();
    for event_id in event_ids.into_iter().take(3) {
        let row: Option<(String, Option<String>, String)> = conn
            .query_row(
                "SELECT event_type, tool_name, COALESCE(content_text, '')
                 FROM captured_events WHERE id = ?1",
                params![event_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if let Some((event_type, tool_name, content)) = row {
            let tool = tool_name
                .map(|value| format!(" tool={value}"))
                .unwrap_or_default();
            previews.push(format!(
                "#{} {}{} {}",
                event_id,
                event_type,
                tool,
                crate::db::truncate_str(&content, 120)
            ));
        } else {
            previews.push(format!("#{event_id} <missing event>"));
        }
    }
    Ok(previews)
}

#[cfg(test)]
mod tests {
    use rusqlite::params;

    use crate::db::{record_captured_event, CaptureEventInput, ExtractionTaskKind};

    use super::*;

    fn setup_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db should open");
        crate::migrate::run_migrations(&conn).expect("migrations should run");
        conn
    }

    fn insert_pending_candidate(conn: &mut Connection, topic_key: &str, text: &str) -> Result<i64> {
        insert_pending_candidate_with_scope(conn, topic_key, text, "project")
    }

    fn insert_pending_candidate_with_scope(
        conn: &mut Connection,
        topic_key: &str,
        text: &str,
        scope: &str,
    ) -> Result<i64> {
        insert_pending_candidate_with_scope_and_type(conn, topic_key, text, scope, "decision")
    }

    fn insert_pending_candidate_with_scope_and_type(
        conn: &mut Connection,
        topic_key: &str,
        text: &str,
        scope: &str,
        memory_type: &str,
    ) -> Result<i64> {
        record_captured_event(
            conn,
            &CaptureEventInput {
                host: "codex-cli",
                session_id: "sess-review",
                project: "/tmp/remem",
                cwd: None,
                event_type: "tool_result",
                role: None,
                tool_name: Some("Bash"),
                content: "cargo test passed",
                task_kind: Some(ExtractionTaskKind::MemoryCandidate),
            },
        )?;
        let task = crate::db::claim_next_extraction_task(conn, "worker-review", 60)?
            .expect("task should claim");
        let evidence_json = serde_json::to_string(&vec![task.high_watermark_event_id.unwrap()])?;
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO memory_candidates
             (project_id, scope, memory_type, topic_key, text, evidence_event_ids,
              confidence, risk_class, review_status, created_at_epoch, updated_at_epoch)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0.72, 'medium',
                     'pending_review', ?7, ?7)",
            params![
                task.project_id,
                scope,
                memory_type,
                topic_key,
                text,
                evidence_json,
                now
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    #[test]
    fn review_list_includes_evidence_preview() -> Result<()> {
        let mut conn = setup_conn();
        let id = insert_pending_candidate(&mut conn, "review-list", "Review this candidate")?;

        let rows = list_pending(&conn, None, 10)?;

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].project.as_deref(), Some("/tmp/remem"));
        assert!(rows[0].evidence_preview[0].contains("tool_result"));
        Ok(())
    }

    #[test]
    fn review_approve_promotes_candidate() -> Result<()> {
        let mut conn = setup_conn();
        let id = insert_pending_candidate(&mut conn, "review-approve", "Approve this memory")?;

        let memory_id = approve_candidate(&mut conn, id)?.expect("candidate should approve");

        let status: String = conn.query_row(
            "SELECT review_status FROM memory_candidates WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        let source_candidate_id: i64 = conn.query_row(
            "SELECT source_candidate_id FROM memories WHERE id = ?1",
            params![memory_id],
            |row| row.get(0),
        )?;
        assert_eq!(status, "approved");
        assert_eq!(source_candidate_id, id);
        Ok(())
    }

    #[test]
    fn review_approve_lesson_candidate_creates_metadata() -> Result<()> {
        let mut conn = setup_conn();
        let id = insert_pending_candidate_with_scope_and_type(
            &mut conn,
            "review-lesson",
            "Lesson: generic lesson promotions must keep metadata so context can load them.",
            "project",
            "lesson",
        )?;

        let memory_id = approve_candidate(&mut conn, id)?.expect("candidate should approve");

        let (memory_type, metadata_count): (String, i64) = conn.query_row(
            "SELECT m.memory_type, COUNT(l.memory_id)
             FROM memories m
             LEFT JOIN memory_lessons l ON l.memory_id = m.id
             WHERE m.id = ?1",
            params![memory_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(memory_type, "lesson");
        assert_eq!(metadata_count, 1);
        Ok(())
    }

    #[test]
    fn review_approve_lesson_candidate_supersedes_old_lesson() -> Result<()> {
        let mut conn = setup_conn();
        let old_id = crate::memory::lesson::save_lesson(
            &conn,
            &crate::memory::lesson::SaveLessonRequest {
                session_id: None,
                project: "/tmp/remem",
                topic_key: Some("review-lesson-update"),
                title: "Old lesson",
                content: "Old lesson content",
                confidence: 0.8,
                source_evidence: None,
                files: None,
                branch: None,
                scope: "project",
                created_at_epoch: None,
                stale_after_epoch: None,
            },
        )?;
        let id = insert_pending_candidate_with_scope_and_type(
            &mut conn,
            "review-lesson-update",
            "Updated lesson content",
            "project",
            "lesson",
        )?;

        let new_id = approve_candidate(&mut conn, id)?.expect("candidate should approve");

        let old_status: String = conn.query_row(
            "SELECT status FROM memories WHERE id = ?1",
            params![old_id],
            |row| row.get(0),
        )?;
        let (content, metadata_count): (String, i64) = conn.query_row(
            "SELECT m.content, COUNT(l.memory_id)
             FROM memories m
             LEFT JOIN memory_lessons l ON l.memory_id = m.id
             WHERE m.id = ?1",
            params![new_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let memory_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
        assert_eq!(old_status, "stale");
        assert_eq!(content, "Updated lesson content");
        assert_eq!(metadata_count, 1);
        assert_eq!(memory_count, 2);
        Ok(())
    }

    #[test]
    fn review_discard_marks_candidate_without_deleting_evidence() -> Result<()> {
        let mut conn = setup_conn();
        let id = insert_pending_candidate(&mut conn, "review-discard", "Discard this memory")?;

        assert!(discard_candidate(&conn, id)?);

        let (status, evidence): (String, String) = conn.query_row(
            "SELECT review_status, evidence_event_ids FROM memory_candidates WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(status, "discarded");
        assert!(evidence.contains('1'));
        Ok(())
    }

    #[test]
    fn review_edit_promotes_edited_candidate() -> Result<()> {
        let mut conn = setup_conn();
        let id = insert_pending_candidate(&mut conn, "review-edit", "Original memory")?;

        let memory_id = edit_candidate(
            &mut conn,
            id,
            CandidateEdit {
                topic_key: Some("edited-topic".to_string()),
                memory_type: Some("architecture".to_string()),
                text: Some("Edited architecture memory".to_string()),
                ..CandidateEdit::default()
            },
        )?
        .expect("candidate should edit");

        let (status, topic_key, memory_type, text): (String, String, String, String) = conn
            .query_row(
                "SELECT c.review_status, m.topic_key, m.memory_type, m.content
                 FROM memory_candidates c
                 JOIN memories m ON m.id = ?2
                 WHERE c.id = ?1",
                params![id, memory_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;
        assert_eq!(status, "edited");
        assert_eq!(topic_key, "edited-topic");
        assert_eq!(memory_type, "architecture");
        assert_eq!(text, "Edited architecture memory");
        Ok(())
    }

    #[test]
    fn review_invalid_ids_are_reported() -> Result<()> {
        let mut conn = setup_conn();

        assert!(approve_candidate(&mut conn, 999)?.is_none());
        assert!(!discard_candidate(&conn, 999)?);
        assert!(edit_candidate(
            &mut conn,
            999,
            CandidateEdit {
                text: Some("missing".to_string()),
                ..CandidateEdit::default()
            },
        )?
        .is_none());
        Ok(())
    }

    #[test]
    fn review_approve_supersedes_duplicate_topic_memory() -> Result<()> {
        let mut conn = setup_conn();
        let old_id = crate::memory::insert_memory_full(
            &conn,
            None,
            "/tmp/remem",
            Some("review-dup"),
            "Existing",
            "Existing memory",
            "decision",
            None,
            None,
            "project",
            None,
        )?;
        let id = insert_pending_candidate(&mut conn, "review-dup", "Updated memory")?;

        approve_candidate(&mut conn, id)?.expect("candidate should approve");

        let memory_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
        let (content, owner_scope, owner_key): (String, String, String) = conn.query_row(
            "SELECT content, owner_scope, owner_key FROM memories
             WHERE topic_key = 'review-dup' AND status = 'active'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let old_status: String = conn.query_row(
            "SELECT status FROM memories WHERE id = ?1",
            params![old_id],
            |row| row.get(0),
        )?;
        assert_eq!(memory_count, 2);
        assert_eq!(content, "Updated memory");
        assert_eq!(old_status, "stale");
        assert_eq!(owner_scope, "repo");
        assert_eq!(owner_key, "/tmp/remem");
        Ok(())
    }

    #[test]
    fn review_approve_preserves_existing_project_memory_for_global_candidate() -> Result<()> {
        let mut conn = setup_conn();
        crate::memory::insert_memory_full(
            &conn,
            None,
            "/tmp/remem",
            Some("review-scope"),
            "Project",
            "Project memory",
            "decision",
            None,
            None,
            "project",
            None,
        )?;
        let id = insert_pending_candidate_with_scope(
            &mut conn,
            "review-scope",
            "Global memory",
            "global",
        )?;

        approve_candidate(&mut conn, id)?.expect("candidate should approve");

        let memory_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE topic_key = 'review-scope'",
            [],
            |row| row.get(0),
        )?;
        let project_content: String = conn.query_row(
            "SELECT content FROM memories
             WHERE topic_key = 'review-scope' AND scope = 'project'",
            [],
            |row| row.get(0),
        )?;
        let global_content: String = conn.query_row(
            "SELECT content FROM memories
             WHERE topic_key = 'review-scope' AND scope = 'global'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(memory_count, 2);
        assert_eq!(project_content, "Project memory");
        assert_eq!(global_content, "Global memory");
        Ok(())
    }
}
