use anyhow::Result;
use rusqlite::Connection;

use crate::memory::Memory;

pub(super) fn annotate_memories_with_temporal_facts_for_query(
    conn: &Connection,
    memories: &mut [Memory],
    query: Option<&str>,
    project: Option<&str>,
) -> Result<()> {
    crate::retrieval::temporal::annotate_memories_with_fact_labels(conn, memories, query, project)
}
