use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::memory::lesson::is_lesson_candidate;
use crate::memory_candidate::{persist_summary_candidates, ParsedMemoryCandidate};

use super::format::{build_content, split_into_items, MIN_DECISION_LEN};
use super::slug::content_hash;

const MIN_LEARNED_LEN: usize = 30;
const MIN_PREFERENCE_LEN: usize = 10;
const SUMMARY_CANDIDATE_CONFIDENCE: f64 = 0.74;
const SUMMARY_CANDIDATE_RISK: &str = "medium";

pub fn promote_summary_to_memory_candidates(
    conn: &mut Connection,
    session_id: &str,
    project: &str,
    request: Option<&str>,
    decisions: Option<&str>,
    learned: Option<&str>,
    preferences: Option<&str>,
) -> Result<usize> {
    let candidates = summary_memory_candidates(request, decisions, learned, preferences);
    if candidates.is_empty() {
        return Ok(0);
    }

    let source = summary_candidate_source(conn, session_id, project)?;
    let summary = persist_summary_candidates(
        conn,
        session_id,
        source.project_id,
        project,
        &source.evidence_event_ids,
        &candidates,
    )?;

    if summary.candidates > 0 {
        crate::log::info(
            "promote",
            &format!(
                "created {} memory candidate(s) from summary project={}",
                summary.candidates, project
            ),
        );
    }
    Ok(summary.candidates)
}

fn summary_memory_candidates(
    request: Option<&str>,
    decisions: Option<&str>,
    learned: Option<&str>,
    preferences: Option<&str>,
) -> Vec<ParsedMemoryCandidate> {
    let request_text = request.unwrap_or("").trim();
    let mut candidates = Vec::new();

    if let Some(text) = decisions {
        append_standard_candidates(
            &mut candidates,
            request_text,
            text,
            MIN_DECISION_LEN,
            "decision",
            "decision",
        );
    }

    if let Some(text) = learned {
        append_learned_candidates(&mut candidates, request_text, text);
    }

    if let Some(text) = preferences {
        let text = text.trim();
        if text.len() >= MIN_PREFERENCE_LEN {
            candidates.push(ParsedMemoryCandidate {
                scope: "project".to_string(),
                memory_type: "preference".to_string(),
                topic_key: format!("preference-{}", content_hash(text)),
                text: text.to_string(),
                confidence: SUMMARY_CANDIDATE_CONFIDENCE,
                risk_class: SUMMARY_CANDIDATE_RISK.to_string(),
            });
        }
    }

    candidates
}

fn append_learned_candidates(
    candidates: &mut Vec<ParsedMemoryCandidate>,
    request_text: &str,
    text: &str,
) {
    let text = text.trim();
    if text.len() < MIN_LEARNED_LEN {
        return;
    }

    let split_items = split_into_items(text);
    let items: Vec<&str> = if split_items.len() > 1 {
        split_items
            .iter()
            .map(String::as_str)
            .filter(|item| item.len() >= MIN_LEARNED_LEN)
            .collect()
    } else {
        vec![text]
    };

    for item in items {
        let content = build_content(item, request_text);
        if is_lesson_candidate(item) {
            candidates.push(ParsedMemoryCandidate {
                scope: "project".to_string(),
                memory_type: "lesson".to_string(),
                topic_key: format!("lesson-{}", content_hash(item)),
                text: content,
                confidence: lesson_confidence(item),
                risk_class: SUMMARY_CANDIDATE_RISK.to_string(),
            });
        } else {
            candidates.push(ParsedMemoryCandidate {
                scope: "project".to_string(),
                memory_type: "discovery".to_string(),
                topic_key: format!("discovery-{}", content_hash(item)),
                text: content,
                confidence: SUMMARY_CANDIDATE_CONFIDENCE,
                risk_class: SUMMARY_CANDIDATE_RISK.to_string(),
            });
        }
    }
}

fn lesson_confidence(item: &str) -> f64 {
    let normalized = item.to_lowercase();
    if normalized.contains("root cause") || normalized.contains("lesson:") {
        0.85
    } else if normalized.contains("never ") || normalized.contains("do not ") {
        0.8
    } else {
        0.7
    }
}

fn append_standard_candidates(
    candidates: &mut Vec<ParsedMemoryCandidate>,
    request_text: &str,
    text: &str,
    min_len: usize,
    topic_prefix: &str,
    memory_type: &str,
) {
    let text = text.trim();
    if text.len() < min_len {
        return;
    }

    let items = split_into_items(text);
    if items.len() > 1 {
        for item in items.iter().filter(|item| item.len() >= min_len) {
            candidates.push(ParsedMemoryCandidate {
                scope: "project".to_string(),
                memory_type: memory_type.to_string(),
                topic_key: format!("{}-{}", topic_prefix, content_hash(item)),
                text: build_content(item, request_text),
                confidence: SUMMARY_CANDIDATE_CONFIDENCE,
                risk_class: SUMMARY_CANDIDATE_RISK.to_string(),
            });
        }
    } else {
        candidates.push(ParsedMemoryCandidate {
            scope: "project".to_string(),
            memory_type: memory_type.to_string(),
            topic_key: format!("{}-{}", topic_prefix, content_hash(text)),
            text: build_content(text, request_text),
            confidence: SUMMARY_CANDIDATE_CONFIDENCE,
            risk_class: SUMMARY_CANDIDATE_RISK.to_string(),
        });
    }
}

#[derive(Debug)]
struct SummaryCandidateSource {
    project_id: i64,
    evidence_event_ids: Vec<i64>,
}

fn summary_candidate_source(
    conn: &Connection,
    session_id: &str,
    project: &str,
) -> Result<SummaryCandidateSource> {
    let row = match latest_captured_event(conn, session_id, project, Some("session_stop"))? {
        Some(row) => Some(row),
        None => latest_captured_event(conn, session_id, project, None)?,
    };
    let Some((project_id, event_id)) = row else {
        bail!(
            "summary candidate extraction missing captured evidence session={} project={}",
            session_id,
            project
        );
    };
    Ok(SummaryCandidateSource {
        project_id,
        evidence_event_ids: vec![event_id],
    })
}

fn latest_captured_event(
    conn: &Connection,
    session_id: &str,
    project: &str,
    event_type: Option<&str>,
) -> Result<Option<(i64, i64)>> {
    match event_type {
        Some(event_type) => conn
            .query_row(
                "SELECT e.project_id, e.id
                 FROM captured_events e
                 JOIN projects p ON p.id = e.project_id
                 WHERE e.session_id = ?1
                   AND p.project_path = ?2
                   AND e.event_type = ?3
                 ORDER BY e.id DESC
                 LIMIT 1",
                params![session_id, project, event_type],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(Into::into),
        None => conn
            .query_row(
                "SELECT e.project_id, e.id
                 FROM captured_events e
                 JOIN projects p ON p.id = e.project_id
                 WHERE e.session_id = ?1
                   AND p.project_path = ?2
                 ORDER BY e.id DESC
                 LIMIT 1",
                params![session_id, project],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(Into::into),
    }
}
