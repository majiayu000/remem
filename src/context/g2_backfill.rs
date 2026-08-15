use anyhow::{Context, Result};
use rusqlite::Connection;

const MAX_SCAN_PAGES: usize = 16;

/// Page through a stable ranked source until `target` G2-admitted rows survive
/// or the source is exhausted. Each query, classification batch, and the total
/// scan are bounded. Hitting the budget is an error so context never silently
/// degrades to a short result.
pub(super) fn fetch_until_admitted<T>(
    conn: &Connection,
    target: usize,
    initial_limit: i64,
    mut fetch: impl FnMut(i64, i64) -> Result<Vec<T>>,
    memory_id: impl Fn(&T) -> i64,
    as_of_epoch: i64,
) -> Result<Vec<T>> {
    if target == 0 {
        return Ok(Vec::new());
    }
    let page_size = initial_limit.max(target as i64).clamp(32, 256);
    let mut offset = 0_i64;
    let mut collected = Vec::new();
    let mut admitted_count = 0usize;
    for page_index in 0..MAX_SCAN_PAGES {
        let rows = fetch(offset, page_size)?;
        let page_len = rows.len();
        let ids = rows.iter().map(&memory_id).collect::<Vec<_>>();
        let admitted = crate::truth::admit_many_for_current_context(conn, &ids, as_of_epoch)?;
        admitted_count += ids
            .iter()
            .filter(|id| {
                admitted
                    .get(id)
                    .is_some_and(|row| row.current_context_eligible)
            })
            .count();
        collected.extend(rows);
        if admitted_count >= target || page_len < page_size as usize {
            return Ok(collected);
        }
        if page_index + 1 == MAX_SCAN_PAGES {
            anyhow::bail!(
                "G2 ranked scan budget exhausted after {} rows without finding {target} eligible rows",
                offset + page_size
            );
        }
        offset = offset
            .checked_add(page_size)
            .context("G2 ranked scan offset overflow before source exhaustion")?;
    }
    unreachable!("bounded G2 page loop always returns or errors")
}

/// Materialize one fixed ranked window and retain the shortest prefix that
/// contains `target` admitted rows. This avoids rebuilding hybrid ranking from
/// offset zero for every page while keeping the same fail-closed scan budget.
pub(super) fn fetch_bounded_ranked<T>(
    conn: &Connection,
    target: usize,
    initial_limit: i64,
    fetch: impl FnOnce(i64) -> Result<Vec<T>>,
    memory_id: impl Fn(&T) -> i64,
    as_of_epoch: i64,
) -> Result<Vec<T>> {
    if target == 0 {
        return Ok(Vec::new());
    }
    let page_size = initial_limit.max(target as i64).clamp(32, 256);
    let scan_limit = page_size
        .checked_mul(MAX_SCAN_PAGES as i64)
        .context("G2 ranked window size overflow")?;
    let mut rows = fetch(scan_limit)?;
    let source_exhausted = rows.len() < scan_limit as usize;
    let ids = rows.iter().map(&memory_id).collect::<Vec<_>>();
    let admitted = crate::truth::admit_many_for_current_context(conn, &ids, as_of_epoch)?;
    let mut admitted_count = 0usize;
    let mut prefix_len = rows.len();
    for (index, id) in ids.iter().enumerate() {
        if admitted
            .get(id)
            .is_some_and(|row| row.current_context_eligible)
        {
            admitted_count += 1;
            if admitted_count == target {
                prefix_len = index + 1;
                break;
            }
        }
    }
    if admitted_count < target && !source_exhausted {
        anyhow::bail!(
            "G2 ranked scan budget exhausted after {scan_limit} rows without finding {target} eligible rows"
        );
    }
    rows.truncate(prefix_len);
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;

    #[test]
    fn legacy_heavy_backfill_advances_bounded_pages() {
        let conn = Connection::open_in_memory().expect("in-memory connection");
        super::super::tests::setup_context_schema(&conn);
        for id in 1..=40 {
            super::super::tests::insert_memory(
                &conn,
                id,
                "demo/project",
                Some(&format!("paged-{id}")),
                "bugfix",
                &format!("memory {id}"),
                "body",
                1_710_000_000,
            );
        }
        conn.execute(
            "UPDATE memory_candidates SET review_status = 'rejected'
             WHERE id BETWEEN 7200001 AND 7200032",
            [],
        )
        .expect("make first page G2-ineligible");
        let source = (1_i64..=40).collect::<Vec<_>>();
        let mut pages = Vec::new();
        let rows = fetch_until_admitted(
            &conn,
            1,
            1,
            |offset, limit| {
                pages.push((offset, limit));
                Ok(source
                    .iter()
                    .skip(offset as usize)
                    .take(limit as usize)
                    .copied()
                    .collect())
            },
            |id| *id,
            1_710_000_000,
        )
        .expect("paged backfill");

        assert_eq!(pages, vec![(0, 32), (32, 32)]);
        assert_eq!(rows, source);
    }

    #[test]
    fn legacy_heavy_backfill_reports_total_budget_exhaustion() {
        let conn = Connection::open_in_memory().expect("in-memory connection");
        super::super::tests::setup_context_schema(&conn);
        let source = (1_i64..=512).collect::<Vec<_>>();
        let error = fetch_until_admitted(
            &conn,
            1,
            1,
            |offset, limit| {
                Ok(source
                    .iter()
                    .skip(offset as usize)
                    .take(limit as usize)
                    .copied()
                    .collect())
            },
            |id| *id,
            1_710_000_000,
        )
        .expect_err("missing rows must exhaust the bounded scan");

        assert!(error.to_string().contains("scan budget exhausted"));
    }
}
