use anyhow::Result;
use rusqlite::OptionalExtension;

pub(super) fn context_injections_has_data_version(conn: &rusqlite::Connection) -> Result<bool> {
    conn.query_row(
        "SELECT 1
         FROM pragma_table_info('context_injections')
         WHERE name = 'data_version'
         LIMIT 1",
        [],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .map(|value| value.is_some())
    .map_err(Into::into)
}
