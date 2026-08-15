use anyhow::Result;
use rusqlite::Connection;

use crate::memory::Memory;

pub(super) fn retrieve(
    conn: &Connection,
    project: &str,
    prompt: &str,
    branch: Option<&str>,
    excluded_types: &[&str],
    target: i64,
    as_of_epoch: i64,
) -> Result<Vec<Memory>> {
    super::g2_backfill::fetch_bounded_ranked(
        conn,
        target as usize,
        target,
        |limit| {
            super::hybrid_context::query_hybrid_context_memories(
                conn,
                project,
                prompt,
                branch,
                excluded_types,
                limit,
                true,
            )
        },
        |memory| memory.id,
        as_of_epoch,
    )
}
