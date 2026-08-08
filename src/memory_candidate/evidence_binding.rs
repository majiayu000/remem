use std::collections::BTreeSet;

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use crate::memory::poisoning::{derive_source_trust_class, SourceTrustClass};

use super::support::supporting_source_groups;
use super::ParsedMemoryCandidate;

#[derive(Debug)]
pub(super) struct SummaryCandidateEvidence {
    pub(super) event_ids: Option<Vec<i64>>,
    pub(super) source_texts: Vec<String>,
}

pub(super) struct SummaryEvidenceResolver {
    events: Vec<CapturedSourceEvent>,
}

#[derive(Debug)]
struct CapturedSourceEvent {
    id: i64,
    text: String,
    trust: SourceTrustClass,
}

impl SummaryEvidenceResolver {
    pub(super) fn load(
        conn: &Connection,
        evidence_event_ids: &[i64],
        source_kind: &str,
    ) -> Result<Self> {
        Ok(Self {
            events: load_captured_source_events(conn, evidence_event_ids, source_kind)?,
        })
    }

    pub(super) fn resolve(&self, candidate: &ParsedMemoryCandidate) -> SummaryCandidateEvidence {
        let Some(selected_ids) = bind_candidate_to_events(candidate, &self.events) else {
            return SummaryCandidateEvidence {
                event_ids: None,
                source_texts: self.events.iter().map(|event| event.text.clone()).collect(),
            };
        };
        let source_texts = self
            .events
            .iter()
            .filter(|event| selected_ids.contains(&event.id))
            .map(|event| event.text.clone())
            .collect();
        SummaryCandidateEvidence {
            event_ids: Some(selected_ids.into_iter().collect()),
            source_texts,
        }
    }
}

fn bind_candidate_to_events(
    candidate: &ParsedMemoryCandidate,
    events: &[CapturedSourceEvent],
) -> Option<BTreeSet<i64>> {
    let source_texts = events
        .iter()
        .map(|event| event.text.as_str())
        .collect::<Vec<_>>();
    let groups = supporting_source_groups(&candidate.text, &source_texts)?;

    let mut selected_ids = BTreeSet::new();
    for group in groups {
        let mut best: Option<&CapturedSourceEvent> = None;
        for source_index in group {
            let Some(event) = events.get(source_index) else {
                continue;
            };
            let should_replace = best.is_none_or(|current| {
                event.trust > current.trust
                    || (event.trust == current.trust && event.id < current.id)
            });
            if should_replace {
                best = Some(event);
            }
        }
        selected_ids.insert(best?.id);
    }
    Some(selected_ids)
}

fn load_captured_source_events(
    conn: &Connection,
    evidence_event_ids: &[i64],
    source_kind: &str,
) -> Result<Vec<CapturedSourceEvent>> {
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
    let unique_ids = evidence_event_ids.iter().copied().collect::<BTreeSet<_>>();
    for event_id in unique_ids {
        let text = stmt
            .query_row(params![event_id], |row| row.get::<_, String>(0))
            .optional()?
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty());
        let Some(text) = text else {
            continue;
        };
        events.push(CapturedSourceEvent {
            id: event_id,
            text,
            trust: derive_source_trust_class(conn, &[event_id], source_kind)?,
        });
    }
    Ok(events)
}
