use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, Context, Result};
use rusqlite::{Connection, OptionalExtension};

use super::{
    ActivationRouteKind, ActiveMemoryWritePermit, ActiveMemoryWriteRequest, ActiveMemoryWriteResult,
};

struct FreshActivation {
    index: usize,
    request_sha256: String,
    result_memory_id: Option<i64>,
    result_sha256: Option<String>,
}

/// Executes independent add-only activations under one active-set snapshot.
///
/// This is intentionally narrower than `execute_one`: batch members cannot
/// supersede rows or use supplemental-save receipts. The shared before/after
/// validation keeps pack imports linear in the number of active rows while
/// preserving one immutable receipt per imported memory.
pub(crate) fn execute_add_batch(
    conn: &Connection,
    requests: &[ActiveMemoryWriteRequest],
    mut write: impl FnMut(usize, &ActiveMemoryWritePermit) -> Result<i64>,
) -> Result<Vec<ActiveMemoryWriteResult>> {
    let mut activation_ids = BTreeSet::new();
    let mut results = vec![None; requests.len()];
    let mut fresh = Vec::new();

    for (index, request) in requests.iter().enumerate() {
        if request.route_kind == ActivationRouteKind::SupplementalSave {
            bail!("supplemental_save activations cannot use the add-only batch boundary");
        }
        if !activation_ids.insert(request.activation_id.as_str()) {
            bail!("add-only activation batch contains duplicate activation ids");
        }
        let normalized_superseded_ids = super::validate_request(request)?;
        if !normalized_superseded_ids.is_empty() {
            bail!("add-only activation batch cannot supersede active memories");
        }
        let request_sha256 =
            super::receipt::current_request_sha256(request, &normalized_superseded_ids)?;
        let existing = conn
            .query_row(
                "SELECT 1 FROM memory_activation_requests WHERE activation_id = ?1",
                [&request.activation_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if existing {
            results[index] = Some(super::execute_one(conn, request, |_| {
                bail!("replayed add-only activation unexpectedly invoked its writer")
            })?);
        } else {
            fresh.push(FreshActivation {
                index,
                request_sha256,
                result_memory_id: None,
                result_sha256: None,
            });
        }
    }

    if fresh.is_empty() {
        return collect_results(results);
    }

    conn.execute_batch("SAVEPOINT remem_active_memory_add_batch")?;
    let applied: Result<()> = (|| {
        let active_before = super::active_memory_snapshot(conn)?;
        let permit = ActiveMemoryWritePermit { _private: () };
        for pending in &mut fresh {
            let request = &requests[pending.index];
            let memory_id = write(pending.index, &permit)?;
            super::validate_result_route(conn, memory_id, request, true)?;
            let result_sha256 = super::payload::validate_result_payload(conn, memory_id, request)?;
            pending.result_memory_id = Some(memory_id);
            pending.result_sha256 = Some(result_sha256);
        }
        let active_after = super::active_memory_snapshot(conn)?;
        validate_add_delta(&active_before, &active_after, &fresh)?;
        for pending in &fresh {
            let request = &requests[pending.index];
            let memory_id = pending
                .result_memory_id
                .context("add-only activation lost its result memory id")?;
            super::validate_result_route(conn, memory_id, request, true)?;
            let final_sha256 = super::payload::validate_result_payload(conn, memory_id, request)?;
            if pending.result_sha256.as_deref() != Some(final_sha256.as_str()) {
                bail!("add-only activation result changed during the batch");
            }
        }
        for pending in &fresh {
            let memory_id = pending
                .result_memory_id
                .context("add-only activation lost its result memory id")?;
            let result_sha256 = pending
                .result_sha256
                .as_deref()
                .context("add-only activation lost its result digest")?;
            super::execute::record_receipt(
                conn,
                &requests[pending.index],
                &[],
                &pending.request_sha256,
                result_sha256,
                memory_id,
                None,
                None,
            )?;
            results[pending.index] = Some(ActiveMemoryWriteResult {
                memory_id,
                replayed: false,
                supplemental_receipt: None,
                supplemental_local_copy_receipt: None,
            });
        }
        Ok(())
    })();

    match applied {
        Ok(()) => conn.execute_batch("RELEASE SAVEPOINT remem_active_memory_add_batch")?,
        Err(error) => {
            let rollback = conn.execute_batch(
                "ROLLBACK TO SAVEPOINT remem_active_memory_add_batch;
                 RELEASE SAVEPOINT remem_active_memory_add_batch;",
            );
            if let Err(rollback_error) = rollback {
                return Err(error.context(format!(
                    "add-only activation rollback also failed: {rollback_error}"
                )));
            }
            return Err(error);
        }
    }
    collect_results(results)
}

fn validate_add_delta(
    before: &BTreeMap<i64, String>,
    after: &BTreeMap<i64, String>,
    fresh: &[FreshActivation],
) -> Result<()> {
    let removed = before
        .keys()
        .filter(|id| !after.contains_key(id))
        .copied()
        .collect::<BTreeSet<_>>();
    if !removed.is_empty() {
        bail!("add-only activation batch removed active rows: {removed:?}");
    }
    let result_ids = fresh
        .iter()
        .map(|pending| {
            pending
                .result_memory_id
                .context("add-only activation lost its result memory id")
        })
        .collect::<Result<BTreeSet<_>>>()?;
    if result_ids.len() != fresh.len() {
        bail!("add-only activation batch returned a result memory more than once");
    }
    let additions = after
        .keys()
        .filter(|id| !before.contains_key(id))
        .copied()
        .collect::<BTreeSet<_>>();
    if additions != result_ids {
        bail!(
            "add-only activation batch result/addition drift: declared={result_ids:?} actual={additions:?}"
        );
    }
    let unrelated_changes = before
        .iter()
        .filter_map(|(id, digest)| {
            after
                .get(id)
                .filter(|after_digest| *after_digest != digest)
                .map(|_| *id)
        })
        .collect::<BTreeSet<_>>();
    if !unrelated_changes.is_empty() {
        bail!("add-only activation batch modified unrelated active rows: {unrelated_changes:?}");
    }
    Ok(())
}

fn collect_results(
    results: Vec<Option<ActiveMemoryWriteResult>>,
) -> Result<Vec<ActiveMemoryWriteResult>> {
    results
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .context("add-only activation batch did not produce every result")
}
