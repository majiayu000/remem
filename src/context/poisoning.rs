use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use super::types::LoadedContext;
use crate::memory::poisoning::{scan_instruction_pattern, InstructionPatternMatch};
use crate::memory::Memory;

#[derive(Debug, Clone, Default)]
pub(super) struct PoisoningDrops {
    pub(super) memories: Vec<Memory>,
    pub(super) lessons: Vec<Memory>,
    pub(super) summaries: Vec<super::types::SessionSummaryBrief>,
    pub(super) workstreams: Vec<crate::workstream::WorkStream>,
}

#[derive(Debug, Clone)]
struct MemoryPoisoningState {
    acknowledged_pattern_id: Option<String>,
    acknowledged_pattern_version: Option<i64>,
    source_trust_class: String,
    source_project: Option<String>,
}

pub(super) fn drop_unacknowledged_poisoned_context(
    conn: &Connection,
    loaded: &mut LoadedContext,
) -> PoisoningDrops {
    let mut drops = std::mem::take(&mut loaded.poisoning_drops);

    let memories = std::mem::take(&mut loaded.memories);
    for memory in memories {
        let keep = should_inject_memory(conn, &memory, "memory").unwrap_or_else(|error| {
            crate::log::error(
                "context-poisoning",
                &format!(
                    "dropping memory {} after poisoning check failed: {error}",
                    memory.id
                ),
            );
            false
        });
        if keep {
            loaded.memories.push(memory);
        } else {
            drops.memories.push(memory);
        }
    }

    let lessons = std::mem::take(&mut loaded.lessons);
    for lesson in lessons {
        let keep = should_inject_memory(conn, &lesson.memory, "lessons").unwrap_or_else(|error| {
            crate::log::error(
                "context-poisoning",
                &format!(
                    "dropping lesson memory {} after poisoning check failed: {error}",
                    lesson.memory.id
                ),
            );
            false
        });
        if keep {
            loaded.lessons.push(lesson);
        } else {
            drops.lessons.push(lesson.memory);
        }
    }

    let summaries = std::mem::take(&mut loaded.summaries);
    for summary in summaries {
        if crate::db::summary_poisoning::summary_injectable(
            conn,
            summary.id,
            &[
                ("request", Some(summary.request.as_str())),
                ("completed", summary.completed.as_deref()),
            ],
            "context_sessions_section",
        ) {
            loaded.summaries.push(summary);
        } else {
            drops.summaries.push(summary);
        }
    }

    let workstreams = std::mem::take(&mut loaded.workstreams);
    let (safe_workstreams, poisoned_workstreams) = partition_workstreams(workstreams);
    loaded.workstreams = safe_workstreams;
    drops.workstreams.extend(poisoned_workstreams);

    drops
}

pub(super) fn partition_workstreams(
    workstreams: Vec<crate::workstream::WorkStream>,
) -> (
    Vec<crate::workstream::WorkStream>,
    Vec<crate::workstream::WorkStream>,
) {
    let mut safe = Vec::new();
    let mut poisoned = Vec::new();
    for workstream in workstreams {
        match scan_instruction_pattern(&workstream_haystack(&workstream)) {
            None => safe.push(workstream),
            Some(pattern_match) => {
                crate::log::error(
                    "context-poisoning",
                    &format!(
                        "dropping poisoned workstream id={} pattern={}@v{}",
                        workstream.id, pattern_match.pattern_id, pattern_match.pattern_set_version
                    ),
                );
                poisoned.push(workstream);
            }
        }
    }
    (safe, poisoned)
}

fn workstream_haystack(workstream: &crate::workstream::WorkStream) -> String {
    [
        Some(workstream.title.as_str()),
        workstream.description.as_deref(),
        workstream.progress.as_deref(),
        workstream.next_action.as_deref(),
        workstream.blockers.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("\n")
}

pub(super) fn should_inject_memory(
    conn: &Connection,
    memory: &Memory,
    channel: &str,
) -> Result<bool> {
    should_inject_memory_payload(conn, memory, &memory_haystack(memory), channel)
}

pub(super) fn should_inject_memory_payload(
    conn: &Connection,
    memory: &Memory,
    payload: &str,
    channel: &str,
) -> Result<bool> {
    let Some(pattern_match) = scan_instruction_pattern(payload) else {
        return Ok(true);
    };
    let state = load_memory_poisoning_state(conn, memory.id)?;
    if acknowledges_pattern(&state, pattern_match) {
        return Ok(true);
    }

    crate::log::error(
        "context-poisoning",
        &format!(
            "dropping unacknowledged poisoned {channel} memory id={} pattern={}@v{}",
            memory.id, pattern_match.pattern_id, pattern_match.pattern_set_version
        ),
    );
    if let Err(error) = record_injection_drop(conn, memory, &state, pattern_match) {
        crate::log::error(
            "context-poisoning",
            &format!(
                "failed to record poisoned memory drop for memory {}: {error}",
                memory.id
            ),
        );
    }
    Ok(false)
}

fn memory_haystack(memory: &Memory) -> String {
    format!("{}\n{}\n{}", memory.memory_type, memory.title, memory.text)
}

fn load_memory_poisoning_state(conn: &Connection, memory_id: i64) -> Result<MemoryPoisoningState> {
    let state = conn
        .query_row(
            "SELECT acknowledged_pattern_id, acknowledged_pattern_version,
                    source_trust_class, source_project
             FROM memories WHERE id = ?1",
            params![memory_id],
            |row| {
                Ok(MemoryPoisoningState {
                    acknowledged_pattern_id: row.get(0)?,
                    acknowledged_pattern_version: row.get(1)?,
                    source_trust_class: row.get(2)?,
                    source_project: row.get(3)?,
                })
            },
        )
        .optional()?
        .unwrap_or_else(|| MemoryPoisoningState {
            acknowledged_pattern_id: None,
            acknowledged_pattern_version: None,
            source_trust_class: "external_content".to_string(),
            source_project: None,
        });
    Ok(state)
}

fn acknowledges_pattern(
    state: &MemoryPoisoningState,
    pattern_match: InstructionPatternMatch,
) -> bool {
    state.acknowledged_pattern_id.as_deref() == Some(pattern_match.pattern_id)
        && state.acknowledged_pattern_version == Some(pattern_match.pattern_set_version)
}

fn record_injection_drop(
    conn: &Connection,
    memory: &Memory,
    state: &MemoryPoisoningState,
    pattern_match: InstructionPatternMatch,
) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_poisoning_injection_drops
         (memory_id, pattern_id, pattern_version, source_trust_class, source_project,
          title, created_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            memory.id,
            pattern_match.pattern_id,
            pattern_match.pattern_set_version,
            state.source_trust_class.as_str(),
            state.source_project.as_deref(),
            memory.title.as_str(),
            chrono::Utc::now().timestamp(),
        ],
    )?;
    Ok(())
}
