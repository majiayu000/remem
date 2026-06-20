use anyhow::Result;
use rusqlite::Connection;

pub(super) fn search_without_query(
    conn: &Connection,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
    branch: Option<&str>,
    include_suppressed: bool,
) -> Result<Vec<crate::memory::Memory>> {
    let project_name = project.unwrap_or("");
    if project_name.is_empty() {
        Ok(vec![])
    } else {
        crate::memory::list_memories_with_suppressed_policy(
            conn,
            project_name,
            memory_type,
            limit,
            offset,
            include_stale,
            branch,
            include_suppressed,
        )
    }
}
