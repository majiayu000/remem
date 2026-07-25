use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection};

use super::IdentityRecord;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RekeyReport {
    pub merged: usize,
    pub rekeyed: usize,
}

pub(crate) fn rekey_legacy_rows(
    conn: &Connection,
    identity: &IdentityRecord,
) -> Result<RekeyReport> {
    if identity.status == "conflict" {
        return Ok(RekeyReport::default());
    }
    let rows: Vec<LegacyRawRow> = {
        let mut statement = conn.prepare(
            "SELECT id, role, content, content_hash, source, created_at_epoch,
                    event_time_source
             FROM raw_messages
             WHERE source_root = ?1 AND session_id IN (?2, ?3)
               AND project IN (?4, ?5) AND transcript_identity_id IS NULL
               AND source = 'transcript'
             ORDER BY id",
        )?;
        let rows = statement
            .query_map(
                params![
                    identity.source_root,
                    identity.fallback_session_id,
                    identity.canonical_session_id,
                    identity.project,
                    identity.legacy_project
                ],
                |row| {
                    Ok(LegacyRawRow {
                        id: row.get(0)?,
                        role: row.get(1)?,
                        content: row.get(2)?,
                        content_hash: row.get(3)?,
                        source: row.get(4)?,
                        created_at_epoch: row.get(5)?,
                        event_time_source: row.get(6)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let mut mutations = Vec::with_capacity(rows.len());
    let mut assigned_targets = BTreeSet::new();
    let mut unmatched_targets = BTreeMap::new();
    for row in rows {
        let targets = load_collision_targets(conn, identity, &row)?;
        if targets
            .iter()
            .any(|target| !stable_collision_matches(&row, target))
        {
            return Err(crate::memory::raw_occurrence::RawIdentityConflict {
                reason: format!(
                    "legacy row {} has {} canonical collision(s) with a stable-field mismatch",
                    row.id,
                    targets.len()
                ),
            }
            .into());
        }
        let target_id = if targets.is_empty() {
            let key = (row.role.clone(), row.content_hash.clone());
            if let Some(target) = unmatched_targets.get(&key) {
                if !stable_collision_matches(&row, target) {
                    return Err(crate::memory::raw_occurrence::RawIdentityConflict {
                        reason: format!("legacy alias {} has a stable-field mismatch", row.id),
                    }
                    .into());
                }
                Some(target.id)
            } else {
                unmatched_targets.insert(key, CollisionTarget::from(&row));
                None
            }
        } else {
            targets
                .iter()
                .find(|target| !assigned_targets.contains(&target.id))
                .or_else(|| targets.first())
                .map(|target| target.id)
        };
        if let Some(target_id) = target_id {
            assigned_targets.insert(target_id);
        }
        mutations.push((row.id, target_id));
    }

    let mut report = RekeyReport::default();
    for (old_id, target_id) in &mutations {
        if let Some(target_id) = *target_id {
            rewrite_evidence_references(conn, *old_id, target_id)?;
            assert_no_evidence_reference(conn, *old_id)?;
            conn.execute("DELETE FROM raw_messages WHERE id = ?1", [old_id])?;
            report.merged += 1;
        }
    }
    for (old_id, target_id) in mutations {
        if target_id.is_none() {
            conn.execute(
                "UPDATE raw_messages SET project = ?2, session_id = ?3 WHERE id = ?1",
                params![old_id, identity.project, identity.canonical_session_id],
            )?;
            report.rekeyed += 1;
        }
    }
    Ok(report)
}

#[derive(Debug)]
struct LegacyRawRow {
    id: i64,
    role: String,
    content: String,
    content_hash: String,
    source: String,
    created_at_epoch: i64,
    event_time_source: String,
}

#[derive(Debug)]
struct CollisionTarget {
    id: i64,
    content: String,
    source: String,
    created_at_epoch: i64,
    event_time_source: String,
}

impl From<&LegacyRawRow> for CollisionTarget {
    fn from(row: &LegacyRawRow) -> Self {
        Self {
            id: row.id,
            content: row.content.clone(),
            source: row.source.clone(),
            created_at_epoch: row.created_at_epoch,
            event_time_source: row.event_time_source.clone(),
        }
    }
}

fn load_collision_targets(
    conn: &Connection,
    identity: &IdentityRecord,
    row: &LegacyRawRow,
) -> Result<Vec<CollisionTarget>> {
    let mut statement = conn.prepare(
        "SELECT id, content, source, created_at_epoch, event_time_source
         FROM raw_messages
         WHERE source_root = ?1 AND project = ?2 AND session_id = ?3
           AND role = ?4 AND content_hash = ?5
           AND transcript_identity_id = ?6 AND id != ?7
         ORDER BY transcript_record_ordinal, id",
    )?;
    let targets = statement
        .query_map(
            params![
                identity.source_root,
                identity.project,
                identity.canonical_session_id,
                row.role,
                row.content_hash,
                identity.id,
                row.id
            ],
            |row| {
                Ok(CollisionTarget {
                    id: row.get(0)?,
                    content: row.get(1)?,
                    source: row.get(2)?,
                    created_at_epoch: row.get(3)?,
                    event_time_source: row.get(4)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(targets)
}

fn stable_collision_matches(old: &LegacyRawRow, target: &CollisionTarget) -> bool {
    let legacy_timestamp_upgrade = old.source == "transcript"
        && old.event_time_source == "legacy_unknown"
        && target.event_time_source == "transcript_event";
    let provenance_matches =
        old.event_time_source == target.event_time_source || legacy_timestamp_upgrade;
    old.content == target.content
        && old.source == target.source
        && (old.created_at_epoch == target.created_at_epoch || legacy_timestamp_upgrade)
        && provenance_matches
}

fn rewrite_evidence_references(conn: &Connection, old_id: i64, new_id: i64) -> Result<()> {
    let rows: Vec<(i64, String)> = {
        let mut statement =
            conn.prepare("SELECT id, evidence_raw_message_ids FROM memory_lesson_feed_events")?;
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for (event_id, json) in rows {
        let mut ids = serde_json::from_str::<Vec<i64>>(&json)
            .with_context(|| format!("parse evidence ids for feed event {event_id}"))?;
        if !ids.contains(&old_id) {
            continue;
        }
        for id in &mut ids {
            if *id == old_id {
                *id = new_id;
            }
        }
        ids.sort_unstable();
        ids.dedup();
        conn.execute(
            "UPDATE memory_lesson_feed_events
             SET evidence_raw_message_ids = ?2 WHERE id = ?1",
            params![event_id, serde_json::to_string(&ids)?],
        )?;
    }
    rewrite_lesson_source_evidence(conn, old_id, new_id)?;
    Ok(())
}

fn rewrite_lesson_source_evidence(conn: &Connection, old_id: i64, new_id: i64) -> Result<()> {
    let old_token = format!("raw_message:{old_id}:");
    let new_token = format!("raw_message:{new_id}:");
    let rows: Vec<(i64, String)> = {
        let mut statement = conn.prepare(
            "SELECT memory_id, source_evidence
             FROM memory_lessons
             WHERE source_evidence IS NOT NULL AND INSTR(source_evidence, ?1) > 0
             ORDER BY memory_id",
        )?;
        let rows = statement
            .query_map([&old_token], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for (memory_id, source_evidence) in rows {
        let rewritten =
            deduplicate_raw_message_references(&source_evidence.replace(&old_token, &new_token));
        conn.execute(
            "UPDATE memory_lessons SET source_evidence = ?2 WHERE memory_id = ?1",
            params![memory_id, rewritten],
        )?;
    }
    Ok(())
}

fn deduplicate_raw_message_references(source_evidence: &str) -> String {
    const PREFIX: &str = "raw_message:";
    let mut output = String::with_capacity(source_evidence.len());
    let mut seen = BTreeSet::new();
    let mut cursor = 0;
    while let Some(relative_start) = source_evidence[cursor..].find(PREFIX) {
        let start = cursor + relative_start;
        output.push_str(&source_evidence[cursor..start]);
        let id_start = start + PREFIX.len();
        let Some(relative_end) = source_evidence[id_start..].find(':') else {
            output.push_str(&source_evidence[start..]);
            return output;
        };
        let id_end = id_start + relative_end;
        let id = &source_evidence[id_start..id_end];
        if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
            output.push_str(PREFIX);
            cursor = id_start;
            continue;
        }
        let token_end = id_end + 1;
        if seen.insert(id.to_string()) {
            output.push_str(&source_evidence[start..token_end]);
        }
        cursor = token_end;
    }
    output.push_str(&source_evidence[cursor..]);
    output
}

fn assert_no_evidence_reference(conn: &Connection, raw_message_id: i64) -> Result<()> {
    let json_reference_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_lesson_feed_events
         WHERE EXISTS (
             SELECT 1 FROM json_each(evidence_raw_message_ids)
             WHERE CAST(value AS INTEGER) = ?1
         )",
        [raw_message_id],
        |row| row.get(0),
    )?;
    let token = format!("raw_message:{raw_message_id}:");
    let text_reference_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_lessons
         WHERE source_evidence IS NOT NULL AND INSTR(source_evidence, ?1) > 0",
        [token],
        |row| row.get(0),
    )?;
    if json_reference_count > 0 || text_reference_count > 0 {
        bail!(
            "raw row {raw_message_id} still has {json_reference_count} JSON and \
             {text_reference_count} text evidence reference(s)"
        );
    }
    Ok(())
}
