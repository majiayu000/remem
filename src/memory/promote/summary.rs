use std::collections::BTreeSet;

use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::memory::lesson::is_lesson_candidate;
#[cfg(test)]
use crate::memory_candidate::persist_summary_candidates_with_gate_mode;
use crate::memory_candidate::{persist_summary_candidates, ParsedMemoryCandidate};
use crate::runtime_config::SummaryGateMode;

use super::format::{build_content, split_into_items, MIN_DECISION_LEN};
use super::slug::content_hash;

const MIN_LEARNED_LEN: usize = 30;
const MIN_PREFERENCE_LEN: usize = 10;
const MIN_SEMANTIC_TOPIC_TERMS: usize = 2;
const MAX_SEMANTIC_TOPIC_TERMS: usize = 10;
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
    promote_summary_to_memory_candidates_inner(
        conn,
        session_id,
        project,
        request,
        decisions,
        learned,
        preferences,
        None,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn promote_summary_to_memory_candidates_with_evidence(
    conn: &mut Connection,
    session_id: &str,
    project_id: i64,
    project: &str,
    evidence_event_ids: &[i64],
    source_texts: &[String],
    request: Option<&str>,
    decisions: Option<&str>,
    learned: Option<&str>,
    preferences: Option<&str>,
) -> Result<usize> {
    if evidence_event_ids.is_empty() {
        bail!("summary candidate extraction requires captured evidence");
    }
    promote_summary_to_memory_candidates_inner(
        conn,
        session_id,
        project,
        request,
        decisions,
        learned,
        preferences,
        None,
        Some(SummaryCandidateSource {
            project_id,
            evidence_event_ids: evidence_event_ids.to_vec(),
            source_texts: source_texts.to_vec(),
        }),
    )
}

#[cfg(test)]
pub(super) fn promote_summary_to_memory_candidates_with_gate_mode(
    conn: &mut Connection,
    session_id: &str,
    project: &str,
    request: Option<&str>,
    decisions: Option<&str>,
    learned: Option<&str>,
    preferences: Option<&str>,
    summary_gate_mode: SummaryGateMode,
) -> Result<usize> {
    promote_summary_to_memory_candidates_inner(
        conn,
        session_id,
        project,
        request,
        decisions,
        learned,
        preferences,
        Some(summary_gate_mode),
        None,
    )
}

fn promote_summary_to_memory_candidates_inner(
    conn: &mut Connection,
    session_id: &str,
    project: &str,
    request: Option<&str>,
    decisions: Option<&str>,
    learned: Option<&str>,
    preferences: Option<&str>,
    summary_gate_mode: Option<SummaryGateMode>,
    source_override: Option<SummaryCandidateSource>,
) -> Result<usize> {
    let candidates = summary_memory_candidates(request, decisions, learned, preferences);
    if candidates.is_empty() {
        return Ok(0);
    }
    let candidates = crate::memory::claims::filter_summary_candidates_by_claims(
        conn,
        session_id,
        project,
        &candidates,
    );
    if candidates.is_empty() {
        return Ok(0);
    }

    let source = match source_override {
        Some(source) => source,
        None => summary_candidate_source(conn, session_id, project)?,
    };
    let summary = match summary_gate_mode {
        #[cfg(test)]
        Some(mode) => persist_summary_candidates_with_gate_mode(
            conn,
            session_id,
            source.project_id,
            project,
            &source.evidence_event_ids,
            &source.source_texts,
            &candidates,
            mode,
        )?,
        #[cfg(not(test))]
        Some(_) => unreachable!("summary gate mode override is test-only"),
        None => persist_summary_candidates(
            conn,
            session_id,
            source.project_id,
            project,
            &source.evidence_event_ids,
            &source.source_texts,
            &candidates,
        )?,
    };

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
                topic_key: summary_topic_key("preference", "preference", "Preference", text, text),
                title_override: None,
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
                topic_key: summary_topic_key("lesson", "lesson", "Lesson", &content, item),
                title_override: None,
                text: content,
                confidence: lesson_confidence(item),
                risk_class: SUMMARY_CANDIDATE_RISK.to_string(),
            });
        } else {
            candidates.push(ParsedMemoryCandidate {
                scope: "project".to_string(),
                memory_type: "discovery".to_string(),
                topic_key: summary_topic_key("discovery", "discovery", "Discovery", &content, item),
                title_override: None,
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
            let candidate_text = build_content(item, request_text);
            candidates.push(ParsedMemoryCandidate {
                scope: "project".to_string(),
                memory_type: memory_type.to_string(),
                topic_key: summary_topic_key(
                    memory_type,
                    topic_prefix,
                    memory_type,
                    &candidate_text,
                    item,
                ),
                title_override: None,
                text: candidate_text,
                confidence: SUMMARY_CANDIDATE_CONFIDENCE,
                risk_class: SUMMARY_CANDIDATE_RISK.to_string(),
            });
        }
    } else {
        let candidate_text = build_content(text, request_text);
        candidates.push(ParsedMemoryCandidate {
            scope: "project".to_string(),
            memory_type: memory_type.to_string(),
            topic_key: summary_topic_key(
                memory_type,
                topic_prefix,
                memory_type,
                &candidate_text,
                text,
            ),
            title_override: None,
            text: candidate_text,
            confidence: SUMMARY_CANDIDATE_CONFIDENCE,
            risk_class: SUMMARY_CANDIDATE_RISK.to_string(),
        });
    }
}

