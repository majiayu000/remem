//! Canonical preference reinforcement state (SP671-T3).
//!
//! Active `preference` memories previously carried no persisted reinforcement
//! count, so the rule compiler had nothing authoritative to gate on. This
//! module wires the memory-candidate apply path to persist a per-preference
//! reinforcement count into `memory_preference_reinforcements` and to record
//! whether the preference text deterministically yields a v1 predicate
//! (`machine_checkable`). Counts carry forward only when the old and new text
//! derive the same safe v1 predicate; accepted exact repeats increment in place.

use std::collections::{BTreeSet, HashSet};

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::rules::PreferencePredicate;

mod overrides;

pub(crate) use overrides::reconcile_preference_project_change;
use overrides::transfer_rule_overrides;

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
    let incoming_evidence = parse_source_evidence(source_evidence)?;
    let mut carried = ReinforcementAggregate::default();
    let mut seen = HashSet::with_capacity(superseded_ids.len());
    for id in superseded_ids.iter().filter(|id| seen.insert(**id)) {
        if *id == new_memory_id {
            continue;
        }
        let (prior_text, prior_state): (String, Option<(i64, Option<String>, i64, i64, String)>) =
            conn.query_row(
                "SELECT m.content, r.reinforcement_count, r.source_evidence,
                    r.last_reinforced_at_epoch, r.created_at_epoch, r.risk_class
             FROM memories m
             LEFT JOIN memory_preference_reinforcements r ON r.memory_id = m.id
             WHERE m.id = ?1",
                [id],
                |row| {
                    let count: Option<i64> = row.get(1)?;
                    let state = if let Some(count) = count {
                        Some((count, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?))
                    } else {
                        None
                    };
                    Ok((row.get(0)?, state))
                },
            )?;
        if !new_predicates.is_empty() && preference_predicates(&prior_text) == new_predicates {
            if let Some((count, evidence, last, created, prior_risk)) = prior_state {
                carried.absorb(ReinforcementState {
                    count,
                    evidence: parse_source_evidence(evidence.as_deref())?,
                    last_reinforced_at_epoch: last,
                    created_at_epoch: created,
                    risk_class: prior_risk,
                })?;
            }
        }
        transfer_rule_overrides(conn, *id, new_memory_id, &prior_text, text)?;
        conn.execute(
            "DELETE FROM memory_preference_reinforcements WHERE memory_id = ?1",
            [id],
        )?;
    }
    let mut state = carried.finish().unwrap_or_else(|| ReinforcementState {
        count: 1,
        evidence: BTreeSet::new(),
        last_reinforced_at_epoch: now,
        created_at_epoch: now,
        risk_class: risk_class.to_string(),
    });
    if !state.evidence.is_empty() && has_novel_evidence(&state.evidence, &incoming_evidence) {
        state.count = state
            .count
            .checked_add(1)
            .context("preference reinforcement count overflow")?;
        state.last_reinforced_at_epoch = now;
    }
    state.evidence.extend(incoming_evidence);
    state.risk_class = restrictive_risk_class(&state.risk_class, risk_class)?.to_string();
    write_reinforcement_state(conn, new_memory_id, &state, !new_predicates.is_empty(), now)?;
    Ok(state.count)
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
    let incoming_evidence = parse_source_evidence(source_evidence)?;
    let stored: Option<(i64, Option<String>, i64, i64, String)> = conn
        .query_row(
            "SELECT reinforcement_count, source_evidence, last_reinforced_at_epoch,
                    created_at_epoch, risk_class
             FROM memory_preference_reinforcements
             WHERE memory_id = ?1",
            [memory_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()?;
    let had_stored_state = stored.is_some();
    let mut state = if let Some((count, evidence, last, created, stored_risk)) = stored {
        ReinforcementState {
            count,
            evidence: parse_source_evidence(evidence.as_deref())?,
            last_reinforced_at_epoch: last,
            created_at_epoch: created,
            risk_class: stored_risk,
        }
    } else {
        ReinforcementState {
            count: 1,
            evidence: BTreeSet::new(),
            last_reinforced_at_epoch: now,
            created_at_epoch: now,
            risk_class: risk_class.to_string(),
        }
    };
    if had_stored_state && has_novel_evidence(&state.evidence, &incoming_evidence) {
        state.count = state
            .count
            .checked_add(1)
            .context("preference reinforcement count overflow")?;
        state.last_reinforced_at_epoch = now;
    }
    state.evidence.extend(incoming_evidence);
    state.risk_class = restrictive_risk_class(&state.risk_class, risk_class)?.to_string();
    write_reinforcement_state(conn, memory_id, &state, machine_checkable != 0, now)?;
    Ok(state.count)
}

/// Remove candidate-derived rule state when an in-place write changes the
/// deterministic predicate. Exact text and same-predicate rewrites retain the
/// existing evidence and user override; unrelated or opposing text does not.
pub(crate) fn reconcile_in_place_preference_update(
    conn: &Connection,
    memory_id: i64,
    previous_text: &str,
    current_text: &str,
) -> Result<bool> {
    let previous = preference_predicates(previous_text);
    let current = preference_predicates(current_text);
    let compatible = crate::memory::operation::same_memory_text(previous_text, current_text)
        || (!current.is_empty() && previous == current);
    if compatible {
        return Ok(false);
    }
    transfer_rule_overrides(conn, memory_id, memory_id, previous_text, current_text)?;
    conn.execute(
        "DELETE FROM memory_preference_reinforcements WHERE memory_id = ?1",
        [memory_id],
    )?;
    conn.execute(
        "UPDATE memories SET source_candidate_id = NULL WHERE id = ?1",
        [memory_id],
    )?;
    Ok(true)
}

/// Reconcile rule state while cleanup selects one canonical preference and
/// stales the rest. Cleanup itself is not new evidence, so it only combines
/// disjoint, same-predicate evidence and never increments the count.
pub(crate) fn reconcile_cleanup_preference(
    conn: &Connection,
    current_memory_id: i64,
    stale_memory_ids: &[i64],
    final_text: &str,
    now: i64,
) -> Result<()> {
    let final_predicates = preference_predicates(final_text);
    let current_text: String = conn.query_row(
        "SELECT content FROM memories WHERE id = ?1",
        [current_memory_id],
        |row| row.get(0),
    )?;
    let current_predicates = preference_predicates(&current_text);
    let current_compatible = crate::memory::operation::same_memory_text(&current_text, final_text)
        || (!final_predicates.is_empty() && current_predicates == final_predicates);
    let current_override_compatible = !final_predicates.is_empty()
        && final_predicates
            .iter()
            .any(|predicate| current_predicates.contains(predicate));
    let mut aggregate = ReinforcementAggregate::default();
    let mut stale_ids = stale_memory_ids.to_vec();
    stale_ids.sort_unstable();
    stale_ids.dedup();
    stale_ids.retain(|memory_id| *memory_id != current_memory_id);

    // Transfer the canonical row before compatible stale rows can write
    // overrides under its rule-id prefix. Otherwise a later canonical transfer
    // can reinterpret an already-remapped stale override using the canonical
    // row's old ordinal. Unrelated final predicates still discard stale
    // provenance rather than adopting it into an edited cleanup plan.
    for memory_id in std::iter::once(current_memory_id).chain(stale_ids) {
        let row: (String, Option<(i64, Option<String>, i64, i64, String)>) = conn.query_row(
            "SELECT m.content, r.reinforcement_count, r.source_evidence,
                        r.last_reinforced_at_epoch, r.created_at_epoch, r.risk_class
                 FROM memories m
                 LEFT JOIN memory_preference_reinforcements r ON r.memory_id = m.id
                 WHERE m.id = ?1",
            [memory_id],
            |row| {
                let count: Option<i64> = row.get(1)?;
                let state = if let Some(count) = count {
                    Some((count, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?))
                } else {
                    None
                };
                Ok((row.get(0)?, state))
            },
        )?;
        let same_predicate =
            !final_predicates.is_empty() && preference_predicates(&row.0) == final_predicates;
        if same_predicate {
            if let Some((count, evidence, last, created, risk_class)) = row.1 {
                aggregate.absorb(ReinforcementState {
                    count,
                    evidence: parse_source_evidence(evidence.as_deref())?,
                    last_reinforced_at_epoch: last,
                    created_at_epoch: created,
                    risk_class,
                })?;
            }
        }
        if memory_id != current_memory_id && !current_override_compatible {
            overrides::remove_rule_overrides(conn, memory_id)?;
        } else if memory_id != current_memory_id
            || !crate::memory::operation::same_memory_text(&row.0, final_text)
        {
            transfer_rule_overrides(conn, memory_id, current_memory_id, &row.0, final_text)?;
        }
        if memory_id != current_memory_id {
            conn.execute(
                "DELETE FROM memory_preference_reinforcements WHERE memory_id = ?1",
                [memory_id],
            )?;
        }
    }

    if !current_compatible {
        conn.execute(
            "UPDATE memories SET source_candidate_id = NULL WHERE id = ?1",
            [current_memory_id],
        )?;
    }
    if let Some(state) = aggregate
        .finish()
        .filter(|_| current_compatible && !final_predicates.is_empty())
    {
        write_reinforcement_state(conn, current_memory_id, &state, true, now)?;
    } else {
        conn.execute(
            "DELETE FROM memory_preference_reinforcements WHERE memory_id = ?1",
            [current_memory_id],
        )?;
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct ReinforcementState {
    count: i64,
    evidence: BTreeSet<i64>,
    last_reinforced_at_epoch: i64,
    created_at_epoch: i64,
    risk_class: String,
}

#[derive(Debug, Default)]
struct ReinforcementAggregate {
    state: Option<ReinforcementState>,
}

impl ReinforcementAggregate {
    fn absorb(&mut self, incoming: ReinforcementState) -> Result<()> {
        let Some(stored) = self.state.as_mut() else {
            self.state = Some(incoming);
            return Ok(());
        };
        let disjoint_known_evidence = !stored.evidence.is_empty()
            && !incoming.evidence.is_empty()
            && stored.evidence.is_disjoint(&incoming.evidence);
        stored.count = if disjoint_known_evidence {
            stored
                .count
                .checked_add(incoming.count)
                .context("preference reinforcement count overflow")?
        } else {
            stored.count.max(incoming.count)
        };
        stored.evidence.extend(incoming.evidence);
        stored.last_reinforced_at_epoch = stored
            .last_reinforced_at_epoch
            .max(incoming.last_reinforced_at_epoch);
        stored.created_at_epoch = stored.created_at_epoch.min(incoming.created_at_epoch);
        stored.risk_class =
            restrictive_risk_class(&stored.risk_class, &incoming.risk_class)?.to_string();
        Ok(())
    }

    fn finish(self) -> Option<ReinforcementState> {
        self.state
    }
}

fn parse_source_evidence(source_evidence: Option<&str>) -> Result<BTreeSet<i64>> {
    let Some(source_evidence) = source_evidence else {
        return Ok(BTreeSet::new());
    };
    let evidence: Vec<i64> = serde_json::from_str(source_evidence)
        .context("parse preference reinforcement source evidence")?;
    if evidence.iter().any(|id| *id <= 0) {
        bail!("preference reinforcement source evidence must contain positive event ids");
    }
    Ok(evidence.into_iter().collect())
}

fn has_novel_evidence(stored: &BTreeSet<i64>, incoming: &BTreeSet<i64>) -> bool {
    !incoming.is_empty() && incoming.iter().any(|id| !stored.contains(id))
}

fn restrictive_risk_class<'a>(left: &'a str, right: &'a str) -> Result<&'a str> {
    fn rank(value: &str) -> Option<u8> {
        match value {
            "low" => Some(0),
            "medium" => Some(1),
            "high" => Some(2),
            "unknown" => Some(3),
            _ => None,
        }
    }
    let left_rank = rank(left).with_context(|| format!("invalid preference risk class {left}"))?;
    let right_rank =
        rank(right).with_context(|| format!("invalid preference risk class {right}"))?;
    Ok(if left_rank >= right_rank { left } else { right })
}

fn write_reinforcement_state(
    conn: &Connection,
    memory_id: i64,
    state: &ReinforcementState,
    machine_checkable: bool,
    now: i64,
) -> Result<()> {
    let source_evidence = if state.evidence.is_empty() {
        None
    } else {
        Some(serde_json::to_string(
            &state.evidence.iter().copied().collect::<Vec<_>>(),
        )?)
    };
    conn.execute(
        "INSERT INTO memory_preference_reinforcements
         (memory_id, reinforcement_count, source_evidence,
          last_reinforced_at_epoch, created_at_epoch, updated_at_epoch,
          machine_checkable, risk_class)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(memory_id) DO UPDATE SET
           reinforcement_count = excluded.reinforcement_count,
           source_evidence = excluded.source_evidence,
           last_reinforced_at_epoch = excluded.last_reinforced_at_epoch,
           created_at_epoch = excluded.created_at_epoch,
           updated_at_epoch = excluded.updated_at_epoch,
           machine_checkable = excluded.machine_checkable,
           risk_class = excluded.risk_class",
        params![
            memory_id,
            state.count,
            source_evidence,
            state.last_reinforced_at_epoch,
            state.created_at_epoch,
            now,
            i64::from(machine_checkable),
            state.risk_class,
        ],
    )?;
    Ok(())
}

fn preference_predicates(text: &str) -> Vec<PreferencePredicate> {
    crate::rules::classify_preference_predicates(text)
        .into_iter()
        .map(|classification| classification.predicate)
        .collect()
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

    #[test]
    fn override_transfer_matches_predicate_identity_not_rule_ordinal() -> Result<()> {
        let _dir = ScopedTestDataDir::new("pref-reinforce-override-identity");
        let conn = db::open_db()?;
        let old_text = "Do not add AI-generated-by or Co-authored-by trailers to commits";
        let new_text = "Do not add Co-authored-by trailer to commits";
        insert_preference(&conn, 1, old_text)?;
        insert_preference(&conn, 2, new_text)?;
        persist_preference_reinforcement(&conn, 1, &[], old_text, "low", None, 100)?;
        conn.execute(
            "INSERT INTO preference_rule_overrides
             (project, rule_id, source_memory_id, disabled, action_override, reason,
              updated_at_epoch)
             VALUES ('/tmp/remem', 'pref-1-1', 1, 1, 'block', 'ai override', 150),
                    ('/tmp/remem', 'pref-1-2', 1, 0, 'block', 'coauthor override', 151)",
            [],
        )?;

        persist_preference_reinforcement(&conn, 2, &[1], new_text, "low", None, 200)?;

        let rows = conn
            .prepare(
                "SELECT rule_id, source_memory_id, disabled, reason
                 FROM preference_rule_overrides
                 ORDER BY rule_id",
            )?
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        assert_eq!(
            rows,
            vec![(
                "pref-2-1".to_string(),
                Some(2),
                0,
                Some("coauthor override".to_string())
            )]
        );
        Ok(())
    }

    #[test]
    fn cleanup_preserves_stale_override_when_final_predicates_are_current_subset() -> Result<()> {
        let _dir = ScopedTestDataDir::new("pref-cleanup-override-subset");
        let conn = db::open_db()?;
        let stale_text = "Do not add Co-authored-by trailer to commits";
        let current_text = "Do not add AI-generated-by or Co-authored-by trailers to commits";
        insert_preference(&conn, 1, stale_text)?;
        insert_preference(&conn, 2, current_text)?;
        conn.execute(
            "INSERT INTO preference_rule_overrides
             (project, rule_id, source_memory_id, disabled, action_override, reason,
              updated_at_epoch)
             VALUES ('/tmp/remem', 'pref-1-1', 1, 1, 'block',
                     'keep coauthor override', 150)",
            [],
        )?;

        reconcile_cleanup_preference(&conn, 2, &[1], stale_text, 200)?;

        let rows = conn
            .prepare(
                "SELECT rule_id, source_memory_id, disabled, action_override, reason
                 FROM preference_rule_overrides
                 ORDER BY rule_id",
            )?
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        assert_eq!(
            rows,
            vec![(
                "pref-2-1".to_string(),
                Some(2),
                1,
                Some("block".to_string()),
                Some("keep coauthor override".to_string()),
            )]
        );
        Ok(())
    }
}
