use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};
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

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ExcludedSessionIdentity {
    pub source_root: String,
    pub project: String,
    pub session_id: String,
    pub host: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawSessionListing {
    pub(crate) sessions: Vec<RawSessionSummary>,
    pub(crate) excluded_legacy_rows: usize,
    pub(crate) excluded_legacy_sessions: usize,
    pub(crate) excluded_legacy_identities: Vec<ExcludedSessionIdentity>,
}

impl std::ops::Deref for RawSessionListing {
    type Target = [RawSessionSummary];

    fn deref(&self) -> &Self::Target {
        &self.sessions
    }
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

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct RawSessionListingJson {
    since_epoch: Option<i64>,
    until_epoch: Option<i64>,
    project: Option<String>,
    sample: i64,
    latest: Option<i64>,
    count: usize,
    excluded_legacy_rows: usize,
    excluded_legacy_sessions: usize,
    excluded_legacy_identities: Vec<ExcludedSessionIdentity>,
    sessions: Vec<RawSessionSummary>,
}

pub(crate) fn build_session_listing_json(
    query: &RawSessionQuery,
    listing: RawSessionListing,
) -> RawSessionListingJson {
    RawSessionListingJson {
        since_epoch: query.since_epoch,
        until_epoch: query.until_epoch,
        project: query.project.clone(),
        sample: query.sample_user_messages,
        latest: query.latest,
        count: listing.sessions.len(),
        excluded_legacy_rows: listing.excluded_legacy_rows,
        excluded_legacy_sessions: listing.excluded_legacy_sessions,
        excluded_legacy_identities: listing.excluded_legacy_identities,
        sessions: listing.sessions,
    }
}

pub fn list_sessions(conn: &Connection, query: &RawSessionQuery) -> Result<Vec<RawSessionSummary>> {
    Ok(list_sessions_with_exclusions(conn, query)?.sessions)
}

pub(crate) fn list_sessions_with_exclusions(
    conn: &Connection,
    query: &RawSessionQuery,
) -> Result<RawSessionListing> {
    if query.latest.is_some_and(|latest| latest <= 0) {
        anyhow::bail!("raw sessions latest must be positive");
    }
    let mut sql = String::from(
        "SELECT r.transcript_identity_id, r.transcript_record_ordinal, \
                r.source_root, r.project, r.session_id, r.role, \
                r.content_hash, r.created_at_epoch, r.id, r.source, \
                r.event_time_source, \
                i.host, i.session_mode, i.status \
         FROM raw_messages r \
         LEFT JOIN raw_session_identities i ON i.id = r.transcript_identity_id \
         WHERE NOT (r.source = 'hook' AND r.transcript_identity_id IS NULL)",
    );
    let mut binds: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
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
                row.get::<_, i64>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, Option<String>>(11)?,
                row.get::<_, Option<String>>(12)?,
                row.get::<_, Option<String>>(13)?,
            ))
        },
    )?;

    let mut grouped: BTreeMap<(String, String, String, String), Accumulator> = BTreeMap::new();
    let mut exclusions = ExclusionState::default();
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
            row_id,
            source,
            event_time_source,
            host,
            session_mode,
            status,
        ) = row?;
        let active = status.as_deref() == Some("active");
        if identity_id.is_none() && source == "transcript" && event_time_source == "legacy_unknown"
        {
            exclusions.exclude(&root, &project, &session_id, None, ExclusionTaint::Tuple);
            continue;
        }
        if !active {
            let taint = if identity_id.is_none() || host.is_none() {
                ExclusionTaint::Tuple
            } else {
                ExclusionTaint::Key
            };
            exclusions.exclude(&root, &project, &session_id, host.as_deref(), taint);
            continue;
        }
        let Some(host) = host else {
            exclusions.exclude(&root, &project, &session_id, None, ExclusionTaint::Tuple);
            continue;
        };
        if crate::identity::InstallHost::parse(&host).is_err() {
            exclusions.exclude(
                &root,
                &project,
                &session_id,
                Some(&host),
                ExclusionTaint::Key,
            );
            continue;
        }
        let Some(session_mode) = session_mode else {
            exclusions.exclude(
                &root,
                &project,
                &session_id,
                Some(&host),
                ExclusionTaint::Key,
            );
            continue;
        };
        if !is_closed_session_mode(&session_mode) {
            exclusions.exclude(
                &root,
                &project,
                &session_id,
                Some(&host),
                ExclusionTaint::Key,
            );
            continue;
        }
        if exclusions.is_tainted(&root, &host, &project, &session_id) {
            exclusions.exclude(
                &root,
                &project,
                &session_id,
                Some(&host),
                ExclusionTaint::None,
            );
            continue;
        }
        let Some(identity_id) = identity_id else {
            exclusions.exclude(
                &root,
                &project,
                &session_id,
                Some(&host),
                ExclusionTaint::Key,
            );
            continue;
        };
        let Some(ordinal) = ordinal else {
            exclusions.exclude(
                &root,
                &project,
                &session_id,
                Some(&host),
                ExclusionTaint::Key,
            );
            continue;
        };
        let key = (
            root.clone(),
            host.clone(),
            project.clone(),
            session_id.clone(),
        );
        if let Some(accumulator) = grouped.get(&key) {
            if accumulator.session_mode != session_mode {
                exclusions.exclude(
                    &root,
                    &project,
                    &session_id,
                    Some(&host),
                    ExclusionTaint::Key,
                );
                continue;
            }
        }
        let accumulator = grouped.entry(key).or_insert_with(|| {
            Accumulator::new(root, host, session_mode.clone(), project, session_id, epoch)
        });
        accumulator.push(
            identity_id,
            ordinal,
            &role,
            &hash,
            epoch,
            row_id,
            query.sample_user_messages.max(0),
        );
    }
    grouped.retain(|key, accumulator| {
        let (root, host, project, session_id) = key;
        if !exclusions.is_tainted(root, host, project, session_id) {
            return true;
        }
        exclusions.drop_accumulated(accumulator);
        false
    });

    let mut accumulators = grouped.into_values().collect::<Vec<_>>();
    if let Some(latest) = query.latest {
        accumulators.sort_by(|left, right| {
            right
                .last_epoch
                .cmp(&left.last_epoch)
                .then_with(|| accumulator_selector_cmp(left, right))
        });
        accumulators.truncate(latest as usize);
    } else {
        accumulators.sort_by(|left, right| {
            left.first_epoch
                .cmp(&right.first_epoch)
                .then_with(|| accumulator_selector_cmp(left, right))
        });
    }
    let mut sample_statement = conn.prepare(
        "SELECT substr(content, 1, ?2)
         FROM raw_messages
         WHERE id = ?1 AND role = 'user'",
    )?;
    let sessions = accumulators
        .into_iter()
        .map(|accumulator| {
            let samples = accumulator
                .sample_ids
                .iter()
                .map(|row_id| {
                    sample_statement
                        .query_row(
                            rusqlite::params![row_id, SAMPLE_PREVIEW_CHARS as i64],
                            |row| row.get::<_, String>(0),
                        )
                        .optional()?
                        .with_context(|| format!("raw session sample row {row_id} is missing"))
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(accumulator.finish(samples))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(exclusions.into_listing(sessions))
}

fn is_closed_session_mode(session_mode: &str) -> bool {
    matches!(
        session_mode,
        "interactive" | "unattended" | "subagent" | "unknown"
    )
}

#[derive(Clone, Copy)]
enum ExclusionTaint {
    Tuple,
    Key,
    None,
}

#[derive(Default)]
struct ExclusionState {
    rows: usize,
    identities: BTreeSet<ExcludedSessionIdentity>,
    tainted_tuples: BTreeSet<(String, String, String)>,
    tainted_keys: BTreeSet<(String, String, String, String)>,
}

impl ExclusionState {
    fn exclude(
        &mut self,
        source_root: &str,
        project: &str,
        session_id: &str,
        host: Option<&str>,
        taint: ExclusionTaint,
    ) {
        self.rows += 1;
        self.identities.insert(ExcludedSessionIdentity {
            source_root: source_root.to_string(),
            project: project.to_string(),
            session_id: session_id.to_string(),
            host: host.map(str::to_string),
        });
        match taint {
            ExclusionTaint::Tuple => {
                self.tainted_tuples.insert((
                    source_root.to_string(),
                    project.to_string(),
                    session_id.to_string(),
                ));
            }
            ExclusionTaint::Key => {
                if let Some(host) = host {
                    self.tainted_keys.insert((
                        source_root.to_string(),
                        host.to_string(),
                        project.to_string(),
                        session_id.to_string(),
                    ));
                } else {
                    self.tainted_tuples.insert((
                        source_root.to_string(),
                        project.to_string(),
                        session_id.to_string(),
                    ));
                }
            }
            ExclusionTaint::None => {}
        }
    }

    fn is_tainted(&self, root: &str, host: &str, project: &str, session_id: &str) -> bool {
        self.tainted_tuples
            .iter()
            .any(|(tainted_root, tainted_project, tainted_session)| {
                tainted_root == root && tainted_project == project && tainted_session == session_id
            })
            || self.tainted_keys.contains(&(
                root.to_string(),
                host.to_string(),
                project.to_string(),
                session_id.to_string(),
            ))
    }

    fn drop_accumulated(&mut self, accumulator: &Accumulator) {
        self.rows += accumulator.message_count as usize;
        self.identities.insert(ExcludedSessionIdentity {
            source_root: accumulator.source_root.clone(),
            project: accumulator.project.clone(),
            session_id: accumulator.session_id.clone(),
            host: Some(accumulator.host.clone()),
        });
    }

    fn into_listing(self, sessions: Vec<RawSessionSummary>) -> RawSessionListing {
        let excluded_legacy_sessions = self
            .identities
            .iter()
            .map(|identity| {
                (
                    identity.source_root.clone(),
                    identity.project.clone(),
                    identity.session_id.clone(),
                )
            })
            .collect::<BTreeSet<_>>()
            .len();
        RawSessionListing {
            sessions,
            excluded_legacy_rows: self.rows,
            excluded_legacy_sessions,
            excluded_legacy_identities: self.identities.into_iter().collect(),
        }
    }
}

fn accumulator_selector_cmp(left: &Accumulator, right: &Accumulator) -> std::cmp::Ordering {
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
    sample_ids: Vec<i64>,
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
            sample_ids: Vec::new(),
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
        row_id: i64,
        limit: i64,
    ) {
        self.last_epoch = epoch;
        self.message_count += 1;
        if role == ROLE_USER {
            self.user_message_count += 1;
            if self.sample_ids.len() < limit as usize {
                self.sample_ids.push(row_id);
            }
        } else if role == ROLE_ASSISTANT {
            self.assistant_message_count += 1;
        }
        self.fingerprint
            .push(identity_id, ordinal, role, hash, epoch);
    }

    fn finish(self, samples: Vec<String>) -> RawSessionSummary {
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
            user_message_samples: samples,
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
