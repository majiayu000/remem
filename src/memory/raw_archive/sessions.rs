use std::collections::BTreeMap;

use anyhow::{Context, Result};
use rusqlite::Connection;
use sha2::{Digest, Sha256};

use super::{ROLE_ASSISTANT, ROLE_USER};

const SAMPLE_PREVIEW_CHARS: usize = 200;

#[derive(Debug, Clone, Default)]
pub struct RawSessionQuery {
    pub since_epoch: Option<i64>,
    pub until_epoch: Option<i64>,
    pub project: Option<String>,
    pub sample_user_messages: i64,
    pub latest: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct RawSessionSummary {
    pub session_ref: String,
    pub host: String,
    pub session_mode: String,
    pub source_root: String,
    pub project: String,
    pub session_id: String,
    pub first_epoch: i64,
    pub last_epoch: i64,
    pub message_count: i64,
    pub user_message_count: i64,
    pub assistant_message_count: i64,
    pub content_hash: String,
    pub user_message_samples: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RawSessionsJson {
    pub since_epoch: Option<i64>,
    pub until_epoch: Option<i64>,
    pub project: Option<String>,
    pub sample: i64,
    pub latest: Option<i64>,
    pub count: usize,
    pub sessions: Vec<RawSessionSummary>,
}

pub fn build_sessions_json(
    query: &RawSessionQuery,
    sessions: Vec<RawSessionSummary>,
) -> RawSessionsJson {
    RawSessionsJson {
        since_epoch: query.since_epoch,
        until_epoch: query.until_epoch,
        project: query.project.clone(),
        sample: query.sample_user_messages,
        latest: query.latest,
        count: sessions.len(),
        sessions,
    }
}

pub fn list_sessions(conn: &Connection, query: &RawSessionQuery) -> Result<Vec<RawSessionSummary>> {
    if query.latest.is_some_and(|latest| latest <= 0) {
        anyhow::bail!("raw sessions latest must be positive");
    }
    let mut sql = String::from(
        "SELECT r.transcript_identity_id, r.transcript_record_ordinal, \
                r.source_root, r.project, r.session_id, r.role, \
                r.content_hash, r.created_at_epoch, \
                CASE WHEN i.status = 'active' THEN i.host END, \
                CASE WHEN i.status = 'active' THEN i.session_mode END, \
                CASE WHEN ?1 > 0 AND r.role = 'user' THEN r.content END \
         FROM raw_messages r \
         LEFT JOIN raw_session_identities i ON i.id = r.transcript_identity_id \
         WHERE NOT (r.source = 'hook' AND r.transcript_identity_id IS NULL)",
    );
    let mut binds: Vec<Box<dyn rusqlite::types::ToSql>> =
        vec![Box::new(query.sample_user_messages.max(0))];
    if let Some(project) = query.project.as_deref() {
        sql.push_str(&format!(" AND r.project = ?{}", binds.len() + 1));
        binds.push(Box::new(project.to_string()));
    }
    push_selector_window(&mut sql, &mut binds, query);
    sql.push_str(" ORDER BY r.created_at_epoch ASC, r.id ASC");

    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(
        rusqlite::params_from_iter(crate::db::to_sql_refs(&binds)),
        |row| {
            Ok((
                row.get::<_, Option<i64>>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<String>>(10)?,
            ))
        },
    )?;

    let mut grouped = BTreeMap::new();
    for row in rows {
        let (
            identity_id,
            ordinal,
            root,
            project,
            session_id,
            role,
            hash,
            epoch,
            host,
            session_mode,
            sample,
        ) = row?;
        let host = host.with_context(|| {
            format!(
                "raw session provenance is missing or conflicted for ({root:?}, {project:?}, {session_id:?}); re-ingest its transcript"
            )
        })?;
        crate::identity::InstallHost::parse(&host)?;
        let session_mode = session_mode.with_context(|| {
            format!(
                "raw session mode provenance is missing for ({root:?}, {project:?}, {session_id:?}); re-ingest its transcript"
            )
        })?;
        if !matches!(
            session_mode.as_str(),
            "interactive" | "unattended" | "subagent" | "unknown"
        ) {
            anyhow::bail!("raw session mode provenance is invalid: {session_mode:?}");
        }
        let identity_id =
            identity_id.context("identified raw row is missing transcript identity")?;
        let ordinal = ordinal.context("identified raw row is missing transcript ordinal")?;
        let key = (
            root.clone(),
            host.clone(),
            project.clone(),
            session_id.clone(),
        );
        let accumulator = grouped.entry(key).or_insert_with(|| {
            Accumulator::new(root, host, session_mode.clone(), project, session_id, epoch)
        });
        if accumulator.session_mode != session_mode {
            anyhow::bail!(
                "raw session mode provenance conflicts for ({:?}, {:?}, {:?})",
                accumulator.source_root,
                accumulator.project,
                accumulator.session_id
            );
        }
        accumulator.push(
            identity_id,
            ordinal,
            &role,
            &hash,
            epoch,
            sample.as_deref(),
            query.sample_user_messages.max(0),
        );
    }

    let mut sessions = grouped
        .into_values()
        .map(Accumulator::finish)
        .collect::<Vec<_>>();
    if let Some(latest) = query.latest {
        sessions.sort_by(|left, right| {
            right
                .last_epoch
                .cmp(&left.last_epoch)
                .then_with(|| selector_cmp(left, right))
        });
        sessions.truncate(latest as usize);
    } else {
        sessions.sort_by(|left, right| {
            left.first_epoch
                .cmp(&right.first_epoch)
                .then_with(|| selector_cmp(left, right))
        });
    }
    Ok(sessions)
}

fn selector_cmp(left: &RawSessionSummary, right: &RawSessionSummary) -> std::cmp::Ordering {
    (
        &left.source_root,
        &left.host,
        &left.project,
        &left.session_id,
    )
        .cmp(&(
            &right.source_root,
            &right.host,
            &right.project,
            &right.session_id,
        ))
}

fn push_selector_window(
    sql: &mut String,
    binds: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
    query: &RawSessionQuery,
) {
    sql.push_str(
        " AND EXISTS (SELECT 1 FROM raw_messages w \
         LEFT JOIN raw_session_identities wi ON wi.id = w.transcript_identity_id \
         WHERE w.source_root = r.source_root AND w.project = r.project \
           AND w.session_id = r.session_id \
           AND NOT (w.source = 'hook' AND w.transcript_identity_id IS NULL) \
           AND ((i.status = 'active' AND wi.status = 'active' AND wi.host = i.host) \
                OR i.id IS NULL OR i.status != 'active' OR i.host IS NULL)",
    );
    if let Some(since) = query.since_epoch {
        sql.push_str(&format!(" AND w.created_at_epoch >= ?{}", binds.len() + 1));
        binds.push(Box::new(since));
    }
    if let Some(until) = query.until_epoch {
        sql.push_str(&format!(" AND w.created_at_epoch <= ?{}", binds.len() + 1));
        binds.push(Box::new(until));
    }
    sql.push(')');
}

struct Accumulator {
    source_root: String,
    host: String,
    session_mode: String,
    project: String,
    session_id: String,
    first_epoch: i64,
    last_epoch: i64,
    message_count: i64,
    user_message_count: i64,
    assistant_message_count: i64,
    samples: Vec<String>,
    fingerprint: SessionFingerprint,
}

impl Accumulator {
    fn new(
        root: String,
        host: String,
        session_mode: String,
        project: String,
        session: String,
        epoch: i64,
    ) -> Self {
        let fingerprint = SessionFingerprint::new(&host, &root, &project, &session);
        Self {
            source_root: root,
            host,
            session_mode,
            project,
            session_id: session,
            first_epoch: epoch,
            last_epoch: epoch,
            message_count: 0,
            user_message_count: 0,
            assistant_message_count: 0,
            samples: Vec::new(),
            fingerprint,
        }
    }

    fn push(
        &mut self,
        identity_id: i64,
        ordinal: i64,
        role: &str,
        hash: &str,
        epoch: i64,
        sample: Option<&str>,
        limit: i64,
    ) {
        self.last_epoch = epoch;
        self.message_count += 1;
        if role == ROLE_USER {
            self.user_message_count += 1;
            if self.samples.len() < limit as usize {
                if let Some(sample) = sample {
                    self.samples
                        .push(sample.chars().take(SAMPLE_PREVIEW_CHARS).collect());
                }
            }
        } else if role == ROLE_ASSISTANT {
            self.assistant_message_count += 1;
        }
        self.fingerprint
            .push(identity_id, ordinal, role, hash, epoch);
    }

    fn finish(self) -> RawSessionSummary {
        RawSessionSummary {
            session_ref: session_ref(
                &self.host,
                &self.source_root,
                &self.project,
                &self.session_id,
            ),
            host: self.host,
            session_mode: self.session_mode,
            source_root: self.source_root,
            project: self.project,
            session_id: self.session_id,
            first_epoch: self.first_epoch,
            last_epoch: self.last_epoch,
            message_count: self.message_count,
            user_message_count: self.user_message_count,
            assistant_message_count: self.assistant_message_count,
            content_hash: self.fingerprint.finish(),
            user_message_samples: self.samples,
        }
    }
}

pub(crate) struct SessionFingerprint {
    hasher: Sha256,
}

impl SessionFingerprint {
    pub(crate) fn new(host: &str, root: &str, project: &str, session: &str) -> Self {
        let mut hasher = Sha256::new();
        for field in [
            b"remem-raw-session-content-v1".as_slice(),
            root.as_bytes(),
            host.as_bytes(),
            project.as_bytes(),
            session.as_bytes(),
        ] {
            hash_field(&mut hasher, field);
        }
        Self { hasher }
    }

    pub(crate) fn push(
        &mut self,
        identity_id: i64,
        ordinal: i64,
        role: &str,
        content_hash: &str,
        epoch: i64,
    ) {
        for field in [
            &identity_id.to_le_bytes()[..],
            &ordinal.to_le_bytes(),
            role.as_bytes(),
            content_hash.as_bytes(),
            &epoch.to_le_bytes(),
        ] {
            hash_field(&mut self.hasher, field);
        }
    }

    pub(crate) fn finish(self) -> String {
        format!("sha256:{:x}", self.hasher.finalize())
    }
}

fn hash_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value);
}

fn session_ref(host: &str, root: &str, project: &str, session: &str) -> String {
    format!(
        "remem://raw-session/v2/{}/{}/{}/{}",
        hex(host),
        hex(root),
        hex(project),
        hex(session)
    )
}

fn hex(value: &str) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value.bytes() {
        output.push(char::from(DIGITS[(byte >> 4) as usize]));
        output.push(char::from(DIGITS[(byte & 15) as usize]));
    }
    output
}
