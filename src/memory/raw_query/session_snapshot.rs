use anyhow::Result;
use rusqlite::{params, Connection};

use super::RawSessionMessagesRequest;

pub(super) fn ensure_provenance_resolved(
    conn: &Connection,
    request: &RawSessionMessagesRequest,
) -> Result<()> {
    let unresolved = conn.query_row(
        "SELECT EXISTS( \
             SELECT 1 FROM raw_messages r \
             LEFT JOIN raw_session_identities i ON i.id = r.transcript_identity_id \
             WHERE r.source_root = ?1 AND r.project = ?2 AND r.session_id = ?3 \
               AND (i.id IS NULL OR i.host IS NULL \
                    OR (i.host = ?4 AND i.status != 'active')) \
         )",
        params![
            request.source_root,
            request.project,
            request.session_id,
            request.host
        ],
        |row| row.get::<_, bool>(0),
    )?;
    if unresolved {
        anyhow::bail!(
            "raw session provenance is missing or conflicted for ({:?}, {:?}, {:?}); re-ingest its transcript",
            request.source_root,
            request.project,
            request.session_id,
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
