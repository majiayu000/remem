use rusqlite::Row;

#[derive(Debug)]
#[allow(dead_code)]
pub struct PendingObservation {
    pub id: i64,
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
            session_id: row.get(1)?,
            project: row.get(2)?,
            tool_name: row.get(3)?,
            tool_input: row.get(4)?,
            tool_response: row.get(5)?,
            cwd: row.get(6)?,
            created_at_epoch: row.get(7)?,
            updated_at_epoch: row.get(8)?,
            status: row.get(9)?,
            attempt_count: row.get(10)?,
            next_retry_epoch: row.get(11)?,
            last_error: row.get(12)?,
        })
    }
}
