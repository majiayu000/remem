use std::collections::HashSet;

use anyhow::Result;
use rusqlite::Connection;

use crate::memory::{self, Memory};
use crate::truth::CurrentTruthProjection;

#[cfg(test)]
thread_local! {
    static FORCE_MATERIALIZATION_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
pub(super) fn with_forced_materialization_failure<T>(run: impl FnOnce() -> T) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            FORCE_MATERIALIZATION_FAILURE.with(|flag| flag.set(false));
        }
    }
    FORCE_MATERIALIZATION_FAILURE.with(|flag| flag.set(true));
    let _reset = Reset;
    run()
}

pub(super) fn clustered_mutable_candidate_ids(
    conn: &Connection,
    drops: &[super::types::ContextPreselectionDrop],
    errors: &mut Vec<super::types::ContextLoadError>,
) -> HashSet<i64> {
    match try_clustered_mutable_candidate_ids(conn, drops) {
        Ok(ids) => ids,
        Err(error) => {
            let message = format!("failed to identify clustered CurrentTruth candidates: {error}");
            crate::log::error("context", &message);
            errors.push(super::types::ContextLoadError::new(
                "current_truth",
                message,
            ));
            HashSet::new()
        }
    }
}

fn try_clustered_mutable_candidate_ids(
    conn: &Connection,
    drops: &[super::types::ContextPreselectionDrop],
) -> Result<HashSet<i64>> {
    let candidate_ids = drops
        .iter()
        .filter(|drop| drop.reason == "memory_cluster_dedup")
        .filter_map(|drop| match &drop.item {
            super::types::ContextPreselectionItem::Memory(memory)
                if matches!(memory.memory_type.as_str(), "decision" | "architecture") =>
            {
                Some(memory.id)
            }
            _ => None,
        })
        .collect::<HashSet<_>>();
    if candidate_ids.is_empty() {
        return Ok(candidate_ids);
    }

    let ids_json = serde_json::to_string(&candidate_ids)?;
    let mut stmt = conn.prepare(
        "SELECT memories.id
         FROM memories
         JOIN memory_state_keys ON memory_state_keys.id = memories.state_key_id
         WHERE memories.id IN (SELECT value FROM json_each(?1))
           AND memory_state_keys.state_key <> COALESCE(memories.topic_key, '')",
    )?;
    let rows = stmt.query_map([ids_json], |row| row.get::<_, i64>(0))?;
    Ok(crate::db::query::collect_rows(rows)?.into_iter().collect())
}

/// Reinsert an authoritative winner only when canonical preselection retained
/// one of its displaced rivals. This repairs the slot without widening the
/// bounded SessionStart candidate universe to every project state slot.
pub(super) fn materialize_winners(
    conn: &Connection,
    memories: &mut Vec<Memory>,
    projection: &CurrentTruthProjection,
    clustered_candidate_ids: &HashSet<i64>,
) -> Result<HashSet<i64>> {
    let existing = memories
        .iter()
        .map(|memory| memory.id)
        .collect::<HashSet<_>>();
    let claim_id = |value: &str| value.strip_prefix("memory:")?.parse::<i64>().ok();
    let missing = projection
        .truths
        .iter()
        .filter_map(|truth| {
            let winner = claim_id(&truth.claim.as_ref()?.canonical_ref)?;
            let displaced_loaded = truth
                .rejected
                .iter()
                .filter_map(|key| claim_id(key))
                .any(|id| existing.contains(&id));
            (!existing.contains(&winner)
                && (displaced_loaded || clustered_candidate_ids.contains(&winner)))
            .then_some(winner)
        })
        .collect::<HashSet<_>>();
    if missing.is_empty() {
        return Ok(missing);
    }
    #[cfg(test)]
    FORCE_MATERIALIZATION_FAILURE.with(|flag| {
        if flag.get() {
            anyhow::bail!("forced CurrentTruth winner materialization failure");
        }
        Ok(())
    })?;
    let ids_json = serde_json::to_string(&missing)?;
    let sql = format!(
        "SELECT {} FROM memories
         WHERE id IN (SELECT value FROM json_each(?1))
           AND {}",
        memory::MEMORY_COLS,
        memory::suppression::memory_policy_filter_sql("memories")
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([ids_json], memory::map_memory_row_pub)?;
    let winners = crate::db::query::collect_rows(rows)?;
    let loaded = winners
        .iter()
        .map(|memory| memory.id)
        .collect::<HashSet<_>>();
    anyhow::ensure!(
        loaded == missing,
        "CurrentTruth winner materialization incomplete: requested {:?}, loaded {:?}",
        missing,
        loaded
    );
    memories.extend(winners);
    Ok(loaded)
}
