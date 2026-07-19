use std::collections::BTreeMap;
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

mod rekey;
pub(crate) use rekey::{rekey_legacy_rows, RekeyReport};

const CONTEXT_PROBE_LINES: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IdentitySource {
    TranscriptMetadata,
    FilenameFallback,
}

impl IdentitySource {
    fn as_str(self) -> &'static str {
        match self {
            Self::TranscriptMetadata => "transcript_metadata",
            Self::FilenameFallback => "filename_fallback",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TranscriptPlan {
    pub source_root: String,
    pub path: PathBuf,
    pub transcript_path: String,
    pub fallback_session_id: String,
    pub canonical_session_id: String,
    pub project: String,
    pub legacy_project: String,
    pub identity_source: IdentitySource,
    pub branch: Option<String>,
    pub cwd: Option<String>,
    pub observed_mtime_ns: i64,
    pub observed_size_bytes: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct IdentityRecord {
    pub id: i64,
    pub source_root: String,
    pub transcript_path: String,
    pub fallback_session_id: String,
    pub canonical_session_id: String,
    pub project: String,
    pub legacy_project: String,
    pub status: String,
    pub contract_version: i64,
    pub observed_mtime_ns: i64,
    pub observed_size_bytes: i64,
    pub first_event_epoch: Option<i64>,
    pub last_event_epoch: Option<i64>,
    pub missing_event_time_count: i64,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct EventIndex {
    pub first_event_epoch: Option<i64>,
    pub last_event_epoch: Option<i64>,
    pub missing_event_time_count: i64,
}

pub(crate) fn probe(
    source_root: &str,
    scan_root: &Path,
    file: &Path,
    fallback_project: Option<&str>,
) -> Result<TranscriptPlan> {
    probe_inner(source_root, scan_root, file, fallback_project, None, None)
}

pub(crate) fn probe_bounded(
    source_root: &str,
    scan_root: &Path,
    file: &Path,
    fallback_project: Option<&str>,
    byte_limit: u64,
) -> Result<TranscriptPlan> {
    probe_inner(
        source_root,
        scan_root,
        file,
        fallback_project,
        None,
        Some(byte_limit),
    )
}

pub(crate) fn probe_with_project_cache(
    source_root: &str,
    scan_root: &Path,
    file: &Path,
    fallback_project: Option<&str>,
    project_cache: &mut BTreeMap<String, String>,
) -> Result<TranscriptPlan> {
    probe_inner(
        source_root,
        scan_root,
        file,
        fallback_project,
        Some(project_cache),
        None,
    )
}

fn probe_inner(
    source_root: &str,
    scan_root: &Path,
    file: &Path,
    fallback_project: Option<&str>,
    project_cache: Option<&mut BTreeMap<String, String>>,
    byte_limit: Option<u64>,
) -> Result<TranscriptPlan> {
    let metadata =
        std::fs::metadata(file).with_context(|| format!("stat transcript {}", file.display()))?;
    let observed_size_bytes = i64::try_from(metadata.len()).unwrap_or(i64::MAX);
    let observed_mtime_ns = metadata
        .modified()
        .unwrap_or(SystemTime::UNIX_EPOCH)
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX))
        .unwrap_or(0);
    let fallback_session_id = file
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_default();
    if fallback_session_id.is_empty() {
        bail!("transcript {} has no filename identity", file.display());
    }
    let context = probe_context(file, byte_limit)?;
    let (canonical_session_id, identity_source) = match context.session_id {
        Some(session_id) if !session_id.trim().is_empty() => {
            (session_id, IdentitySource::TranscriptMetadata)
        }
        _ => (
            fallback_session_id.clone(),
            IdentitySource::FilenameFallback,
        ),
    };
    let legacy_project = fallback_project_slug(scan_root, file, source_root);
    let project = match (context.cwd.as_deref(), project_cache) {
        (Some(cwd), Some(cache)) => cache
            .entry(cwd.to_string())
            .or_insert_with(|| crate::project_id::project_from_cwd(cwd))
            .clone(),
        (Some(cwd), None) => crate::project_id::project_from_cwd(cwd),
        (None, _) => fallback_project
            .map(str::to_string)
            .unwrap_or_else(|| legacy_project.clone()),
    };

    Ok(TranscriptPlan {
        source_root: source_root.to_string(),
        path: file.to_path_buf(),
        transcript_path: file.to_string_lossy().to_string(),
        fallback_session_id,
        canonical_session_id,
        project,
        legacy_project,
        identity_source,
        branch: context.branch,
        cwd: context.cwd,
        observed_mtime_ns,
        observed_size_bytes,
    })
}

pub(crate) fn upsert_claim(conn: &Connection, plan: &TranscriptPlan, now: i64) -> Result<i64> {
    let existing: Option<(i64, i64, i64)> = conn
        .query_row(
            "SELECT id, observed_mtime_ns, observed_size_bytes
             FROM raw_session_identities
             WHERE source_root = ?1 AND transcript_path = ?2",
            params![plan.source_root, plan.transcript_path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let tuple_changed = existing
        .map(|(_, mtime, size)| mtime != plan.observed_mtime_ns || size != plan.observed_size_bytes)
        .unwrap_or(true);
    conn.execute(
        "INSERT INTO raw_session_identities (
            source_root, transcript_path, fallback_session_id,
            canonical_session_id, project, legacy_project, status,
            contract_version, observed_mtime_ns, observed_size_bytes,
            first_seen_at_epoch, last_seen_at_epoch
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', 0, ?7, ?8, ?9, ?9)
         ON CONFLICT(source_root, transcript_path) DO UPDATE SET
            fallback_session_id = excluded.fallback_session_id,
            project = excluded.project,
            legacy_project = excluded.legacy_project,
            observed_mtime_ns = excluded.observed_mtime_ns,
            observed_size_bytes = excluded.observed_size_bytes,
            contract_version = CASE WHEN ?10 THEN 0 ELSE contract_version END,
            last_seen_at_epoch = excluded.last_seen_at_epoch",
        params![
            plan.source_root,
            plan.transcript_path,
            plan.fallback_session_id,
            plan.canonical_session_id,
            plan.project,
            plan.legacy_project,
            plan.observed_mtime_ns,
            plan.observed_size_bytes,
            now,
            tuple_changed
        ],
    )?;
    let identity_id: i64 = conn.query_row(
        "SELECT id FROM raw_session_identities
         WHERE source_root = ?1 AND transcript_path = ?2",
        params![plan.source_root, plan.transcript_path],
        |row| row.get(0),
    )?;
    conn.execute(
        "INSERT INTO raw_session_identity_claims (
            transcript_identity_id, claimed_session_id, identity_source,
            first_seen_at_epoch, last_seen_at_epoch
         ) VALUES (?1, ?2, ?3, ?4, ?4)
         ON CONFLICT(transcript_identity_id, claimed_session_id, identity_source)
         DO UPDATE SET last_seen_at_epoch = excluded.last_seen_at_epoch",
        params![
            identity_id,
            plan.canonical_session_id,
            plan.identity_source.as_str(),
            now
        ],
    )?;
    Ok(identity_id)
}

pub(crate) fn resolve_fallback_group(
    conn: &Connection,
    source_root: &str,
    fallback_session_id: &str,
) -> Result<()> {
    let inherited_conflict_reason = conn
        .query_row(
            "SELECT COALESCE(conflict_reason, 'inherited_group_conflict')
             FROM raw_session_identities
             WHERE source_root = ?1 AND fallback_session_id = ?2
               AND status = 'conflict'
             ORDER BY id
             LIMIT 1",
            params![source_root, fallback_session_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if let Some(reason) = inherited_conflict_reason {
        conn.execute(
            "UPDATE raw_session_identities
             SET status = 'conflict', conflict_reason = ?3
             WHERE source_root = ?1 AND fallback_session_id = ?2",
            params![source_root, fallback_session_id, reason],
        )?;
        return Ok(());
    }
    let metadata_claims: Vec<String> = {
        let mut statement = conn.prepare(
            "SELECT DISTINCT c.claimed_session_id
             FROM raw_session_identities i
             JOIN raw_session_identity_claims c ON c.transcript_identity_id = i.id
             WHERE i.source_root = ?1 AND i.fallback_session_id = ?2
               AND c.identity_source = 'transcript_metadata'
             ORDER BY c.claimed_session_id",
        )?;
        let rows = statement
            .query_map(params![source_root, fallback_session_id], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    if metadata_claims.len() > 1 {
        conn.execute(
            "UPDATE raw_session_identities
             SET status = 'conflict', conflict_reason = 'conflicting_metadata_claims'
             WHERE source_root = ?1 AND fallback_session_id = ?2",
            params![source_root, fallback_session_id],
        )?;
        return Ok(());
    }
    let canonical = metadata_claims
        .first()
        .map(String::as_str)
        .unwrap_or(fallback_session_id);
    conn.execute(
        "UPDATE raw_session_identities
         SET canonical_session_id = ?3
         WHERE source_root = ?1 AND fallback_session_id = ?2
           AND status = 'active'",
        params![source_root, fallback_session_id, canonical],
    )?;
    Ok(())
}

pub(crate) fn mark_fallback_group_conflict(
    conn: &Connection,
    source_root: &str,
    fallback_session_id: &str,
    reason: &str,
) -> Result<()> {
    conn.execute(
        "UPDATE raw_session_identities
         SET status = 'conflict', conflict_reason = ?3
         WHERE source_root = ?1 AND fallback_session_id = ?2",
        params![source_root, fallback_session_id, reason],
    )?;
    Ok(())
}

pub(crate) fn load(conn: &Connection, identity_id: i64) -> Result<IdentityRecord> {
    conn.query_row(
        "SELECT id, source_root, transcript_path, fallback_session_id,
                canonical_session_id, project, legacy_project, status,
                contract_version, observed_mtime_ns, observed_size_bytes,
                first_event_epoch, last_event_epoch, missing_event_time_count
         FROM raw_session_identities WHERE id = ?1",
        [identity_id],
        |row| {
            Ok(IdentityRecord {
                id: row.get(0)?,
                source_root: row.get(1)?,
                transcript_path: row.get(2)?,
                fallback_session_id: row.get(3)?,
                canonical_session_id: row.get(4)?,
                project: row.get(5)?,
                legacy_project: row.get(6)?,
                status: row.get(7)?,
                contract_version: row.get(8)?,
                observed_mtime_ns: row.get(9)?,
                observed_size_bytes: row.get(10)?,
                first_event_epoch: row.get(11)?,
                last_event_epoch: row.get(12)?,
                missing_event_time_count: row.get(13)?,
            })
        },
    )
    .map_err(Into::into)
}

pub(crate) fn load_by_path(
    conn: &Connection,
    source_root: &str,
    transcript_path: &str,
) -> Result<Option<IdentityRecord>> {
    let identity_id = conn
        .query_row(
            "SELECT id FROM raw_session_identities
             WHERE source_root = ?1 AND transcript_path = ?2",
            params![source_root, transcript_path],
            |row| row.get(0),
        )
        .optional()?;
    identity_id.map(|id| load(conn, id)).transpose()
}

pub(crate) fn index_events(path: &str, byte_limit: u64) -> Result<EventIndex> {
    let mut index = EventIndex::default();
    crate::memory::raw_transcript::stream_transcript_lines(path, Some(byte_limit), |line, _| {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            return;
        };
        if let Some(epoch) = crate::memory::raw_transcript::transcript_timestamp_epoch(&value) {
            index.first_event_epoch =
                Some(index.first_event_epoch.map_or(epoch, |old| old.min(epoch)));
            index.last_event_epoch =
                Some(index.last_event_epoch.map_or(epoch, |old| old.max(epoch)));
        } else if crate::memory::raw_transcript::parse_transcript_message(&value).is_some() {
            index.missing_event_time_count += 1;
        }
    })?;
    Ok(index)
}

pub(crate) fn record_unfinalized_event_index(
    conn: &Connection,
    identity_id: i64,
    index: EventIndex,
    now: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE raw_session_identities
         SET first_event_epoch = ?2, last_event_epoch = ?3,
             missing_event_time_count = ?4, last_seen_at_epoch = ?5
         WHERE id = ?1 AND status = 'active'",
        params![
            identity_id,
            index.first_event_epoch,
            index.last_event_epoch,
            index.missing_event_time_count,
            now
        ],
    )?;
    Ok(())
}

pub(crate) fn mark_complete(
    conn: &Connection,
    identity_id: i64,
    index: EventIndex,
    now: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE raw_session_identities
         SET contract_version = 1, first_event_epoch = ?2, last_event_epoch = ?3,
             missing_event_time_count = ?4, last_seen_at_epoch = ?5
         WHERE id = ?1 AND status = 'active'",
        params![
            identity_id,
            index.first_event_epoch,
            index.last_event_epoch,
            index.missing_event_time_count,
            now
        ],
    )?;
    Ok(())
}

#[derive(Default)]
struct TranscriptContext {
    session_id: Option<String>,
    cwd: Option<String>,
    branch: Option<String>,
}

fn probe_context(file: &Path, byte_limit: Option<u64>) -> Result<TranscriptContext> {
    let mut context = TranscriptContext::default();
    let handle = std::fs::File::open(file)
        .with_context(|| format!("open transcript probe {}", file.display()))?;
    let mut reader = std::io::BufReader::new(handle);
    let max_bytes = byte_limit.unwrap_or(u64::MAX);
    let mut consumed = 0_u64;
    for _ in 0..CONTEXT_PROBE_LINES {
        if consumed >= max_bytes {
            break;
        }
        let mut line = String::new();
        let read = reader
            .by_ref()
            .take(max_bytes - consumed)
            .read_line(&mut line)
            .with_context(|| format!("read transcript probe {}", file.display()))?;
        if read == 0 {
            if byte_limit.is_some() && consumed < max_bytes {
                bail!(
                    "transcript truncated before captured probe boundary: expected {max_bytes} bytes, read {consumed}"
                );
            }
            break;
        }
        consumed = consumed.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let payload = value.get("payload");
        context.session_id = context.session_id.or_else(|| {
            value
                .get("sessionId")
                .and_then(serde_json::Value::as_str)
                .or_else(|| value.get("session_id").and_then(serde_json::Value::as_str))
                .or_else(|| {
                    (value.get("type").and_then(serde_json::Value::as_str) == Some("session_meta"))
                        .then_some(())
                        .and_then(|_| payload?.get("id")?.as_str())
                })
                .map(str::to_string)
        });
        context.cwd = context.cwd.or_else(|| {
            value
                .get("cwd")
                .and_then(serde_json::Value::as_str)
                .or_else(|| payload?.get("cwd")?.as_str())
                .map(str::to_string)
        });
        context.branch = context.branch.or_else(|| {
            value
                .get("gitBranch")
                .and_then(serde_json::Value::as_str)
                .or_else(|| payload?.get("git")?.get("branch")?.as_str())
                .map(str::to_string)
        });
    }
    Ok(context)
}

fn fallback_project_slug(scan_root: &Path, file: &Path, source_root: &str) -> String {
    let parent = file.parent().unwrap_or(scan_root);
    let relative = parent.strip_prefix(scan_root).unwrap_or(parent);
    let slug = relative.to_string_lossy();
    if slug.is_empty() {
        source_root.to_string()
    } else {
        slug.to_string()
    }
}

#[cfg(test)]
#[path = "session_identity/tests.rs"]
mod tests;
