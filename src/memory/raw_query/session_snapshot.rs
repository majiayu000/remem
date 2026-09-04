use anyhow::Result;
use rusqlite::{params, Connection};

use super::RawSessionMessagesRequest;

pub(super) fn ensure_provenance_resolved(
    conn: &Connection,
    request: &RawSessionMessagesRequest,
) -> Result<()> {
    let (tuple_exists, requested_host_exists, unresolved) = conn.query_row(
        "WITH eligible AS ( \
             SELECT r.transcript_identity_id \
             FROM raw_messages r \
             WHERE r.source_root = ?1 AND r.project = ?2 AND r.session_id = ?3 \
               AND NOT (r.source = 'hook' AND r.transcript_identity_id IS NULL) \
         ) \
         SELECT EXISTS(SELECT 1 FROM eligible), \
                EXISTS( \
                    SELECT 1 FROM eligible e \
                    JOIN raw_session_identities i ON i.id = e.transcript_identity_id \
                    WHERE i.host = ?4 AND i.status = 'active' \
                ), \
                EXISTS( \
                    SELECT 1 FROM eligible e \
                    LEFT JOIN raw_session_identities i ON i.id = e.transcript_identity_id \
                    WHERE i.id IS NULL OR i.host IS NULL \
                       OR (i.host = ?4 AND i.status != 'active') \
                )",
        params![
            request.source_root,
            request.project,
            request.session_id,
            request.host
        ],
        |row| {
            Ok((
                row.get::<_, bool>(0)?,
                row.get::<_, bool>(1)?,
                row.get::<_, bool>(2)?,
            ))
        },
    )?;
    if unresolved {
        anyhow::bail!(
            "raw session provenance is missing or conflicted for ({:?}, {:?}, {:?})",
            request.source_root,
            request.project,
            request.session_id,
        );
    }
    if tuple_exists && !requested_host_exists {
        anyhow::bail!(
            "raw session host mismatch for ({:?}, {:?}, {:?}): no active transcript identity for {:?}",
            request.source_root,
            request.project,
            request.session_id,
            request.host,
        );
    }
    Ok(())
}

pub(super) fn content_hash(
    conn: &Connection,
    request: &RawSessionMessagesRequest,
    snapshot_max_id: Option<i64>,
) -> Result<String> {
    let mut fingerprint = crate::memory::raw_archive::SessionFingerprint::new(
        &request.host,
        &request.source_root,
        &request.project,
        &request.session_id,
    );
    let Some(snapshot_max_id) = snapshot_max_id else {
        return Ok(fingerprint.finish());
    };
    let mut statement = conn.prepare(
        "SELECT r.transcript_identity_id, r.transcript_record_ordinal, r.role, \
                r.content_hash, r.created_at_epoch \
         FROM raw_messages r \
         JOIN raw_session_identities i ON i.id = r.transcript_identity_id \
         WHERE r.source_root = ?1 AND r.project = ?2 AND r.session_id = ?3 \
           AND i.status = 'active' AND i.host = ?4 AND r.id <= ?5 \
         ORDER BY r.created_at_epoch ASC, r.id ASC",
    )?;
    let rows = statement.query_map(
        params![
            request.source_root,
            request.project,
            request.session_id,
            request.host,
            snapshot_max_id
        ],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        },
    )?;
    for row in rows {
        let (identity_id, ordinal, role, hash, epoch) = row?;
        fingerprint.push(identity_id, ordinal, &role, &hash, epoch);
    }
    Ok(fingerprint.finish())
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use rusqlite::params;

    use super::super::{query_raw_session_messages, RawSessionMessagesRequest};
    use crate::memory::raw_archive::{
        insert_raw_message_from_root_at, list_sessions, RawSessionQuery, ROLE_ASSISTANT, ROLE_USER,
        SOURCE_HOOK, SOURCE_TRANSCRIPT,
    };

    #[test]
    fn unbound_hook_fallback_does_not_poison_transcript_page_or_hash() -> Result<()> {
        let conn = rusqlite::Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        let transcript_id = insert_raw_message_from_root_at(
            &conn,
            "s1",
            "/repo",
            ROLE_USER,
            "question",
            SOURCE_TRANSCRIPT,
            None,
            Some("/repo"),
            "root-a",
            Some(10),
        )?
        .context("transcript row")?
        .id;
        conn.execute(
            "INSERT INTO raw_session_identities
             (source_root, transcript_path, host, session_mode, fallback_session_id,
              canonical_session_id, project, legacy_project, status,
              contract_version, observed_mtime_ns, observed_size_bytes,
              first_seen_at_epoch, last_seen_at_epoch)
             VALUES ('root-a', '/tmp/.codex/sessions/s1.jsonl', 'codex-cli',
                     'interactive', 's1', 's1', '/repo', '/repo',
                     'active', 1, 1, 1, 1, 1)",
            [],
        )?;
        conn.execute(
            "UPDATE raw_messages
             SET transcript_identity_id = ?1, transcript_record_ordinal = 1
             WHERE id = ?2",
            params![conn.last_insert_rowid(), transcript_id],
        )?;
        insert_raw_message_from_root_at(
            &conn,
            "s1",
            "/repo",
            ROLE_ASSISTANT,
            "partial hook fallback",
            SOURCE_HOOK,
            None,
            Some("/repo"),
            "root-a",
            Some(20),
        )?
        .context("hook fallback row")?;
        insert_raw_message_from_root_at(
            &conn,
            "hook-only",
            "/repo",
            ROLE_ASSISTANT,
            "standalone hook fallback",
            SOURCE_HOOK,
            None,
            Some("/repo"),
            "root-a",
            Some(30),
        )?
        .context("hook-only fallback row")?;

        let summaries = list_sessions(&conn, &RawSessionQuery::default())?;
        let page = query_raw_session_messages(
            &conn,
            &RawSessionMessagesRequest {
                host: "codex-cli".to_string(),
                source_root: "root-a".to_string(),
                project: "/repo".to_string(),
                session_id: "s1".to_string(),
                limit: 2,
                cursor: None,
            },
        )?;

        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].message_count, 1);
        assert_eq!(page.messages.len(), 1);
        assert_eq!(page.messages[0].id, transcript_id);
        assert_eq!(page.content_hash, summaries[0].content_hash);
        assert!(!page.has_more);
        assert!(page.next_cursor.is_none());
        let wrong_host = query_raw_session_messages(
            &conn,
            &RawSessionMessagesRequest {
                host: "claude-code".to_string(),
                source_root: "root-a".to_string(),
                project: "/repo".to_string(),
                session_id: "s1".to_string(),
                limit: 2,
                cursor: None,
            },
        )
        .expect_err("a selector naming only another active host must fail");
        assert!(wrong_host.to_string().contains("host mismatch"));
        let hook_only = query_raw_session_messages(
            &conn,
            &RawSessionMessagesRequest {
                host: "codex-cli".to_string(),
                source_root: "root-a".to_string(),
                project: "/repo".to_string(),
                session_id: "hook-only".to_string(),
                limit: 2,
                cursor: None,
            },
        )?;
        assert!(hook_only.messages.is_empty());
        assert!(!hook_only.has_more);
        assert!(hook_only.next_cursor.is_none());
        Ok(())
    }
}