fn summary_topic_key(
    memory_type: &str,
    topic_prefix: &str,
    title: &str,
    _candidate_text: &str,
    hash_seed: &str,
) -> String {
    let fallback = format!("{}-{}", topic_prefix, content_hash(hash_seed));
    if let Some(decision) =
        crate::memory::state_key::derive_state_key(memory_type, Some(&fallback), title, hash_seed)
    {
        return decision.state_key;
    }
    summary_semantic_topic_key(topic_prefix, hash_seed).unwrap_or(fallback)
}

fn summary_semantic_topic_key(topic_prefix: &str, text: &str) -> Option<String> {
    let mut terms = summary_semantic_terms(text);
    if terms.len() < MIN_SEMANTIC_TOPIC_TERMS {
        return None;
    }
    terms.truncate(MAX_SEMANTIC_TOPIC_TERMS);
    let key = format!("{}-{}", topic_prefix, terms.join("-"));
    let slug = crate::memory::promote::slugify_for_topic(&key, 120);
    if slug.is_empty() {
        None
    } else {
        Some(slug)
    }
}

fn summary_semantic_terms(text: &str) -> Vec<String> {
    let mut terms = BTreeSet::new();
    for raw in text.split(|ch: char| !ch.is_ascii_alphanumeric()) {
        let Some(term) = normalize_summary_semantic_term(raw) else {
            continue;
        };
        if !is_summary_semantic_stopword(&term) {
            terms.insert(term);
        }
    }
    terms.into_iter().collect()
}

fn normalize_summary_semantic_term(raw: &str) -> Option<String> {
    let mut term = raw.trim().to_ascii_lowercase();
    if term.is_empty() {
        return None;
    }
    term = match term.as_str() {
        "tokenization" | "tokenized" | "tokenize" | "tokenizing" => "tokenizer".to_string(),
        "summaries" => "summary".to_string(),
        "memories" => "memory".to_string(),
        "claims" => "claim".to_string(),
        "candidates" => "candidate".to_string(),
        "decisions" => "decision".to_string(),
        "observations" => "observation".to_string(),
        "indexes" | "indexed" | "indexing" => "index".to_string(),
        "tests" | "tested" | "testing" => "test".to_string(),
        "changes" | "changed" | "changing" => "change".to_string(),
        "updates" | "updated" | "updating" => "update".to_string(),
        "embeddings" => "embedding".to_string(),
        "vectors" => "vector".to_string(),
        _ => term,
    };
    if term.len() > 4 && term.ends_with('s') && !term.ends_with("ss") {
        term.pop();
    }
    let has_digit = term.chars().any(|ch| ch.is_ascii_digit());
    if term.len() < 3 && !has_digit {
        return None;
    }
    Some(term)
}

fn is_summary_semantic_stopword(term: &str) -> bool {
    matches!(
        term,
        "about"
            | "active"
            | "add"
            | "after"
            | "again"
            | "against"
            | "always"
            | "and"
            | "as"
            | "because"
            | "before"
            | "choose"
            | "current"
            | "default"
            | "disable"
            | "disabled"
            | "enable"
            | "enabled"
            | "for"
            | "from"
            | "in"
            | "keep"
            | "later"
            | "must"
            | "now"
            | "of"
            | "only"
            | "or"
            | "prefer"
            | "record"
            | "remove"
            | "removed"
            | "run"
            | "should"
            | "stop"
            | "support"
            | "switch"
            | "text"
            | "the"
            | "this"
            | "through"
            | "to"
            | "use"
            | "using"
            | "with"
            | "without"
    )
}

#[derive(Debug)]
struct SummaryCandidateSource {
    project_id: i64,
    evidence_event_ids: Vec<i64>,
    source_texts: Vec<String>,
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
    let source_texts = load_summary_source_texts(conn, &[event_id])?;
    Ok(SummaryCandidateSource {
        project_id,
        evidence_event_ids: vec![event_id],
        source_texts,
    })
}

fn load_summary_source_texts(conn: &Connection, evidence_event_ids: &[i64]) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(
                    CASE
                        WHEN b.content_encoding = 'plain' THEN CAST(b.content_bytes AS TEXT)
                        ELSE NULL
                    END,
                    e.content_text,
                    ''
                ) AS content
         FROM captured_events e
         LEFT JOIN event_blobs b ON b.id = e.content_blob_id
         WHERE e.id = ?1",
    )?;
    let mut texts = Vec::new();
    for event_id in evidence_event_ids {
        if let Some(text) = stmt
            .query_row(params![event_id], |row| row.get::<_, String>(0))
            .optional()?
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
        {
            texts.push(text);
        }
    }
    Ok(texts)
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
