use anyhow::{bail, ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

pub(super) const EPHEMERAL_EVENT_TYPES: [&str; 7] = [
    "file_edit",
    "file_create",
    "bash",
    "search",
    "agent",
    "tool_result",
    "cursor_tool_failure",
];

fn event_retention_class(event_type: &str) -> &'static str {
    if EPHEMERAL_EVENT_TYPES.contains(&event_type) {
        "ephemeral"
    } else {
        "audit"
    }
}

pub fn insert_event(
    conn: &Connection,
    session_id: &str,
    project: &str,
    event_type: &str,
    summary: &str,
    detail: Option<&str>,
    files: Option<&str>,
    exit_code: Option<i32>,
) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO events \
         (session_id, project, event_type, summary, detail, files, exit_code,
          created_at_epoch, retention_class) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            session_id,
            project,
            event_type,
            summary,
            detail,
            files,
            exit_code,
            now,
            event_retention_class(event_type)
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn insert_event_for_capture(
    conn: &Connection,
    captured_event_id: i64,
    session_id: &str,
    project: &str,
    event_type: &str,
    summary: &str,
    detail: Option<&str>,
    files: Option<&str>,
    exit_code: Option<i32>,
) -> Result<i64> {
    ensure!(captured_event_id > 0, "captured event id must be positive");
    let retention_class = event_retention_class(event_type);
    let now = chrono::Utc::now().timestamp();
    let inserted = conn.execute(
        "INSERT INTO events \
         (session_id, project, event_type, summary, detail, files, exit_code,
          created_at_epoch, retention_class, captured_event_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
         ON CONFLICT(captured_event_id) WHERE captured_event_id IS NOT NULL \
         DO NOTHING",
        params![
            session_id,
            project,
            event_type,
            summary,
            detail,
            files,
            exit_code,
            now,
            retention_class,
            captured_event_id,
        ],
    )?;
    if inserted == 1 {
        return Ok(conn.last_insert_rowid());
    }

    let existing = event_projection(conn, captured_event_id)?.with_context(|| {
        format!(
            "captured event projection conflict without an existing row: captured_event_id={captured_event_id}"
        )
    })?;
    if existing.matches(
        session_id,
        project,
        event_type,
        summary,
        detail,
        files,
        exit_code,
        retention_class,
    ) {
        return Ok(existing.id);
    }
    bail!("captured event projection payload mismatch: captured_event_id={captured_event_id}")
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn replace_event_for_capture(
    conn: &Connection,
    captured_event_id: i64,
    session_id: &str,
    project: &str,
    event_type: &str,
    summary: &str,
    detail: Option<&str>,
    files: Option<&str>,
    exit_code: Option<i32>,
) -> Result<i64> {
    ensure!(captured_event_id > 0, "captured event id must be positive");
    let retention_class = event_retention_class(event_type);
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO events \
         (session_id, project, event_type, summary, detail, files, exit_code,
          created_at_epoch, retention_class, captured_event_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
         ON CONFLICT(captured_event_id) WHERE captured_event_id IS NOT NULL \
         DO UPDATE SET session_id = excluded.session_id,
                       project = excluded.project,
                       event_type = excluded.event_type,
                       summary = excluded.summary,
                       detail = excluded.detail,
                       files = excluded.files,
                       exit_code = excluded.exit_code,
                       retention_class = excluded.retention_class",
        params![
            session_id,
            project,
            event_type,
            summary,
            detail,
            files,
            exit_code,
            now,
            retention_class,
            captured_event_id,
        ],
    )?;
    event_projection(conn, captured_event_id)?
        .map(|event| event.id)
        .with_context(|| {
            format!(
                "captured event projection missing after replace: captured_event_id={captured_event_id}"
            )
        })
}

struct EventProjection {
    id: i64,
    session_id: String,
    project: String,
    event_type: String,
    summary: String,
    detail: Option<String>,
    files: Option<String>,
    exit_code: Option<i32>,
    retention_class: String,
}

impl EventProjection {
    #[allow(clippy::too_many_arguments)]
    fn matches(
        &self,
        session_id: &str,
        project: &str,
        event_type: &str,
        summary: &str,
        detail: Option<&str>,
        files: Option<&str>,
        exit_code: Option<i32>,
        retention_class: &str,
    ) -> bool {
        self.session_id == session_id
            && self.project == project
            && self.event_type == event_type
            && self.summary == summary
            && self.detail.as_deref() == detail
            && self.files.as_deref() == files
            && self.exit_code == exit_code
            && self.retention_class == retention_class
    }
}

fn event_projection(conn: &Connection, captured_event_id: i64) -> Result<Option<EventProjection>> {
    Ok(conn
        .query_row(
            "SELECT id, session_id, project, event_type, summary, detail, files,
                    exit_code, retention_class
             FROM events
             WHERE captured_event_id = ?1",
            [captured_event_id],
            |row| {
                Ok(EventProjection {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    project: row.get(2)?,
                    event_type: row.get(3)?,
                    summary: row.get(4)?,
                    detail: row.get(5)?,
                    files: row.get(6)?,
                    exit_code: row.get(7)?,
                    retention_class: row.get(8)?,
                })
            },
        )
        .optional()?)
}
