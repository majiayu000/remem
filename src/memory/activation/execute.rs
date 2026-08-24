use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

use super::*;

pub(super) fn one_inner(
    conn: &Connection,
    request: &ActiveMemoryWriteRequest,
    write: impl FnOnce(
        &ActiveMemoryWritePermit,
    ) -> Result<(
        i64,
        Option<SupplementalSaveReceipt>,
        Option<SupplementalLocalCopyReceipt>,
    )>,
) -> Result<ActiveMemoryWriteResult> {
    let normalized_superseded_ids = validate_request(request)?;
    let request_sha256 = receipt::current_request_sha256(request, &normalized_superseded_ids)?;
    with_write_serialization(conn, |conn| {
        one_serialized(
            conn,
            request,
            normalized_superseded_ids,
            request_sha256,
            write,
        )
    })
}

fn one_serialized(
    conn: &Connection,
    request: &ActiveMemoryWriteRequest,
    normalized_superseded_ids: Vec<i64>,
    request_sha256: String,
    write: impl FnOnce(
        &ActiveMemoryWritePermit,
    ) -> Result<(
        i64,
        Option<SupplementalSaveReceipt>,
        Option<SupplementalLocalCopyReceipt>,
    )>,
) -> Result<ActiveMemoryWriteResult> {
    if let Some((
        stored_sha256,
        stored_result_sha256,
        memory_id,
        stored_result_trust_class,
        claim_status,
        claim_id,
        claim_error,
        local_copy_status,
        local_copy_path,
        local_copy_saved_at,
        local_copy_sha256,
    )) = conn
        .query_row(
            "SELECT request_sha256, result_sha256, result_memory_id,
                    result_source_trust_class, claim_status, claim_id, claim_error,
                    local_copy_status, local_copy_path, local_copy_saved_at,
                    local_copy_sha256
             FROM memory_activation_requests
             WHERE activation_id = ?1",
            [&request.activation_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                ))
            },
        )
        .optional()?
    {
        if stored_sha256 != request_sha256 {
            let is_v086 = stored_result_trust_class.starts_with("legacy_v086_source_");
            let matches_legacy_request = if request.route_kind
                == ActivationRouteKind::SupplementalSave
            {
                receipt::supplemental_request_matches_receipt(
                    conn,
                    request,
                    &normalized_superseded_ids,
                )?
            } else if is_v086 {
                stored_sha256 == receipt::v086_request_sha256(request, &normalized_superseded_ids)?
            } else {
                false
            };
            if !matches_legacy_request {
                return Err(ActivationIdConflictError {
                    activation_id: request.activation_id.clone(),
                }
                .into());
            }
        }
        replay::validate_replayed_result(
            conn,
            &request.activation_id,
            memory_id,
            &stored_result_sha256,
        )?;
        let supplemental_receipt =
            SupplementalSaveReceipt::from_columns(claim_status, claim_id, claim_error)?;
        let supplemental_local_copy_receipt =
            if request.route_kind == ActivationRouteKind::SupplementalSave {
                Some(SupplementalLocalCopyReceipt::from_columns(
                    local_copy_status,
                    local_copy_path,
                    local_copy_saved_at,
                    local_copy_sha256,
                )?)
            } else {
                None
            };
        return Ok(ActiveMemoryWriteResult {
            memory_id,
            replayed: true,
            supplemental_receipt,
            supplemental_local_copy_receipt,
        });
    }

    conn.execute_batch("SAVEPOINT remem_active_memory_boundary")?;
    let result: Result<ActiveMemoryWriteResult> = (|| {
        validate_supersede_routes(conn, request, &normalized_superseded_ids)?;
        let active_before = active_memory_snapshot(conn)?;
        let permit = ActiveMemoryWritePermit { _private: () };
        let (memory_id, supplemental_receipt, supplemental_local_copy_receipt) = write(&permit)?;
        validate_result_route(conn, memory_id, request, true)?;
        let result_sha256 = payload::validate_result_payload(conn, memory_id, request)?;
        let active_after = active_memory_snapshot(conn)?;
        validate_active_delta(
            memory_id,
            &normalized_superseded_ids,
            &active_before,
            &active_after,
        )?;
        record_receipt(
            conn,
            request,
            &normalized_superseded_ids,
            &request_sha256,
            &result_sha256,
            memory_id,
            supplemental_receipt.as_ref(),
            supplemental_local_copy_receipt.as_ref(),
        )?;
        Ok(ActiveMemoryWriteResult {
            memory_id,
            replayed: false,
            supplemental_receipt,
            supplemental_local_copy_receipt,
        })
    })();

    match result {
        Ok(result) => {
            conn.execute_batch("RELEASE SAVEPOINT remem_active_memory_boundary")?;
            Ok(result)
        }
        Err(error) => {
            let rollback = conn.execute_batch(
                "ROLLBACK TO SAVEPOINT remem_active_memory_boundary;
                 RELEASE SAVEPOINT remem_active_memory_boundary;",
            );
            if let Err(rollback_error) = rollback {
                return Err(error.context(format!(
                    "memory activation rollback also failed: {rollback_error}"
                )));
            }
            Err(error)
        }
    }
}

