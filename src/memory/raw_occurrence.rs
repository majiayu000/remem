use anyhow::Result;
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

pub(crate) fn legacy_row_has_unique_identity(
    conn: &Connection,
    raw_message_id: i64,
    expected_identity_id: i64,
) -> Result<bool> {
    let mut statement = conn.prepare(
        "SELECT DISTINCT i.id
         FROM raw_messages r
         JOIN raw_session_identities i
           ON i.source_root = r.source_root
          AND r.session_id IN (i.fallback_session_id, i.canonical_session_id)
          AND r.project IN (i.project, i.legacy_project)
         WHERE r.id = ?1 AND i.host IS NOT NULL
         ORDER BY i.id",
    )?;
    let candidates = statement
        .query_map([raw_message_id], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(candidates == [expected_identity_id])
}

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
    reject_ambiguous_matching_unresolved_legacy_row(
        conn,
        source_root,
        role,
        &content_hash,
        transcript_identity_id,
    )?;
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
    let existing: Option<(i64, String, String, String, String, i64, String)> = conn
        .query_row(
            "SELECT id, role, content, content_hash, event_time_source,
                    created_at_epoch, source_root
             FROM raw_messages
             WHERE transcript_identity_id = ?1 AND transcript_record_ordinal = ?2",
            params![identity_id, ordinal],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .optional()?;
    let Some((
        id,
        stored_role,
        stored_content,
        stored_hash,
        stored_time_source,
        stored_epoch,
        stored_source_root,
    )) = existing
    else {
        return Ok(None);
    };
    let timestamp_matches =
        event_time_source != EVENT_TIME_TRANSCRIPT || created_at_epoch == Some(stored_epoch);
    if stored_role != role
        || stored_content != content
        || stored_hash != content_hash
        || stored_time_source != event_time_source
        || stored_source_root != source_root
        || !timestamp_matches
    {
        return Err(RawIdentityConflict {
            reason: format!("ordinal {ordinal} stable fields differ from the captured transcript"),
        }
        .into());
    }
    conn.execute(
        "UPDATE raw_messages SET project = ?2, session_id = ?3 WHERE id = ?1",
        params![id, project, session_id],
    )?;
    Ok(Some(id))
}

fn reject_ambiguous_matching_unresolved_legacy_row(
    conn: &Connection,
    source_root: &str,
    role: &str,
    content_hash: &str,
    identity_id: i64,
) -> Result<()> {
    let row_ids = {
        let mut statement = conn.prepare(
            "SELECT r.id
             FROM raw_messages r
             JOIN raw_session_identities i ON i.id = ?1
             WHERE r.transcript_identity_id IS NULL
               AND r.source_root = ?2
               AND r.project IN (i.project, i.legacy_project)
               AND r.session_id IN (i.fallback_session_id, i.canonical_session_id)
               AND r.role = ?3 AND r.content_hash = ?4
               AND r.source = 'transcript'
            ORDER BY r.id",
        )?;
        let rows = statement
            .query_map(
                params![identity_id, source_root, role, content_hash],
                |row| row.get::<_, i64>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for row_id in row_ids {
        if !legacy_row_has_unique_identity(conn, row_id, identity_id)? {
            return Err(RawIdentityConflict {
                reason: format!(
                    "legacy raw row {row_id} has ambiguous or untrusted host provenance and cannot be claimed"
                ),
            }
            .into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unresolved_legacy_row_fails_before_claim_mutation() {
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
            [&hash],
        )
        .expect("insert legacy row");
        conn.execute(
            "INSERT INTO raw_messages (
                id, session_id, project, role, content, content_hash, source,
                created_at_epoch, source_root, event_time_source,
                transcript_identity_id, transcript_record_ordinal
             ) VALUES (42, 'old-canonical', 'old-project', 'user', 'repeat',
                       ?1, 'transcript', 100, 'local', 'transcript_event', 1, 0)",
            [&hash],
        )
        .expect("insert identified replay row");

        let error = insert_transcript_occurrence(
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
        .expect_err("hostless legacy occurrence must remain unresolved");

        assert!(error.downcast_ref::<RawIdentityConflict>().is_some());
        assert_eq!(
            conn.query_row(
                "SELECT session_id || ':' || project || ':' || event_time_source
                 FROM raw_messages WHERE id = 41 AND transcript_identity_id IS NULL",
                [],
                |row| row.get::<_, String>(0)
            )
            .expect("load unchanged legacy row"),
            "fallback:legacy-project:legacy_unknown"
        );
        assert_eq!(
            conn.query_row(
                "SELECT session_id || ':' || project FROM raw_messages WHERE id = 42",
                [],
                |row| row.get::<_, String>(0)
            )
            .expect("load unchanged identified row"),
            "old-canonical:old-project"
        );
    }

    #[test]
    fn unresolved_legacy_aliases_fail_closed_without_mutation() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        conn.execute(
            "INSERT INTO raw_session_identities (
                id, source_root, transcript_path, fallback_session_id,
                canonical_session_id, project, legacy_project, status,
                contract_version, observed_mtime_ns, observed_size_bytes,
                first_seen_at_epoch, last_seen_at_epoch
             ) VALUES (1, 'local', '/tmp/split-alias.jsonl', 'fallback',
                       'canonical', 'current-project', 'legacy-project',
                       'active', 0, 1, 1, 1, 1)",
            [],
        )?;
        let hash = crate::db::content_identity_hash(b"same occurrence");
        conn.execute(
            "INSERT INTO raw_messages (
                id, session_id, project, role, content, content_hash, source,
                created_at_epoch, source_root, event_time_source
             ) VALUES
                (41, 'fallback', 'legacy-project', 'user', 'same occurrence',
                 ?1, 'transcript', 7, 'local', 'legacy_unknown'),
                (42, 'canonical', 'current-project', 'user', 'same occurrence',
                 ?1, 'transcript', 8, 'local', 'legacy_unknown'),
                (43, 'fallback', 'current-project', 'user', 'same occurrence',
                 ?1, 'hook', 9, 'local', 'legacy_unknown')",
            [hash],
        )?;

        let error = insert_transcript_occurrence(
            &conn,
            "canonical",
            "current-project",
            "user",
            "same occurrence",
            None,
            None,
            "local",
            Some(100),
            1,
            0,
        )
        .expect_err("hostless aliases cannot be assigned to the current host");

        assert!(error.downcast_ref::<RawIdentityConflict>().is_some());
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM raw_messages
                 WHERE transcript_identity_id IS NULL",
                [],
                |row| row.get::<_, i64>(0)
            )?,
            3
        );
        assert_eq!(
            conn.query_row(
                "SELECT GROUP_CONCAT(id || ':' || session_id || ':' || project, ',')
                 FROM (SELECT id, session_id, project FROM raw_messages ORDER BY id)",
                [],
                |row| row.get::<_, String>(0)
            )?,
            "41:fallback:legacy-project,42:canonical:current-project,43:fallback:current-project"
        );
        Ok(())
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

    #[test]
    fn unresolved_legacy_timestamp_row_fails_before_mutation() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        conn.execute(
            "INSERT INTO raw_session_identities (
                id, source_root, transcript_path, fallback_session_id,
                canonical_session_id, project, legacy_project, status,
                contract_version, observed_mtime_ns, observed_size_bytes,
                first_seen_at_epoch, last_seen_at_epoch
             ) VALUES (1, 'local', '/tmp/downgrade.jsonl', 'fallback',
                       'canonical', 'project', 'legacy', 'active',
                       0, 1, 1, 1, 1)",
            [],
        )?;
        let hash = crate::db::content_identity_hash(b"same");
        conn.execute(
            "INSERT INTO raw_messages (
                session_id, project, role, content, content_hash, source,
                created_at_epoch, source_root, event_time_source
             ) VALUES ('fallback', 'legacy', 'user', 'same', ?1,
                       'transcript', 100, 'local', 'transcript_event')",
            [hash],
        )?;

        let error = insert_transcript_occurrence(
            &conn,
            "canonical",
            "project",
            "user",
            "same",
            None,
            None,
            "local",
            None,
            1,
            0,
        )
        .expect_err("missing timestamp cannot downgrade transcript provenance");

        assert!(error.downcast_ref::<RawIdentityConflict>().is_some());
        Ok(())
    }
}
