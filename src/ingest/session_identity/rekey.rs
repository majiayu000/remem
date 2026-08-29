use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use super::IdentityRecord;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RekeyReport {
    pub merged: usize,
}

pub(crate) fn rekey_legacy_rows(
    conn: &Connection,
    identity: &IdentityRecord,
) -> Result<RekeyReport> {
    if identity.status == "conflict" {
        return Ok(RekeyReport::default());
    }
    let unresolved_row_id = conn
        .query_row(
            "SELECT id
             FROM raw_messages
             WHERE source_root = ?1 AND session_id IN (?2, ?3)
               AND project IN (?4, ?5) AND transcript_identity_id IS NULL
               AND source = 'transcript'
             ORDER BY id
             LIMIT 1",
            params![
                identity.source_root,
                identity.fallback_session_id,
                identity.canonical_session_id,
                identity.project,
                identity.legacy_project
            ],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    if let Some(row_id) = unresolved_row_id {
        return Err(crate::memory::raw_occurrence::RawIdentityConflict {
            reason: format!(
                "legacy raw row {row_id} has no trusted host provenance and cannot be rekeyed"
            ),
        }
        .into());
    }
    Ok(RekeyReport::default())
}