pub(super) fn record_receipt(
    conn: &Connection,
    request: &ActiveMemoryWriteRequest,
    normalized_superseded_ids: &[i64],
    request_sha256: &str,
    result_sha256: &str,
    memory_id: i64,
    supplemental_receipt: Option<&SupplementalSaveReceipt>,
    supplemental_local_copy_receipt: Option<&SupplementalLocalCopyReceipt>,
) -> Result<()> {
    let superseded_json = serde_json::to_string(normalized_superseded_ids)
        .context("serialize activation supersede set")?;
    let claim_status = supplemental_receipt.map(SupplementalSaveReceipt::status);
    let claim_id = supplemental_receipt.and_then(SupplementalSaveReceipt::claim_id);
    let claim_error = supplemental_receipt.and_then(SupplementalSaveReceipt::error);
    let local_copy_status =
        supplemental_local_copy_receipt.and_then(SupplementalLocalCopyReceipt::status);
    let local_copy_path =
        supplemental_local_copy_receipt.and_then(SupplementalLocalCopyReceipt::path);
    let local_copy_saved_at =
        supplemental_local_copy_receipt.and_then(SupplementalLocalCopyReceipt::saved_at);
    let local_copy_sha256 =
        supplemental_local_copy_receipt.and_then(SupplementalLocalCopyReceipt::sha256);
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO memory_activation_requests
         (activation_id, request_sha256, route_kind, actor_kind,
          source_operation, source_trust_class, result_source_trust_class,
          source_project, project, branch_present,
          branch, scope, owner_scope, owner_key, target_project,
          provenance_kind, provenance_ref, payload_sha256, result_sha256,
          poisoning_verdict, superseded_ids_json, result_memory_id,
          claim_status, claim_id, claim_error, local_copy_status, local_copy_path,
          local_copy_saved_at, local_copy_sha256, created_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                 ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21,
                 ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30)",
        params![
            request.activation_id,
            request_sha256,
            enum_json(request.route_kind)?,
            enum_json(request.actor_kind)?,
            request.source_operation,
            request.source_trust.as_str(),
            request.result_source_trust.as_str(),
            request.source_project,
            request.route.project,
            i64::from(request.route.branch.is_some()),
            request.route.branch,
            request.route.scope,
            request.route.owner_scope,
            request.route.owner_key,
            request.route.target_project,
            enum_json(request.provenance_kind)?,
            request.provenance_ref,
            request.payload_sha256,
            result_sha256,
            enum_json(request.poisoning_verdict)?,
            superseded_json,
            memory_id,
            claim_status,
            claim_id,
            claim_error,
            local_copy_status,
            local_copy_path,
            local_copy_saved_at,
            local_copy_sha256,
            now,
        ],
    )?;
    Ok(())
}

fn with_write_serialization<T>(
    conn: &Connection,
    run: impl FnOnce(&Connection) -> Result<T>,
) -> Result<T> {
    if !conn.is_autocommit() {
        conn.execute(
            "UPDATE memory_activation_requests
             SET activation_id = activation_id WHERE 0",
            [],
        )
        .context("serialize memory activation inside caller transaction")?;
        return run(conn);
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .context("begin serialized memory activation")?;
    match run(&tx) {
        Ok(value) => {
            tx.commit().context("commit serialized memory activation")?;
            Ok(value)
        }
        Err(error) => match tx.rollback() {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(error.context(format!(
                "memory activation transaction rollback also failed: {rollback_error}"
            ))),
        },
    }
}
