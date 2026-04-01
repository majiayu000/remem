use rusqlite::Row;

#[derive(Debug, Clone)]
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
