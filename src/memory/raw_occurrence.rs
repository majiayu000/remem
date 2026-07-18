use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};

use super::raw_archive::RawInsertOutcome;

pub(crate) const EVENT_TIME_TRANSCRIPT: &str = "transcript_event";
pub(crate) const EVENT_TIME_FALLBACK: &str = "ingest_fallback";

#[derive(Debug)]
pub(crate) struct RawIdentityConflict {
    pub reason: String,
}

impl std::fmt::Display for RawIdentityConflict {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "raw transcript identity conflict: {}",
            self.reason
        )
    }
}

impl std::error::Error for RawIdentityConflict {}

#[allow(clippy::too_many_arguments)]
pub(crate) fn insert_transcript_occurrence(
    conn: &Connection,
    session_id: &str,
    project: &str,
    role: &str,
    content: &str,
    branch: Option<&str>,
    cwd: Option<&str>,
    source_root: &str,
    created_at_epoch: Option<i64>,
    transcript_identity_id: i64,
    transcript_record_ordinal: i64,
) -> Result<Option<RawInsertOutcome>> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let content_hash = crate::db::content_identity_hash(trimmed.as_bytes());
    let stored_epoch = created_at_epoch.unwrap_or_else(|| chrono::Utc::now().timestamp());
    let event_time_source = if created_at_epoch.is_some() {
        EVENT_TIME_TRANSCRIPT
    } else {
        EVENT_TIME_FALLBACK
    };
    if let Some(id) = existing_occurrence(
        conn,
        source_root,
        project,
        session_id,
        transcript_identity_id,
        transcript_record_ordinal,
        role,
        trimmed,
        &content_hash,
        created_at_epoch,
        event_time_source,
    )? {
        return Ok(Some(RawInsertOutcome {
            id,
            inserted: false,
        }));
    }
    if let Some(id) = claim_matching_legacy_row(
        conn,
        session_id,
        project,
        role,
        trimmed,
        &content_hash,
        branch,
        cwd,
        source_root,
        created_at_epoch,
        event_time_source,
        transcript_identity_id,
        transcript_record_ordinal,
    )? {
        return Ok(Some(RawInsertOutcome {
            id,
            inserted: false,
        }));
    }
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO raw_messages (
            session_id, project, role, content, content_hash, source, branch, cwd,
            created_at_epoch, source_root, event_time_source,
            transcript_identity_id, transcript_record_ordinal
         ) VALUES (?1, ?2, ?3, ?4, ?5, 'transcript', ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            session_id,
            project,
            role,
            trimmed,
            content_hash,
            branch,
            cwd,
            stored_epoch,
            source_root,
            event_time_source,
            transcript_identity_id,
            transcript_record_ordinal
        ],
    )?;
    if inserted > 0 {
        return Ok(Some(RawInsertOutcome {
            id: conn.last_insert_rowid(),
            inserted: true,
        }));
    }

    let id = existing_occurrence(
        conn,
        source_root,
        project,
        session_id,
        transcript_identity_id,
        transcript_record_ordinal,
        role,
        trimmed,
        &content_hash,
        created_at_epoch,
        event_time_source,
    )?
    .ok_or_else(|| anyhow::anyhow!("raw occurrence insert was ignored without a target row"))?;
    Ok(Some(RawInsertOutcome {
        id,
        inserted: false,
    }))
}

