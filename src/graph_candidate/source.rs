use std::collections::BTreeSet;

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::db;
use crate::graph_candidate::ParsedGraphCandidate;
use crate::memory::format::{xml_escape_attr, xml_escape_text};

#[derive(Debug, Clone)]
struct GraphSourceObservation {
    id: i64,
    observation_type: String,
    text: String,
    files_read: Vec<String>,
    files_modified: Vec<String>,
    evidence_event_ids: Vec<i64>,
    confidence: f64,
}

#[derive(Debug, Clone)]
struct GraphSourceEvent {
    id: i64,
    text: String,
}

#[derive(Debug, Clone)]
struct GraphSourceMemory {
    id: i64,
    memory_type: String,
    topic_key: Option<String>,
    title: String,
    content: String,
    evidence_event_ids: Vec<i64>,
}

pub(super) struct GraphObservationBatch {
    pub(super) from_event_id: i64,
    pub(super) to_event_id: i64,
    pub(super) evidence_event_ids: Vec<i64>,
    source_events: Vec<GraphSourceEvent>,
    observations: Vec<GraphSourceObservation>,
    source_memories: Vec<GraphSourceMemory>,
}

pub(super) fn load_graph_observation_batch(
    conn: &Connection,
    task: &db::ExtractionTask,
) -> Result<Option<GraphObservationBatch>> {
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
        "SELECT id,
                COALESCE(observation_type, type, 'discovery') AS observation_type,
                COALESCE(text, narrative, title, '') AS text,
                files_read,
                files_modified,
                evidence_event_ids,
                COALESCE(confidence, 0.5) AS confidence
         FROM observations
         WHERE session_row_id = ?1
           AND evidence_event_ids IS NOT NULL
           AND text IS NOT NULL
         ORDER BY id ASC",
    )?;
    let rows = stmt
        .query_map(params![session_row_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, f64>(6)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut observations = Vec::new();
    let mut evidence_set = BTreeSet::new();
    for (
        id,
        observation_type,
        text,
        files_read_json,
        files_modified_json,
        evidence_json,
        confidence,
    ) in rows
    {
        let event_ids: Vec<i64> = serde_json::from_str(&evidence_json)
            .with_context(|| format!("observation {id} has malformed evidence_event_ids"))?;
        let in_range = event_ids
            .iter()
            .any(|event_id| *event_id > cursor && *event_id <= high_watermark);
        if !in_range {
            continue;
        }
        for event_id in &event_ids {
            evidence_set.insert(*event_id);
        }
        observations.push(GraphSourceObservation {
            id,
            observation_type,
            text,
            files_read: parse_observation_file_list(id, "files_read", files_read_json)?,
            files_modified: parse_observation_file_list(id, "files_modified", files_modified_json)?,
            evidence_event_ids: event_ids,
            confidence,
        });
    }

    if observations.is_empty() || evidence_set.is_empty() {
        return Ok(None);
    }
    let from_event_id = *evidence_set.iter().next().unwrap_or(&0);
    let to_event_id = *evidence_set.iter().next_back().unwrap_or(&0);
    let evidence_event_ids = evidence_set.into_iter().collect::<Vec<_>>();
    let source_events = load_graph_source_events(conn, &evidence_event_ids)?;
    let source_memories = load_graph_source_memories(conn, &task.project, &evidence_event_ids)?;
    Ok(Some(GraphObservationBatch {
        from_event_id,
        to_event_id,
        evidence_event_ids,
        source_events,
        observations,
        source_memories,
    }))
}

fn load_graph_source_events(
    conn: &Connection,
    evidence_event_ids: &[i64],
) -> Result<Vec<GraphSourceEvent>> {
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
    let mut events = Vec::new();
    for event_id in evidence_event_ids {
        if let Some(text) = stmt
            .query_row(params![event_id], |row| row.get(0))
            .optional()?
        {
            events.push(GraphSourceEvent {
                id: *event_id,
                text,
            });
        }
    }
    Ok(events)
}

fn load_graph_source_memories(
    conn: &Connection,
    source_project: &str,
    evidence_event_ids: &[i64],
) -> Result<Vec<GraphSourceMemory>> {
    let evidence_event_ids = evidence_event_ids.iter().copied().collect::<BTreeSet<_>>();
    let mut stmt = conn.prepare(
        "SELECT m.id,
                m.memory_type,
                m.topic_key,
                m.title,
                m.content,
                COALESCE(m.evidence_event_ids, c.evidence_event_ids) AS evidence_event_ids
         FROM memories m
         LEFT JOIN memory_candidates c ON c.id = m.source_candidate_id
         WHERE m.status = 'active'
           AND COALESCE(m.evidence_event_ids, c.evidence_event_ids) IS NOT NULL
           AND (
                (m.owner_scope = 'repo' AND m.owner_key = ?1)
                OR m.target_project = ?1
                OR (
                    m.owner_scope IS NULL
                    AND m.project = ?1
                    AND COALESCE(m.scope, 'project') != 'global'
                )
           )
         ORDER BY m.updated_at_epoch DESC, m.id DESC
         LIMIT 200",
    )?;
    let rows = stmt
        .query_map(params![source_project], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut memories = Vec::new();
    for (id, memory_type, topic_key, title, content, evidence_json) in rows {
        let memory_event_ids = serde_json::from_str::<Vec<i64>>(&evidence_json)
            .with_context(|| format!("memory {id} has malformed evidence_event_ids"))?;
        if memory_event_ids
            .iter()
            .any(|event_id| evidence_event_ids.contains(event_id))
        {
            memories.push(GraphSourceMemory {
                id,
                memory_type,
                topic_key,
                title,
                content,
                evidence_event_ids: memory_event_ids,
            });
        }
    }
    memories.sort_by_key(|memory| memory.id);
    Ok(memories)
}

fn parse_observation_file_list(
    observation_id: i64,
    field: &str,
    raw: Option<String>,
) -> Result<Vec<String>> {
    let Some(raw) = raw.as_deref().map(str::trim).filter(|raw| !raw.is_empty()) else {
        return Ok(Vec::new());
    };
    serde_json::from_str::<Vec<String>>(raw)
        .with_context(|| format!("observation {observation_id} has malformed {field}"))
}

pub(super) fn graph_candidate_blocked_by_memory_candidates(
    conn: &Connection,
    task: &db::ExtractionTask,
    batch: &GraphObservationBatch,
) -> Result<Option<String>> {
    let Some(session_row_id) = task.session_row_id else {
        return Ok(None);
    };
    let high_watermark = task.high_watermark_event_id.unwrap_or(batch.to_event_id);
    let incomplete_memory_task: Option<(i64, String)> = conn
        .query_row(
            "SELECT id, status
             FROM extraction_tasks
             WHERE host_id = ?1
               AND project_id = ?2
               AND session_row_id = ?3
               AND task_kind = 'memory_candidate'
               AND COALESCE(high_watermark_event_id, 0) >= ?4
               AND COALESCE(cursor_event_id, 0) < ?4
               AND status != 'done'
             ORDER BY id ASC
             LIMIT 1",
            params![
                task.host_id,
                task.project_id,
                session_row_id,
                high_watermark
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((task_id, status)) = incomplete_memory_task {
        return Ok(Some(format!(
            "memory_candidate task {task_id} is {status}; graph extraction waits for memory extraction completion"
        )));
    }

    let pending_memory_candidates = count_pending_memory_candidates_with_evidence_overlap(
        conn,
        task.project_id,
        &batch.evidence_event_ids,
    )?;
    if pending_memory_candidates > 0 {
        return Ok(Some(format!(
            "{pending_memory_candidates} memory candidate(s) for this evidence range are pending review"
        )));
    }

    Ok(None)
}

fn count_pending_memory_candidates_with_evidence_overlap(
    conn: &Connection,
    project_id: i64,
    evidence_event_ids: &[i64],
) -> Result<i64> {
    let evidence_event_ids = evidence_event_ids.iter().copied().collect::<BTreeSet<_>>();
    let mut stmt = conn.prepare(
        "SELECT id, evidence_event_ids
         FROM memory_candidates
         WHERE project_id = ?1
           AND review_status = 'pending_review'
           AND evidence_event_ids IS NOT NULL",
    )?;
    let rows = stmt
        .query_map(params![project_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut count = 0;
    for (candidate_id, candidate_evidence_json) in rows {
        let candidate_event_ids = serde_json::from_str::<Vec<i64>>(&candidate_evidence_json)
            .with_context(|| {
                format!("memory candidate {candidate_id} has malformed evidence_event_ids")
            })?;
        if candidate_event_ids
            .iter()
            .any(|event_id| evidence_event_ids.contains(event_id))
        {
            count += 1;
        }
    }
    Ok(count)
}

pub(super) fn graph_candidate_has_source_support(
    candidate: &ParsedGraphCandidate,
    batch: &GraphObservationBatch,
) -> bool {
    let candidate_evidence = candidate
        .evidence_event_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let observation_supported = batch
        .observations
        .iter()
        .filter(|observation| {
            observation
                .evidence_event_ids
                .iter()
                .any(|event_id| candidate_evidence.contains(event_id))
        })
        .any(|observation| refs_supported_by_observation(candidate, observation));
    let cited_events_supported = candidate.evidence_event_ids.iter().all(|event_id| {
        let event_text = batch
            .source_events
            .iter()
            .find(|event| event.id == *event_id)
            .map(|event| event.text.as_str());
        let event_text_supported =
            event_text.is_some_and(|text| refs_supported_by_event(candidate, *event_id, text));
        event_text_supported
            || batch.observations.iter().any(|observation| {
                observation.evidence_event_ids.contains(event_id)
                    && structured_file_evidence_supports_cited_event(
                        candidate,
                        *event_id,
                        event_text,
                        observation,
                    )
            })
    });
    observation_supported && cited_events_supported
}

fn refs_supported_by_observation(
    candidate: &ParsedGraphCandidate,
    observation: &GraphSourceObservation,
) -> bool {
    ref_supported_by_observation(&candidate.from_ref, observation)
        && ref_supported_by_observation(&candidate.to_ref, observation)
}

fn ref_supported_by_observation(reference: &str, observation: &GraphSourceObservation) -> bool {
    parse_episode_ref_id(reference)
        .is_some_and(|episode_id| observation.evidence_event_ids.contains(&episode_id))
        || ref_supported_by_text(reference, &observation_support_text(observation))
}

fn structured_file_evidence_supports_cited_event(
    candidate: &ParsedGraphCandidate,
    event_id: i64,
    event_text: Option<&str>,
    observation: &GraphSourceObservation,
) -> bool {
    candidate.edge_type == "touches_file"
        && event_text
            .is_some_and(|text| ref_supported_by_event(&candidate.from_ref, event_id, text))
        && ref_supported_by_structured_files(&candidate.to_ref, observation)
}

fn ref_supported_by_structured_files(
    reference: &str,
    observation: &GraphSourceObservation,
) -> bool {
    observation
        .files_read
        .iter()
        .chain(observation.files_modified.iter())
        .any(|file| ref_supported_by_text(reference, file))
}

fn observation_support_text(observation: &GraphSourceObservation) -> String {
    let mut text = observation.text.clone();
    for file in observation
        .files_read
        .iter()
        .chain(observation.files_modified.iter())
    {
        text.push('\n');
        text.push_str(file);
    }
    text
}

fn refs_supported_by_event(candidate: &ParsedGraphCandidate, event_id: i64, text: &str) -> bool {
    ref_supported_by_event(&candidate.from_ref, event_id, text)
        && ref_supported_by_event(&candidate.to_ref, event_id, text)
}

fn ref_supported_by_text(reference: &str, text: &str) -> bool {
    let haystack = text.to_ascii_lowercase();
    reference_support_needles(reference)
        .into_iter()
        .any(|needle| contains_support_needle(&haystack, &needle))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SupportBoundary {
    Word,
    Entity,
    Path,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SupportNeedle {
    text: String,
    boundary: SupportBoundary,
}

fn reference_support_needles(reference: &str) -> Vec<SupportNeedle> {
    let reference = reference.trim();
    if let Some(path) = reference.strip_prefix("file:") {
        return vec![
            SupportNeedle {
                text: path.trim_start_matches("./").to_string(),
                boundary: SupportBoundary::Path,
            },
            SupportNeedle {
                text: path.to_string(),
                boundary: SupportBoundary::Path,
            },
        ];
    }
    if let Some(entity) = reference.strip_prefix("entity:") {
        return vec![
            SupportNeedle {
                text: entity.to_string(),
                boundary: SupportBoundary::Entity,
            },
            SupportNeedle {
                text: entity.replace(['_', '-'], " "),
                boundary: SupportBoundary::Entity,
            },
        ];
    }
    if let Some(memory_id) = reference.strip_prefix("memory:") {
        return vec![
            SupportNeedle {
                text: reference.to_string(),
                boundary: SupportBoundary::Word,
            },
            SupportNeedle {
                text: format!("memory {memory_id}"),
                boundary: SupportBoundary::Word,
            },
        ];
    }
    if let Some(episode_id) = reference.strip_prefix("episode:") {
        return vec![
            SupportNeedle {
                text: reference.to_string(),
                boundary: SupportBoundary::Word,
            },
            SupportNeedle {
                text: format!("event {episode_id}"),
                boundary: SupportBoundary::Word,
            },
        ];
    }
    vec![SupportNeedle {
        text: reference.to_string(),
        boundary: SupportBoundary::Word,
    }]
}

fn contains_support_needle(haystack: &str, needle: &SupportNeedle) -> bool {
    let needle_text = needle.text.trim().to_ascii_lowercase();
    !needle_text.is_empty()
        && haystack.match_indices(&needle_text).any(|(start, _)| {
            let end = start + needle_text.len();
            support_boundary_before(haystack, start, needle.boundary)
                && support_boundary_after(haystack, end, needle.boundary)
        })
}

fn support_boundary_before(haystack: &str, start: usize, boundary: SupportBoundary) -> bool {
    haystack[..start]
        .chars()
        .next_back()
        .is_none_or(|ch| is_support_boundary(ch, boundary))
}

fn support_boundary_after(haystack: &str, end: usize, boundary: SupportBoundary) -> bool {
    let mut chars = haystack[end..].chars();
    let Some(ch) = chars.next() else {
        return true;
    };
    if boundary == SupportBoundary::Path
        && ch == '.'
        && chars
            .next()
            .is_some_and(|next| next.is_ascii_alphanumeric())
    {
        return false;
    }
    is_support_boundary(ch, boundary)
}

fn is_support_boundary(ch: char, boundary: SupportBoundary) -> bool {
    if ch.is_ascii_alphanumeric() || ch == '_' {
        return false;
    }
    match boundary {
        SupportBoundary::Word => true,
        SupportBoundary::Entity => !matches!(ch, '-' | '/' | '.'),
        SupportBoundary::Path => !matches!(ch, '-' | '/'),
    }
}

fn ref_supported_by_event(reference: &str, event_id: i64, text: &str) -> bool {
    parse_episode_ref_id(reference).is_some_and(|episode_id| episode_id == event_id)
        || ref_supported_by_text(reference, text)
}

fn parse_episode_ref_id(reference: &str) -> Option<i64> {
    reference
        .strip_prefix("episode:")
        .and_then(|raw| raw.trim().parse::<i64>().ok())
        .filter(|id| *id > 0)
}

pub(super) fn build_graph_candidate_prompt(
    task: &db::ExtractionTask,
    batch: &GraphObservationBatch,
) -> String {
    let mut prompt = format!(
        "Task: graph_candidate\nProject: {}\nHost: {}\nSession: {}\nCovered evidence events: {}..{}\n\n",
        task.project,
        task.host,
        task.session_id.as_deref().unwrap_or("<unknown>"),
        batch.from_event_id,
        batch.to_event_id
    );
    if !batch.source_memories.is_empty() {
        prompt.push_str("<memory_refs>\n");
        for memory in &batch.source_memories {
            let evidence = memory
                .evidence_event_ids
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(",");
            prompt.push_str(&format!(
                "<memory_ref ref=\"memory:{}\" type=\"{}\" topic_key=\"{}\" evidence_event_ids=\"{}\">\n",
                memory.id,
                xml_escape_attr(&memory.memory_type),
                xml_escape_attr(memory.topic_key.as_deref().unwrap_or("")),
                xml_escape_attr(&evidence)
            ));
            prompt.push_str("<title>");
            prompt.push_str(&xml_escape_text(crate::db::truncate_str(
                &memory.title,
                200,
            )));
            prompt.push_str("</title>\n<content>");
            prompt.push_str(&xml_escape_text(crate::db::truncate_str(
                &memory.content,
                500,
            )));
            prompt.push_str("</content>\n</memory_ref>\n");
        }
        prompt.push_str("</memory_refs>\n\n");
    }
    for observation in &batch.observations {
        let evidence = observation
            .evidence_event_ids
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        prompt.push_str(&format!(
            "<observation id=\"{}\" type=\"{}\" confidence=\"{}\" evidence_event_ids=\"{}\">\n",
            observation.id,
            xml_escape_attr(&observation.observation_type),
            observation.confidence,
            xml_escape_attr(&evidence)
        ));
        prompt.push_str(&xml_escape_text(&observation.text));
        if !observation.files_read.is_empty() {
            prompt.push_str("\n<files_read>\n");
            for file in &observation.files_read {
                prompt.push_str(&xml_escape_text(file));
                prompt.push('\n');
            }
            prompt.push_str("</files_read>\n");
        }
        if !observation.files_modified.is_empty() {
            prompt.push_str("\n<files_modified>\n");
            for file in &observation.files_modified {
                prompt.push_str(&xml_escape_text(file));
                prompt.push('\n');
            }
            prompt.push_str("</files_modified>\n");
        }
        prompt.push_str("\n</observation>\n\n");
    }
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn support_matching_uses_ref_boundaries() {
        assert!(!ref_supported_by_text(
            "memory:1",
            "Memory 10 mentions Worker."
        ));
        assert!(ref_supported_by_text(
            "memory:1",
            "Memory 1 mentions Worker."
        ));
        assert!(!ref_supported_by_text(
            "entity:Worker",
            "Changed src/worker.rs"
        ));
        assert!(ref_supported_by_text(
            "entity:Worker",
            "The Worker entity is mentioned."
        ));
        assert!(!ref_supported_by_text(
            "file:src/worker.rs",
            "Changed src/worker.rs.bak"
        ));
        assert!(ref_supported_by_text(
            "file:src/worker.rs",
            "Changed src/worker.rs."
        ));
    }
}
