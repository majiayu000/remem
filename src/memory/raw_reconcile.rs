use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::path::Path;
use std::time::SystemTime;

use anyhow::{bail, Context, Result};
use rusqlite::Connection;
use serde::Serialize;

use crate::ingest::session_identity::IdentityRecord;
use crate::ingest::sessions::{discover_transcript_files, ScanRoot};

use super::raw_archive::{ROLE_ASSISTANT, ROLE_USER};
use super::raw_transcript::{classify_transcript_line, TranscriptRecordClass};

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub(crate) struct ReconcileSide {
    pub sessions: usize,
    pub messages: usize,
    pub user_messages: usize,
    pub assistant_messages: usize,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub(crate) struct ReconcileComparison {
    pub exact_sessions: usize,
    pub message_mismatch_sessions: usize,
    pub transcript_only_sessions: usize,
    pub transcript_only_messages: usize,
    pub archive_only_sessions: usize,
    pub archive_only_messages: usize,
    pub transcript_excess_messages: usize,
    pub transcript_excess_user_messages: usize,
    pub transcript_excess_assistant_messages: usize,
    pub archive_excess_messages: usize,
    pub archive_excess_user_messages: usize,
    pub archive_excess_assistant_messages: usize,
    pub identity_conflicts: usize,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub(crate) struct ReconcileExclusions {
    pub meta_user: usize,
    pub xml_control_user: usize,
    pub empty_text: usize,
    pub unsupported_record: usize,
    pub missing_event_time: usize,
    pub archive_ingest_fallback_event_time: usize,
    pub archive_unknown_event_time: usize,
    pub malformed_record: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct RawReconcileReport {
    pub policy_version: i64,
    pub since_epoch: i64,
    pub until_epoch: i64,
    pub transcript: ReconcileSide,
    pub archive: ReconcileSide,
    pub comparison: ReconcileComparison,
    pub intentional_exclusions: ReconcileExclusions,
    pub parity: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct MessageKey {
    ordinal: i64,
    role: String,
    content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum SessionKey {
    TranscriptIdentity(i64),
    Legacy {
        source_root: String,
        project: String,
        session_id: String,
    },
}

type SessionMessages = BTreeMap<SessionKey, BTreeMap<MessageKey, usize>>;

struct CapturedTranscript {
    identity: IdentityRecord,
    file: File,
    byte_limit: u64,
}

pub(crate) fn reconcile_raw_archive(
    conn: &Connection,
    roots: &[ScanRoot],
    since_epoch: i64,
    until_epoch: i64,
) -> Result<RawReconcileReport> {
    if since_epoch > until_epoch {
        bail!("invalid reconciliation window: --since must be <= --until");
    }

    let captured = capture_candidates(conn, roots, since_epoch, until_epoch)?;
    let source_roots = roots.iter().map(|root| root.label.clone()).collect();
    reconcile_captured(conn, captured, &source_roots, since_epoch, until_epoch)
}

fn reconcile_captured(
    conn: &Connection,
    captured: Vec<CapturedTranscript>,
    source_roots: &BTreeSet<String>,
    since_epoch: i64,
    until_epoch: i64,
) -> Result<RawReconcileReport> {
    let mut transcript_sessions = SessionMessages::new();
    let mut exclusions = ReconcileExclusions::default();
    let mut conflict_groups = BTreeSet::new();

    for candidate in captured {
        let identity = candidate.identity;
        let session_key = SessionKey::TranscriptIdentity(identity.id);
        let messages = transcript_sessions.entry(session_key.clone()).or_default();
        let mut ordinal = 0_i64;
        let mut participates_in_window = false;
        super::raw_transcript::stream_captured_transcript(
            candidate.file,
            candidate.byte_limit,
            |line, _| {
                let current_ordinal = ordinal;
                ordinal += 1;
                match classify_transcript_line(line, Some((since_epoch, until_epoch))) {
                    TranscriptRecordClass::Conversation(message) => {
                        participates_in_window = true;
                        insert_transcript_key(messages, current_ordinal, message);
                    }
                    TranscriptRecordClass::MetaUser(message) => {
                        participates_in_window = true;
                        exclusions.meta_user += 1;
                        insert_transcript_key(messages, current_ordinal, message);
                    }
                    TranscriptRecordClass::XmlControlUser(message) => {
                        participates_in_window = true;
                        exclusions.xml_control_user += 1;
                        insert_transcript_key(messages, current_ordinal, message);
                    }
                    TranscriptRecordClass::MissingEventTime(_) => {
                        participates_in_window = true;
                        exclusions.missing_event_time += 1;
                    }
                    TranscriptRecordClass::EmptyText => {
                        participates_in_window = true;
                        exclusions.empty_text += 1;
                    }
                    TranscriptRecordClass::UnsupportedRecord => {
                        participates_in_window = true;
                        exclusions.unsupported_record += 1;
                    }
                    TranscriptRecordClass::MalformedRecord => {
                        participates_in_window = true;
                        exclusions.malformed_record += 1;
                    }
                    TranscriptRecordClass::OutsideWindow => {}
                }
            },
        )
        .context("read captured transcript boundary")?;
        if messages.is_empty() {
            transcript_sessions.remove(&session_key);
        }
        if identity.status == "conflict" && participates_in_window {
            conflict_groups.insert((
                identity.source_root.clone(),
                identity.fallback_session_id.clone(),
            ));
        }
    }

    let archive_sessions = load_archive_messages(
        conn,
        source_roots,
        since_epoch,
        until_epoch,
        &mut exclusions,
    )?;
    let transcript = summarize_side(&transcript_sessions);
    let archive = summarize_side(&archive_sessions);
    let mut comparison = compare_sessions(&transcript_sessions, &archive_sessions);
    comparison.identity_conflicts = conflict_groups.len();
    let parity = comparison.message_mismatch_sessions == 0
        && comparison.transcript_only_sessions == 0
        && comparison.archive_only_sessions == 0
        && comparison.transcript_excess_messages == 0
        && comparison.archive_excess_messages == 0
        && comparison.identity_conflicts == 0
        && exclusions.malformed_record == 0
        && exclusions.missing_event_time == 0
        && exclusions.archive_ingest_fallback_event_time == 0
        && exclusions.archive_unknown_event_time == 0;

    Ok(RawReconcileReport {
        policy_version: 1,
        since_epoch,
        until_epoch,
        transcript,
        archive,
        comparison,
        intentional_exclusions: exclusions,
        parity,
    })
}

fn capture_candidates(
    conn: &Connection,
    roots: &[ScanRoot],
    since_epoch: i64,
    until_epoch: i64,
) -> Result<Vec<CapturedTranscript>> {
    let mut captured = Vec::new();
    let mut captured_identity_ids = BTreeSet::new();
    let mut stale_count = 0_usize;
    for root in roots {
        let (files, failures) = discover_transcript_files(root);
        if !failures.is_empty() {
            bail!(
                "transcript discovery failed for {} required entry/entries; run `remem ingest-sessions`",
                failures.len()
            );
        }
        let discovered: BTreeSet<String> = files
            .iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect();
        stale_count += extra_ledger_entry_count(conn, root, &discovered)?;
        for path in files {
            let transcript_path = path.to_string_lossy().to_string();
            let Some(identity) =
                crate::ingest::session_identity::load_by_path(conn, &root.label, &transcript_path)?
            else {
                stale_count += 1;
                continue;
            };
            let file = File::open(&path).context("open transcript snapshot")?;
            let metadata = file.metadata().context("stat transcript snapshot")?;
            let byte_limit = metadata.len();
            let mtime_ns = modified_ns(&metadata);
            let is_conflict = identity.status == "conflict";
            if identity.transcript_path != transcript_path
                || identity.observed_size_bytes != i64::try_from(byte_limit).unwrap_or(i64::MAX)
                || identity.observed_mtime_ns != mtime_ns
            {
                stale_count += 1;
                continue;
            }
            let intersects = identity
                .first_event_epoch
                .zip(identity.last_event_epoch)
                .is_some_and(|(first, last)| first <= until_epoch && last >= since_epoch);
            let outside_window = !intersects && identity.missing_event_time_count == 0;
            if !is_conflict && outside_window && identity.event_index_status == "since_indexed" {
                continue;
            }
            if !is_conflict
                && (identity.contract_version != 1
                    || !crate::ingest::sessions::cursor_matches_identity(
                        conn,
                        root,
                        &path,
                        identity.observed_mtime_ns,
                        identity.observed_size_bytes,
                    )?)
            {
                stale_count += 1;
                continue;
            }
            if !is_conflict && outside_window {
                continue;
            }
            if !captured_identity_ids.insert(identity.id) {
                continue;
            }
            captured.push(CapturedTranscript {
                identity,
                file,
                byte_limit,
            });
        }
    }
    if stale_count > 0 {
        bail!(
            "transcript identity index has {stale_count} stale or missing entries; run `remem ingest-sessions`"
        );
    }
    Ok(captured)
}

fn modified_ns(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .unwrap_or(SystemTime::UNIX_EPOCH)
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn extra_ledger_entry_count(
    conn: &Connection,
    root: &ScanRoot,
    discovered: &BTreeSet<String>,
) -> Result<usize> {
    let mut statement = conn.prepare(
        "SELECT transcript_path FROM raw_session_identities
         WHERE source_root = ?1 ORDER BY transcript_path",
    )?;
    let paths = statement
        .query_map([&root.label], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(paths
        .into_iter()
        .filter(|path| Path::new(path).starts_with(&root.path))
        .filter(|path| !discovered.contains(path))
        .count())
}

fn insert_transcript_key(
    messages: &mut BTreeMap<MessageKey, usize>,
    ordinal: i64,
    message: super::raw_transcript::ParsedTranscriptMessage,
) {
    let key = MessageKey {
        ordinal,
        role: message.role.to_string(),
        content_hash: crate::db::content_identity_hash(message.text.trim().as_bytes()),
    };
    *messages.entry(key).or_default() += 1;
}

fn load_archive_messages(
    conn: &Connection,
    source_roots: &BTreeSet<String>,
    since_epoch: i64,
    until_epoch: i64,
    exclusions: &mut ReconcileExclusions,
) -> Result<SessionMessages> {
    let mut statement = conn.prepare(
        "SELECT id, transcript_identity_id, transcript_record_ordinal,
                source_root, project, session_id, role, content_hash,
                event_time_source
         FROM raw_messages
         WHERE (
             (event_time_source = 'transcript_event'
              AND created_at_epoch >= ?1
              AND created_at_epoch <= ?2)
             OR event_time_source != 'transcript_event'
           )
         ORDER BY transcript_identity_id, source_root, project, session_id,
                  transcript_record_ordinal, id",
    )?;
    let rows = statement.query_map(rusqlite::params![since_epoch, until_epoch], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Option<i64>>(1)?,
            row.get::<_, Option<i64>>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, String>(8)?,
        ))
    })?;
    let mut sessions = SessionMessages::new();
    for row in rows {
        let (
            raw_id,
            identity_id,
            ordinal,
            source_root,
            project,
            session_id,
            role,
            content_hash,
            event_time_source,
        ) = row?;
        if !source_roots.is_empty() && !source_roots.contains(&source_root) {
            continue;
        }
        match event_time_source.as_str() {
            "transcript_event" => {
                let session_key = identity_id.map_or_else(
                    || SessionKey::Legacy {
                        source_root,
                        project,
                        session_id,
                    },
                    SessionKey::TranscriptIdentity,
                );
                let key = MessageKey {
                    ordinal: ordinal.unwrap_or(raw_id),
                    role,
                    content_hash,
                };
                *sessions
                    .entry(session_key)
                    .or_default()
                    .entry(key)
                    .or_default() += 1;
            }
            "ingest_fallback" => exclusions.archive_ingest_fallback_event_time += 1,
            _ => exclusions.archive_unknown_event_time += 1,
        }
    }
    Ok(sessions)
}

fn summarize_side(sessions: &SessionMessages) -> ReconcileSide {
    let mut summary = ReconcileSide {
        sessions: sessions.len(),
        ..ReconcileSide::default()
    };
    for messages in sessions.values() {
        for (key, count) in messages {
            summary.messages += count;
            if key.role == ROLE_USER {
                summary.user_messages += count;
            } else if key.role == ROLE_ASSISTANT {
                summary.assistant_messages += count;
            }
        }
    }
    summary
}

fn compare_sessions(
    transcript: &SessionMessages,
    archive: &SessionMessages,
) -> ReconcileComparison {
    let mut comparison = ReconcileComparison::default();
    let session_ids: BTreeSet<SessionKey> =
        transcript.keys().chain(archive.keys()).cloned().collect();
    for session_id in session_ids {
        match (transcript.get(&session_id), archive.get(&session_id)) {
            (Some(left), Some(right)) if left == right => comparison.exact_sessions += 1,
            (Some(left), Some(right)) => {
                comparison.message_mismatch_sessions += 1;
                add_excess(left, right, true, &mut comparison);
                add_excess(right, left, false, &mut comparison);
            }
            (Some(left), None) => {
                comparison.transcript_only_sessions += 1;
                comparison.transcript_only_messages += count_messages(left);
            }
            (None, Some(right)) => {
                comparison.archive_only_sessions += 1;
                comparison.archive_only_messages += count_messages(right);
            }
            (None, None) => {}
        }
    }
    comparison
}

fn add_excess(
    left: &BTreeMap<MessageKey, usize>,
    right: &BTreeMap<MessageKey, usize>,
    transcript_side: bool,
    comparison: &mut ReconcileComparison,
) {
    for (key, left_count) in left {
        let excess = left_count.saturating_sub(right.get(key).copied().unwrap_or(0));
        if excess == 0 {
            continue;
        }
        if transcript_side {
            comparison.transcript_excess_messages += excess;
            if key.role == ROLE_USER {
                comparison.transcript_excess_user_messages += excess;
            } else if key.role == ROLE_ASSISTANT {
                comparison.transcript_excess_assistant_messages += excess;
            }
        } else {
            comparison.archive_excess_messages += excess;
            if key.role == ROLE_USER {
                comparison.archive_excess_user_messages += excess;
            } else if key.role == ROLE_ASSISTANT {
                comparison.archive_excess_assistant_messages += excess;
            }
        }
    }
}

fn count_messages(messages: &BTreeMap<MessageKey, usize>) -> usize {
    messages.values().sum()
}

pub(crate) fn render_reconcile_human(report: &RawReconcileReport) -> String {
    format!(
        "Raw reconciliation policy={} window={}..{} parity={}\n\
         transcript: sessions={} messages={} user={} assistant={}\n\
         archive: sessions={} messages={} user={} assistant={}\n\
         comparison: exact_sessions={} mismatch_sessions={} transcript_only_sessions={} transcript_only_messages={} archive_only_sessions={} archive_only_messages={}\n\
         excess: transcript={} transcript_user={} transcript_assistant={} archive={} archive_user={} archive_assistant={} conflicts={}\n\
         exclusions: meta={} xml={} empty={} unsupported={} missing_time={} fallback_time={} unknown_time={} malformed={}\n",
        report.policy_version,
        report.since_epoch,
        report.until_epoch,
        report.parity,
        report.transcript.sessions,
        report.transcript.messages,
        report.transcript.user_messages,
        report.transcript.assistant_messages,
        report.archive.sessions,
        report.archive.messages,
        report.archive.user_messages,
        report.archive.assistant_messages,
        report.comparison.exact_sessions,
        report.comparison.message_mismatch_sessions,
        report.comparison.transcript_only_sessions,
        report.comparison.transcript_only_messages,
        report.comparison.archive_only_sessions,
        report.comparison.archive_only_messages,
        report.comparison.transcript_excess_messages,
        report.comparison.transcript_excess_user_messages,
        report.comparison.transcript_excess_assistant_messages,
        report.comparison.archive_excess_messages,
        report.comparison.archive_excess_user_messages,
        report.comparison.archive_excess_assistant_messages,
        report.comparison.identity_conflicts,
        report.intentional_exclusions.meta_user,
        report.intentional_exclusions.xml_control_user,
        report.intentional_exclusions.empty_text,
        report.intentional_exclusions.unsupported_record,
        report.intentional_exclusions.missing_event_time,
        report
            .intentional_exclusions
            .archive_ingest_fallback_event_time,
        report.intentional_exclusions.archive_unknown_event_time,
        report.intentional_exclusions.malformed_record,
    )
}

#[cfg(test)]
#[path = "raw_reconcile/tests.rs"]
mod tests;
