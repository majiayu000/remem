//! Canonical preference reinforcement state (SP671-T3).
//!
//! Active `preference` memories previously carried no persisted reinforcement
//! count, so the rule compiler had nothing authoritative to gate on. This
//! module wires the memory-candidate apply path to persist a per-preference
//! reinforcement count into `memory_preference_reinforcements` and to record
//! whether the preference text deterministically yields a v1 predicate
//! (`machine_checkable`). Counts carry forward only when the old and new text
//! derive the same safe v1 predicate; accepted exact repeats increment in place.

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::rules::PreferencePredicate;

/// Persist the reinforcement count for a freshly written preference memory.
///
/// `superseded_ids` are the prior preference memories this write replaces (from
/// the same apply pass). Counts for the same safe predicate are summed forward;
/// opposite or ambiguous preferences start a new count. Superseded state rows
/// are removed so only the current authoritative memory carries the count.
pub(crate) fn persist_preference_reinforcement(
    conn: &Connection,
    new_memory_id: i64,
    superseded_ids: &[i64],
    text: &str,
    risk_class: &str,
    source_evidence: Option<&str>,
    now: i64,
) -> Result<i64> {
    let new_predicates = preference_predicates(text);
    let mut carried = 0i64;
    let mut seen = std::collections::HashSet::with_capacity(superseded_ids.len());
    for id in superseded_ids.iter().filter(|id| seen.insert(**id)) {
        if *id == new_memory_id {
            continue;
        }
        let prior: Option<(i64, String)> = conn
            .query_row(
                "SELECT r.reinforcement_count, m.content
                 FROM memory_preference_reinforcements r
                 JOIN memories m ON m.id = r.memory_id
                 WHERE r.memory_id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((count, prior_text)) = prior {
            if !new_predicates.is_empty() && preference_predicates(&prior_text) == new_predicates {
                carried = carried
                    .checked_add(count)
                    .context("preference reinforcement count overflow")?;
                transfer_rule_overrides(conn, *id, new_memory_id)?;
            }
        }
        conn.execute(
            "DELETE FROM memory_preference_reinforcements WHERE memory_id = ?1",
            [id],
        )?;
    }
    let new_count = carried
        .checked_add(1)
        .context("preference reinforcement count overflow")?;
    let machine_checkable = i64::from(!new_predicates.is_empty());
    conn.execute(
        "INSERT INTO memory_preference_reinforcements
         (memory_id, reinforcement_count, source_evidence,
          last_reinforced_at_epoch, created_at_epoch, updated_at_epoch,
          machine_checkable, risk_class)
         VALUES (?1, ?2, ?3, ?4, ?4, ?4, ?5, ?6)
         ON CONFLICT(memory_id) DO UPDATE SET
           reinforcement_count = excluded.reinforcement_count,
           source_evidence = excluded.source_evidence,
           last_reinforced_at_epoch = excluded.last_reinforced_at_epoch,
           updated_at_epoch = excluded.updated_at_epoch,
           machine_checkable = excluded.machine_checkable,
           risk_class = excluded.risk_class",
        params![
            new_memory_id,
            new_count,
            source_evidence,
            now,
            machine_checkable,
            risk_class
        ],
    )?;
    Ok(new_count)
}

/// Count an accepted duplicate correction against its existing authoritative
/// preference instead of discarding the reinforcement signal with the noop.
pub(crate) fn reinforce_existing_preference(
    conn: &Connection,
    memory_id: i64,
    text: &str,
    risk_class: &str,
    source_evidence: Option<&str>,
    now: i64,
) -> Result<i64> {
    let machine_checkable = i64::from(!preference_predicates(text).is_empty());
    conn.execute(
        "INSERT INTO memory_preference_reinforcements
         (memory_id, reinforcement_count, source_evidence,
          last_reinforced_at_epoch, created_at_epoch, updated_at_epoch,
          machine_checkable, risk_class)
         VALUES (?1, 1, ?2, ?3, ?3, ?3, ?4, ?5)
         ON CONFLICT(memory_id) DO UPDATE SET
           reinforcement_count = memory_preference_reinforcements.reinforcement_count + 1,
           source_evidence = excluded.source_evidence,
           last_reinforced_at_epoch = excluded.last_reinforced_at_epoch,
           updated_at_epoch = excluded.updated_at_epoch,
           machine_checkable = excluded.machine_checkable,
           risk_class = excluded.risk_class",
        params![
            memory_id,
            source_evidence,
            now,
            machine_checkable,
            risk_class
        ],
    )?;
    conn.query_row(
        "SELECT reinforcement_count
         FROM memory_preference_reinforcements
         WHERE memory_id = ?1",
        [memory_id],
        |row| row.get(0),
    )
    .context("load reinforced existing preference count")
}

fn preference_predicates(text: &str) -> Vec<PreferencePredicate> {
    crate::rules::classify_preference_predicates(text)
        .into_iter()
        .map(|classification| classification.predicate)
        .collect()
}

#[derive(Debug)]
struct StoredOverride {
    id: i64,
    project: String,
    rule_id: String,
    disabled: i64,
    action_override: Option<String>,
    reason: Option<String>,
    updated_by: String,
    updated_at_epoch: i64,
}

fn transfer_rule_overrides(
    conn: &Connection,
    old_memory_id: i64,
    new_memory_id: i64,
) -> Result<()> {
    let old_prefix = format!("pref-{old_memory_id}-");
    let mut stmt = conn.prepare(
        "SELECT id, project, rule_id, disabled, action_override, reason,
                updated_by, updated_at_epoch
         FROM preference_rule_overrides
         WHERE source_memory_id = ?1 OR rule_id LIKE ?2
         ORDER BY updated_at_epoch ASC, id ASC",
    )?;
    let rows = stmt.query_map(params![old_memory_id, format!("{old_prefix}%")], |row| {
        Ok(StoredOverride {
            id: row.get(0)?,
            project: row.get(1)?,
            rule_id: row.get(2)?,
            disabled: row.get(3)?,
            action_override: row.get(4)?,
            reason: row.get(5)?,
            updated_by: row.get(6)?,
            updated_at_epoch: row.get(7)?,
        })
    })?;
    let overrides = crate::db::query::collect_rows(rows)?;
    drop(stmt);

    for stored in overrides {
        let suffix = stored.rule_id.strip_prefix(&old_prefix).with_context(|| {
            format!(
                "override {} references preference memory {} but has no transferable rule suffix",
                stored.rule_id, old_memory_id
            )
        })?;
        if suffix.is_empty() || !suffix.chars().all(|ch| ch.is_ascii_digit()) {
            bail!(
                "override {} has an invalid generated rule suffix",
                stored.rule_id
            );
        }
        let new_rule_id = format!("pref-{new_memory_id}-{suffix}");
        conn.execute(
            "INSERT INTO preference_rule_overrides
             (project, rule_id, source_memory_id, disabled, action_override, reason,
              updated_by, updated_at_epoch)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(project, rule_id) DO UPDATE SET
               source_memory_id = excluded.source_memory_id,
               disabled = excluded.disabled,
               action_override = excluded.action_override,
               reason = excluded.reason,
               updated_by = excluded.updated_by,
               updated_at_epoch = excluded.updated_at_epoch
             WHERE excluded.updated_at_epoch >= preference_rule_overrides.updated_at_epoch",
            params![
                stored.project,
                new_rule_id,
                new_memory_id,
                stored.disabled,
                stored.action_override,
                stored.reason,
                stored.updated_by,
                stored.updated_at_epoch,
            ],
        )?;
        conn.execute(
            "DELETE FROM preference_rule_overrides WHERE id = ?1",
            [stored.id],
        )?;
    }
    Ok(())
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

        let count = persist_preference_reinforcement(
            &conn,
            1,
            &[],
            "Use bun, not npm",
            "low",
            Some("[1]"),
            100,
        )?;
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

        persist_preference_reinforcement(
            &conn,
            1,
            &[],
            "Use bun, not npm",
            "low",
            Some("[1]"),
            100,
        )?;
        let second = persist_preference_reinforcement(
            &conn,
            2,
            &[1],
            "Prefer bun over npm",
            "low",
            Some("[2]"),
            200,
        )?;
        assert_eq!(second, 2);
        let third = persist_preference_reinforcement(
            &conn,
            3,
            &[2],
            "Use bun instead of npm",
            "low",
            Some("[3]"),
            300,
        )?;
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

        persist_preference_reinforcement(&conn, 1, &[], "I like clean code", "low", None, 100)?;
        let checkable: i64 = conn.query_row(
            "SELECT machine_checkable FROM memory_preference_reinforcements WHERE memory_id = 1",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(checkable, 0);
        Ok(())
    }

    #[test]
    fn opposite_preference_does_not_inherit_reinforcement() -> Result<()> {
        let _dir = ScopedTestDataDir::new("pref-reinforce-opposite");
        let conn = db::open_db()?;
        insert_preference(&conn, 1, "Use bun, not npm")?;
        insert_preference(&conn, 2, "Use npm, not yarn")?;

        persist_preference_reinforcement(&conn, 1, &[], "Use bun, not npm", "low", None, 100)?;
        reinforce_existing_preference(&conn, 1, "Use bun, not npm", "low", None, 150)?;
        reinforce_existing_preference(&conn, 1, "Use bun, not npm", "low", None, 175)?;

        let count = persist_preference_reinforcement(
            &conn,
            2,
            &[1],
            "Use npm, not yarn",
            "low",
            None,
            200,
        )?;
        assert_eq!(count, 1);
        Ok(())
    }

    #[test]
    fn override_follows_replacement_memory() -> Result<()> {
        let _dir = ScopedTestDataDir::new("pref-reinforce-override");
        let conn = db::open_db()?;
        insert_preference(&conn, 1, "Use bun, not npm")?;
        insert_preference(&conn, 2, "Use bun, not npm")?;
        persist_preference_reinforcement(&conn, 1, &[], "Use bun, not npm", "low", None, 100)?;
        conn.execute(
            "INSERT INTO preference_rule_overrides
             (project, rule_id, source_memory_id, disabled, action_override, updated_at_epoch)
             VALUES ('/tmp/remem', 'pref-1-1', 1, 1, 'block', 150)",
            [],
        )?;

        persist_preference_reinforcement(&conn, 2, &[1], "Use bun, not npm", "low", None, 200)?;

        let row: (String, Option<i64>, i64, Option<String>) = conn.query_row(
            "SELECT rule_id, source_memory_id, disabled, action_override
             FROM preference_rule_overrides WHERE project = '/tmp/remem'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        assert_eq!(row.0, "pref-2-1");
        assert_eq!(row.1, Some(2));
        assert_eq!(row.2, 1);
        assert_eq!(row.3.as_deref(), Some("block"));
        Ok(())
    }
}
