//! Canonical preference reinforcement state (SP671-T3).
//!
//! Active `preference` memories previously carried no persisted reinforcement
//! count, so the rule compiler had nothing authoritative to gate on. This
//! module wires the memory-candidate apply path to persist a per-preference
//! reinforcement count into `memory_preference_reinforcements` and to record
//! whether the preference text deterministically yields a v1 predicate
//! (`machine_checkable`). Counts carry forward across supersession so that
//! reinforcing an existing preference increments its canonical count.

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

/// Persist the reinforcement count for a freshly written preference memory.
///
/// `superseded_ids` are the prior preference memories this write replaces (from
/// the same apply pass). Their counts are summed forward and their rows removed
/// so the count follows the current authoritative memory. Returns the new
/// canonical reinforcement count.
pub(crate) fn persist_preference_reinforcement(
    conn: &Connection,
    new_memory_id: i64,
    superseded_ids: &[i64],
    text: &str,
    now: i64,
) -> Result<i64> {
    let mut carried = 0i64;
    for id in superseded_ids {
        if *id == new_memory_id {
            continue;
        }
        let prior: Option<i64> = conn
            .query_row(
                "SELECT reinforcement_count
                 FROM memory_preference_reinforcements
                 WHERE memory_id = ?1",
                [id],
                |row| row.get(0),
            )
            .optional()?;
        carried += prior.unwrap_or(0);
        conn.execute(
            "DELETE FROM memory_preference_reinforcements WHERE memory_id = ?1",
            [id],
        )?;
    }
    let new_count = carried + 1;
    let machine_checkable = i64::from(crate::rules::classify_preference_predicate(text).is_some());
    conn.execute(
        "INSERT INTO memory_preference_reinforcements
         (memory_id, reinforcement_count, source_evidence,
          last_reinforced_at_epoch, created_at_epoch, updated_at_epoch, machine_checkable)
         VALUES (?1, ?2, NULL, ?3, ?3, ?3, ?4)
         ON CONFLICT(memory_id) DO UPDATE SET
           reinforcement_count = excluded.reinforcement_count,
           last_reinforced_at_epoch = excluded.last_reinforced_at_epoch,
           updated_at_epoch = excluded.updated_at_epoch,
           machine_checkable = excluded.machine_checkable",
        params![new_memory_id, new_count, now, machine_checkable],
    )?;
    Ok(new_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{self, test_support::ScopedTestDataDir};

    fn insert_preference(conn: &Connection, id: i64, content: &str) -> Result<()> {
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch,
              status, scope, owner_scope, owner_key)
             VALUES (?1, '/tmp/remem', 'pref', ?2, 'preference', 1, 1, 'active', 'project', 'repo', '/tmp/remem')",
            params![id, content],
        )?;
        Ok(())
    }

    #[test]
    fn first_reinforcement_starts_at_one_and_flags_machine_checkable() -> Result<()> {
        let _dir = ScopedTestDataDir::new("pref-reinforce-first");
        let conn = db::open_db()?;
        insert_preference(&conn, 1, "Use bun, not npm")?;

        let count = persist_preference_reinforcement(&conn, 1, &[], "Use bun, not npm", 100)?;
        assert_eq!(count, 1);

        let (stored, checkable): (i64, i64) = conn.query_row(
            "SELECT reinforcement_count, machine_checkable
             FROM memory_preference_reinforcements WHERE memory_id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(stored, 1);
        assert_eq!(checkable, 1);
        Ok(())
    }

    #[test]
    fn reinforcement_carries_count_forward_across_supersession() -> Result<()> {
        let _dir = ScopedTestDataDir::new("pref-reinforce-carry");
        let conn = db::open_db()?;
        insert_preference(&conn, 1, "Use bun, not npm")?;
        insert_preference(&conn, 2, "Use bun, not npm")?;
        insert_preference(&conn, 3, "Use bun, not npm")?;

        persist_preference_reinforcement(&conn, 1, &[], "Use bun, not npm", 100)?;
        let second = persist_preference_reinforcement(&conn, 2, &[1], "Use bun, not npm", 200)?;
        assert_eq!(second, 2);
        let third = persist_preference_reinforcement(&conn, 3, &[2], "Use bun, not npm", 300)?;
        assert_eq!(third, 3);

        // Superseded rows are removed; only the current memory carries state.
        let live: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_preference_reinforcements",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(live, 1);
        let current: i64 = conn.query_row(
            "SELECT reinforcement_count FROM memory_preference_reinforcements WHERE memory_id = 3",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(current, 3);
        Ok(())
    }

    #[test]
    fn ambiguous_preference_is_not_machine_checkable() -> Result<()> {
        let _dir = ScopedTestDataDir::new("pref-reinforce-ambiguous");
        let conn = db::open_db()?;
        insert_preference(&conn, 1, "I like clean code")?;

        persist_preference_reinforcement(&conn, 1, &[], "I like clean code", 100)?;
        let checkable: i64 = conn.query_row(
            "SELECT machine_checkable FROM memory_preference_reinforcements WHERE memory_id = 1",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(checkable, 0);
        Ok(())
    }
}
