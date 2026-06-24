use std::collections::{BTreeSet, HashMap, HashSet};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::db;

use super::ParsedUserContextCandidate;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ExternalSourceLabel {
    File,
    Readme,
    Website,
}

#[derive(Debug, Clone)]
pub(super) struct SourceEvent {
    pub(super) id: i64,
    pub(super) event_type: String,
    pub(super) role: Option<String>,
    pub(super) tool_name: Option<String>,
    pub(super) content: String,
    pub(super) token_estimate: i64,
    pub(super) created_at_epoch: i64,
}

#[derive(Debug, Clone)]
pub(super) struct SessionSummarySource {
    pub(super) id: i64,
    pub(super) summary_text: Option<String>,
    pub(super) request: Option<String>,
    pub(super) completed: Option<String>,
    pub(super) decisions: Option<String>,
    pub(super) learned: Option<String>,
    pub(super) next_steps: Option<String>,
    pub(super) preferences: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct CandidateSourceBatch {
    pub(super) from_event_id: i64,
    pub(super) to_event_id: i64,
    pub(super) event_ids: Vec<i64>,
    pub(super) events: Vec<SourceEvent>,
    pub(super) summary: Option<SessionSummarySource>,
    user_event_ids: HashSet<i64>,
    event_index: HashMap<i64, SourceEvent>,
}

impl CandidateSourceBatch {
    pub(super) fn has_event(&self, id: i64) -> bool {
        self.event_index.contains_key(&id)
    }

    pub(super) fn event_is_user_authored(&self, id: i64) -> bool {
        self.user_event_ids.contains(&id)
    }

