use rusqlite::Row;

#[derive(Debug)]
#[allow(dead_code)]
pub struct PendingObservation {
    pub id: i64,
    pub host: String,
    pub session_id: String,
    pub project: String,
    pub tool_name: String,
    pub tool_input: Option<String>,
    pub tool_response: Option<String>,
    pub cwd: Option<String>,
    pub created_at_epoch: i64,
    pub updated_at_epoch: i64,
    pub status: String,
    pub attempt_count: i64,
    pub next_retry_epoch: Option<i64>,
    pub last_error: Option<String>,
}

impl PendingObservation {
    pub(super) fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            host: row.get(1)?,
            session_id: row.get(2)?,
            project: row.get(3)?,
            tool_name: row.get(4)?,
            tool_input: row.get(5)?,
            tool_response: row.get(6)?,
            cwd: row.get(7)?,
            created_at_epoch: row.get(8)?,
            updated_at_epoch: row.get(9)?,
            status: row.get(10)?,
            attempt_count: row.get(11)?,
            next_retry_epoch: row.get(12)?,
            last_error: row.get(13)?,
        })
    }
}