#[allow(clippy::too_many_arguments)]
fn existing_occurrence(
    conn: &Connection,
    source_root: &str,
    project: &str,
    session_id: &str,
    identity_id: i64,
    ordinal: i64,
    role: &str,
    content: &str,
    content_hash: &str,
    created_at_epoch: Option<i64>,
    event_time_source: &str,
) -> Result<Option<i64>> {
    let existing: Option<(i64, String, String, String, String, i64)> = conn
        .query_row(
            "SELECT id, role, content, content_hash, event_time_source, created_at_epoch
             FROM raw_messages
             WHERE source_root = ?1 AND project = ?2 AND session_id = ?3
               AND transcript_identity_id = ?4 AND transcript_record_ordinal = ?5",
            params![source_root, project, session_id, identity_id, ordinal],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()?;
    let Some((id, stored_role, stored_content, stored_hash, stored_time_source, stored_epoch)) =
        existing
    else {
        return Ok(None);
    };
    let timestamp_matches =
        event_time_source != EVENT_TIME_TRANSCRIPT || created_at_epoch == Some(stored_epoch);
    if stored_role != role
        || stored_content != content
        || stored_hash != content_hash
        || stored_time_source != event_time_source
        || !timestamp_matches
    {
        return Err(RawIdentityConflict {
            reason: format!("ordinal {ordinal} stable fields differ from the captured transcript"),
        }
        .into());
    }
    Ok(Some(id))
}

#[allow(clippy::too_many_arguments)]
fn claim_matching_legacy_row(
    conn: &Connection,
    session_id: &str,
    project: &str,
    role: &str,
    content: &str,
    content_hash: &str,
    branch: Option<&str>,
    cwd: Option<&str>,
    source_root: &str,
    created_at_epoch: Option<i64>,
    event_time_source: &str,
    identity_id: i64,
    ordinal: i64,
) -> Result<Option<i64>> {
    let row: Option<(i64, String, i64, String)> = conn
        .query_row(
            "SELECT r.id, r.content, r.created_at_epoch, r.event_time_source
             FROM raw_messages r
             JOIN raw_session_identities i ON i.id = ?1
             WHERE r.transcript_identity_id IS NULL
               AND r.source_root = ?2
               AND r.project IN (i.project, i.legacy_project)
               AND r.session_id IN (i.fallback_session_id, i.canonical_session_id)
               AND r.role = ?3 AND r.content_hash = ?4
               AND r.source = 'transcript'
             ORDER BY r.id LIMIT 1",
            params![identity_id, source_root, role, content_hash],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let Some((id, old_content, old_epoch, old_time_source)) = row else {
        return Ok(None);
    };
    if old_content.trim() != content {
        bail!("legacy raw row content disagrees with its content hash");
    }
    if old_time_source == EVENT_TIME_TRANSCRIPT
        && created_at_epoch.is_some_and(|epoch| epoch != old_epoch)
    {
        bail!("legacy transcript event time conflicts with the source occurrence");
    }
    let stored_epoch = created_at_epoch.unwrap_or(old_epoch);
    conn.execute(
        "UPDATE raw_messages
         SET session_id = ?2, project = ?3, branch = COALESCE(branch, ?4),
             cwd = COALESCE(cwd, ?5), created_at_epoch = ?6,
             event_time_source = ?7, transcript_identity_id = ?8,
             transcript_record_ordinal = ?9
         WHERE id = ?1",
        params![
            id,
            session_id,
            project,
            branch,
            cwd,
            stored_epoch,
            event_time_source,
            identity_id,
            ordinal
        ],
    )?;
    Ok(Some(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_row_is_claimed_in_place_and_repeated_turn_is_preserved() {
        let conn = Connection::open_in_memory().expect("open occurrence fixture");
        crate::migrate::run_migrations(&conn).expect("migrate occurrence fixture");
        conn.execute(
            "INSERT INTO raw_session_identities (
                id, source_root, transcript_path, fallback_session_id,
                canonical_session_id, project, legacy_project, status,
                contract_version, observed_mtime_ns, observed_size_bytes,
                first_seen_at_epoch, last_seen_at_epoch
             ) VALUES (1, 'local', '/tmp/repeated.jsonl', 'fallback',
                       'canonical', 'current-project', 'legacy-project',
                       'active', 0, 1, 1, 1, 1)",
            [],
        )
        .expect("insert identity");
        let hash = crate::db::content_identity_hash(b"repeat");
        conn.execute(
            "INSERT INTO raw_messages (
                id, session_id, project, role, content, content_hash, source,
                created_at_epoch, source_root, event_time_source
             ) VALUES (41, 'fallback', 'legacy-project', 'user', 'repeat',
                       ?1, 'transcript', 7, 'local', 'legacy_unknown')",
            [hash],
        )
        .expect("insert legacy row");

        let first = insert_transcript_occurrence(
            &conn,
            "canonical",
            "current-project",
            "user",
            "repeat",
            None,
            None,
            "local",
            Some(100),
            1,
            0,
        )
        .expect("claim first occurrence")
        .expect("non-empty first occurrence");
        let second = insert_transcript_occurrence(
            &conn,
            "canonical",
            "current-project",
            "user",
            "repeat",
            None,
            None,
            "local",
            Some(101),
            1,
            1,
        )
        .expect("insert repeated occurrence")
        .expect("non-empty second occurrence");

        assert_eq!(first.id, 41);
        assert!(!first.inserted);
        assert!(second.inserted);
        let rows: Vec<(i64, i64, i64, String)> = {
            let mut statement = conn
                .prepare(
                    "SELECT id, transcript_identity_id, transcript_record_ordinal,
                            event_time_source
                     FROM raw_messages ORDER BY transcript_record_ordinal",
                )
                .expect("prepare occurrence rows");
            statement
                .query_map([], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })
                .expect("query occurrence rows")
                .collect::<rusqlite::Result<Vec<_>>>()
                .expect("collect occurrence rows")
        };
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], (41, 1, 0, EVENT_TIME_TRANSCRIPT.to_string()));
        assert_eq!(rows[1].2, 1);
    }

    #[test]
    fn replayed_ordinal_with_different_stable_fields_is_a_conflict() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        conn.execute(
            "INSERT INTO raw_session_identities (
                id, source_root, transcript_path, fallback_session_id,
                canonical_session_id, project, legacy_project, status,
                contract_version, observed_mtime_ns, observed_size_bytes,
                first_seen_at_epoch, last_seen_at_epoch
             ) VALUES (1, 'local', '/tmp/replay.jsonl', 'fallback',
                       'canonical', 'project', 'legacy', 'active',
                       0, 1, 1, 1, 1)",
            [],
        )?;

        let first = insert_transcript_occurrence(
            &conn,
            "canonical",
            "project",
            "user",
            "original",
            None,
            None,
            "local",
            Some(100),
            1,
            7,
        )?;
        assert!(first.is_some());
        let error = insert_transcript_occurrence(
            &conn,
            "canonical",
            "project",
            "assistant",
            "replacement",
            None,
            None,
            "local",
            Some(101),
            1,
            7,
        )
        .expect_err("ordinal reuse with changed stable fields must fail");

        assert!(error.downcast_ref::<RawIdentityConflict>().is_some());
        assert_eq!(
            conn.query_row(
                "SELECT role || ':' || content FROM raw_messages
                 WHERE transcript_identity_id = 1 AND transcript_record_ordinal = 7",
                [],
                |row| row.get::<_, String>(0)
            )?,
            "user:original"
        );
        Ok(())
    }
}