    pub(super) fn events_for_candidate(
        &self,
        candidate: &ParsedUserContextCandidate,
    ) -> Vec<&SourceEvent> {
        candidate
            .source_event_ids
            .iter()
            .filter_map(|id| self.event_index.get(id))
            .collect()
    }
}

pub(super) fn load_source_batch(
    conn: &Connection,
    task: &db::ExtractionTask,
) -> Result<Option<CandidateSourceBatch>> {
    let Some(session_row_id) = task.session_row_id else {
        return Ok(None);
    };
    let Some(high_watermark) = task.high_watermark_event_id else {
        return Ok(None);
    };
    let cursor = task.cursor_event_id.unwrap_or(0);
    if high_watermark <= cursor {
        return Ok(None);
    }

    let mut stmt = conn.prepare(
        "SELECT e.id, e.event_type, e.role, e.tool_name,
                COALESCE(
                    CASE
                        WHEN b.content_encoding = 'plain' THEN CAST(b.content_bytes AS TEXT)
                        ELSE NULL
                    END,
                    e.content_text,
                    ''
                ) AS content,
                e.token_estimate, e.created_at_epoch
         FROM captured_events e
         LEFT JOIN event_blobs b ON b.id = e.content_blob_id
         WHERE e.host_id = ?1
           AND e.project_id = ?2
           AND e.session_row_id = ?3
           AND e.id > ?4
           AND e.id <= ?5
         ORDER BY e.id ASC",
    )?;
    let events = stmt
        .query_map(
            params![
                task.host_id,
                task.project_id,
                session_row_id,
                cursor,
                high_watermark
            ],
            |row| {
                Ok(SourceEvent {
                    id: row.get(0)?,
                    event_type: row.get(1)?,
                    role: row.get(2)?,
                    tool_name: row.get(3)?,
                    content: row.get(4)?,
                    token_estimate: row.get(5)?,
                    created_at_epoch: row.get(6)?,
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    if events.is_empty() {
        return Ok(None);
    }
    let event_ids = events.iter().map(|event| event.id).collect::<Vec<_>>();
    let from_event_id = event_ids.iter().copied().min().unwrap_or(0);
    let to_event_id = event_ids.iter().copied().max().unwrap_or(0);
    let summary = load_session_summary(conn, session_row_id, from_event_id, to_event_id)?;
    let user_event_ids = events
        .iter()
        .filter(|event| is_user_authored_event(event))
        .map(|event| event.id)
        .collect::<HashSet<_>>();
    let event_index = events
        .iter()
        .map(|event| (event.id, event.clone()))
        .collect::<HashMap<_, _>>();
    Ok(Some(CandidateSourceBatch {
        from_event_id,
        to_event_id,
        event_ids,
        events,
        summary,
        user_event_ids,
        event_index,
    }))
}

pub(super) fn source_refs_json(
    batch: &CandidateSourceBatch,
    candidate: &ParsedUserContextCandidate,
) -> Result<String> {
    let mut refs = BTreeSet::new();
    for id in &candidate.source_event_ids {
        refs.insert(serde_json::json!({"kind": "captured_event", "id": id}).to_string());
    }
    if let Some(summary) = &batch.summary {
        refs.insert(serde_json::json!({"kind": "session_summary", "id": summary.id}).to_string());
    }
    let values = refs
        .into_iter()
        .map(|value| serde_json::from_str::<serde_json::Value>(&value))
        .collect::<Result<Vec<_>, _>>()?;
    serde_json::to_string(&values).context("serialize user-context candidate source refs")
}

pub(super) fn source_preview(
    batch: &CandidateSourceBatch,
    candidate: &ParsedUserContextCandidate,
) -> Option<String> {
    let parts = batch
        .events_for_candidate(candidate)
        .into_iter()
        .filter_map(|event| evidence_preview_for_event(&event.content, &candidate.claim_text))
        .collect::<Vec<_>>();
    let preview = parts.join("\n");
    (!preview.is_empty()).then(|| crate::db::truncate_str(&preview, 500).to_string())
}

fn evidence_preview_for_event(content: &str, claim_text: &str) -> Option<String> {
    let claim_tokens = preview_match_tokens(claim_text);
    if claim_tokens.is_empty() {
        return None;
    }
    let segments = evidence_segments(content);
    let evidence = segments
        .iter()
        .filter(|segment| segment_matches_claim(segment, &claim_tokens))
        .cloned()
        .collect::<Vec<_>>();
    if evidence.is_empty() {
        return None;
    }
    let mut preview = Vec::new();
    let source_labels = preview_external_source_labels(claim_text, &evidence);
    if !source_labels.is_empty() {
        preview.extend(
            segments
                .iter()
                .filter(|segment| {
                    crate::user_context::non_retention::has_external_source_approval(segment)
                        && source_labels_overlap(&source_labels, &external_source_labels(segment))
                        && !evidence.iter().any(|evidence| evidence == *segment)
                })
                .cloned(),
        );
    }
    preview.extend(evidence);
    Some(preview.join(" "))
}

pub(super) fn evidence_segments(content: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut start = 0;
    for (index, ch) in content.char_indices() {
        if matches!(ch, '.' | '?' | '!' | '\n' | ';') {
            let end = index + ch.len_utf8();
            let segment = content[start..end].trim();
            if !segment.is_empty() {
                segments.push(segment.to_string());
            }
            start = end;
        }
    }
    let tail = content[start..].trim();
    if !tail.is_empty() {
        segments.push(tail.to_string());
    }
    segments
}

fn preview_external_source_labels(
    claim_text: &str,
    evidence: &[String],
) -> BTreeSet<ExternalSourceLabel> {
    let mut labels = external_source_labels(claim_text);
    for segment in evidence {
        labels.extend(external_source_labels(segment));
    }
    labels
}

fn source_labels_overlap(
    left: &BTreeSet<ExternalSourceLabel>,
    right: &BTreeSet<ExternalSourceLabel>,
) -> bool {
    left.iter().any(|label| right.contains(label))
}

fn external_source_labels(text: &str) -> BTreeSet<ExternalSourceLabel> {
    let lower = text.to_ascii_lowercase();
    let tokens = lower
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect::<BTreeSet<_>>();
    let mut labels = BTreeSet::new();
    if tokens.contains("readme") {
        labels.insert(ExternalSourceLabel::Readme);
        labels.insert(ExternalSourceLabel::File);
    }
    if tokens.contains("file") || tokens.contains("files") {
        labels.insert(ExternalSourceLabel::File);
    }
    if lower.contains("website") || lower.contains("web page") || lower.contains("browser page") {
        labels.insert(ExternalSourceLabel::Website);
    }
    labels
}

fn segment_matches_claim(segment: &str, claim_tokens: &[String]) -> bool {
    let segment_tokens = preview_match_tokens(segment);
    if segment_tokens.is_empty() {
        return false;
    }
    let matches = claim_tokens
        .iter()
        .filter(|token| {
            segment_tokens
                .iter()
                .any(|segment_token| segment_token == *token)
        })
        .count();
    matches >= claim_tokens.len().min(2)
}

fn preview_match_tokens(text: &str) -> Vec<String> {
    let mut tokens = text
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| normalize_preview_token(&token.to_ascii_lowercase()))
        .filter(|token| !is_preview_stopword(token))
        .collect::<Vec<_>>();
    tokens.sort();
    tokens.dedup();
    tokens
}

fn normalize_preview_token(token: &str) -> String {
    if let Some(stem) = token.strip_suffix("ies") {
        return format!("{stem}y");
    }
    if token.len() > 4 && token.ends_with('s') && !token.ends_with("ss") && !token.ends_with("us") {
        return token[..token.len() - 1].to_string();
    }
    token.to_string()
}

fn is_preview_stopword(token: &str) -> bool {
    matches!(
        token,
        "a" | "an"
            | "and"
            | "as"
            | "for"
            | "from"
            | "i"
            | "in"
            | "is"
            | "me"
            | "my"
            | "of"
            | "on"
            | "our"
            | "s"
            | "the"
            | "their"
            | "to"
            | "user"
            | "we"
    )
}

fn load_session_summary(
    conn: &Connection,
    session_row_id: i64,
    from_event_id: i64,
    to_event_id: i64,
) -> Result<Option<SessionSummarySource>> {
    conn.query_row(
        "SELECT id, summary_text, request, completed, decisions, learned, next_steps, preferences
         FROM session_summaries
         WHERE session_row_id = ?1
           AND covered_from_event_id = ?2
           AND covered_to_event_id = ?3
         ORDER BY created_at_epoch DESC, id DESC
         LIMIT 1",
        params![session_row_id, from_event_id, to_event_id],
        |row| {
            Ok(SessionSummarySource {
                id: row.get(0)?,
                summary_text: row.get(1)?,
                request: row.get(2)?,
                completed: row.get(3)?,
                decisions: row.get(4)?,
                learned: row.get(5)?,
                next_steps: row.get(6)?,
                preferences: row.get(7)?,
            })
        },
    )
    .optional()
    .context("load session summary for user-context candidate extraction")
}

fn is_user_authored_event(event: &SourceEvent) -> bool {
    event.role.as_deref() == Some("user") || event.event_type == "user_prompt_submit"
}
