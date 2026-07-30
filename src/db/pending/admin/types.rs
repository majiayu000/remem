use rusqlite::Row;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct FailedPendingRow {
    pub id: i64,
    pub session_id: String,
    pub project: String,
    pub tool_name: String,
    pub attempt_count: i64,
    pub updated_at_epoch: i64,
    pub last_error: Option<String>,
}

impl FailedPendingRow {
    pub(super) fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            session_id: row.get(1)?,
            project: row.get(2)?,
            tool_name: row.get(3)?,
            attempt_count: row.get(4)?,
            updated_at_epoch: row.get(5)?,
            last_error: row.get(6)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AdminRequiredArchivedLegacyPendingRow {
    pub id: i64,
    pub host: String,
    pub failure_class: Option<String>,
    pub archived_at_epoch: i64,
}

impl AdminRequiredArchivedLegacyPendingRow {
    pub(super) fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            host: row.get(1)?,
            failure_class: row.get(2)?,
            archived_at_epoch: row.get(3)?,
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ArchivedTransientLegacyPendingStats {
    pub(crate) due: usize,
    pub(crate) deferred: usize,
    pub(crate) earliest_deferred_retry_epoch: Option<i64>,
}
