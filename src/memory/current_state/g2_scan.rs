use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::memory::{self, Memory};

use super::{
    memory_is_current_context_eligible, prefixed_memory_cols, ref_parts_from_edge_row,
    CurrentStateMemoryRefParts, HISTORY_LIMIT,
};

const PAGE_SIZE: i64 = 100;
const MAX_SCAN_PAGES: usize = 16;

pub(super) fn load_active_state_key_rivals(
    conn: &Connection,
    state_key_id: i64,
    current_memory_id: i64,
    now_epoch: i64,
) -> Result<Vec<CurrentStateMemoryRefParts>> {
    let sql = format!(
        "SELECT {}, e.edge_type, e.reason, e.evidence_event_ids,
                e.source_candidate_id, e.source_operation_id
         FROM memories m
         LEFT JOIN memory_edges e ON e.id = (
             SELECT ce.id FROM memory_edges ce
             WHERE ce.edge_type = 'conflicts' AND ce.state_key_id = ?1
               AND ((ce.from_memory_id = m.id AND ce.to_memory_id = ?2)
                    OR (ce.from_memory_id = ?2 AND ce.to_memory_id = m.id))
             ORDER BY ce.created_at_epoch DESC, ce.id DESC LIMIT 1)
         WHERE m.state_key_id = ?1 AND m.id <> ?2 AND m.status = 'active'
           AND COALESCE(m.valid_from_epoch, m.created_at_epoch) <= ?3
           AND (m.valid_to_epoch IS NULL OR m.valid_to_epoch > ?3)
           AND (m.expires_at_epoch IS NULL OR m.expires_at_epoch > ?3)
           AND {}
         ORDER BY m.updated_at_epoch DESC, m.id DESC LIMIT ?4 OFFSET ?5",
        prefixed_memory_cols("m"),
        memory::suppression::memory_policy_filter_sql("m"),
    );
    let mut admitted = Vec::new();
    let mut offset = 0_i64;
    for page_index in 0..MAX_SCAN_PAGES {
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(
            rusqlite::params![
                state_key_id,
                current_memory_id,
                now_epoch,
                PAGE_SIZE,
                offset
            ],
            |row| {
                let memory = memory::map_memory_row_pub(row)?;
                ref_parts_from_edge_row(memory, row, 13)
            },
        )?;
        let page =
            crate::db::query::collect_rows(rows).context("load active current-state rivals")?;
        let page_len = page.len();
        for parts in page {
            if memory_is_current_context_eligible(conn, parts.memory.id, now_epoch)? {
                admitted.push(parts);
                if admitted.len() == HISTORY_LIMIT as usize {
                    break;
                }
            }
        }
        if admitted.len() == HISTORY_LIMIT as usize {
            break;
        }
        if page_len < PAGE_SIZE as usize {
            break;
        }
        if page_index + 1 == MAX_SCAN_PAGES {
            anyhow::bail!(
                "current-state rival G2 scan budget exhausted after {} rows",
                PAGE_SIZE * MAX_SCAN_PAGES as i64
            );
        }
        offset = offset
            .checked_add(PAGE_SIZE)
            .context("current-state rival offset overflow")?;
    }
    Ok(admitted)
}

pub(super) fn load_memories_as_of(
    conn: &Connection,
    state_key_id: i64,
    as_of_epoch: i64,
) -> Result<Vec<Memory>> {
    let sql = format!(
        "SELECT {} FROM memories
         WHERE state_key_id = ?1
           AND (status = 'active'
                OR (status = 'stale' AND (valid_to_epoch IS NOT NULL OR updated_at_epoch > ?2))
                OR (status = 'archived' AND valid_to_epoch IS NOT NULL))
           AND COALESCE(valid_from_epoch, created_at_epoch) <= ?2
           AND (valid_to_epoch IS NULL OR valid_to_epoch > ?2)
           AND (status <> 'active' OR updated_at_epoch <= ?2)
           AND (expires_at_epoch IS NULL OR expires_at_epoch > ?2)
           AND {}
         ORDER BY COALESCE(valid_from_epoch, created_at_epoch) DESC,
                  updated_at_epoch DESC, id DESC LIMIT ?3 OFFSET ?4",
        memory::MEMORY_COLS,
        memory::suppression::memory_policy_filter_sql("memories"),
    );
    let mut admitted = Vec::new();
    let mut offset = 0_i64;
    for page_index in 0..MAX_SCAN_PAGES {
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(
            rusqlite::params![state_key_id, as_of_epoch, PAGE_SIZE, offset],
            memory::map_memory_row_pub,
        )?;
        let page =
            crate::db::query::collect_rows(rows).context("load current-state memories as-of")?;
        let page_len = page.len();
        for memory in page {
            if crate::truth::admit_for_historical_context(conn, memory.id, as_of_epoch)?
                .current_context_eligible
            {
                admitted.push(memory);
                if admitted.len() == HISTORY_LIMIT as usize {
                    break;
                }
            }
        }
        if admitted.len() == HISTORY_LIMIT as usize {
            break;
        }
        if page_len < PAGE_SIZE as usize {
            break;
        }
        if page_index + 1 == MAX_SCAN_PAGES {
            anyhow::bail!(
                "current-state history G2 scan budget exhausted after {} rows",
                PAGE_SIZE * MAX_SCAN_PAGES as i64
            );
        }
        offset = offset
            .checked_add(PAGE_SIZE)
            .context("current-state as-of offset overflow")?;
    }
    Ok(admitted)
}
