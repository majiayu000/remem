use anyhow::{bail, ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use super::{ActivationPoisoningVerdict, ExpectedActiveMemory};
use crate::memory::poisoning::SourceTrustClass;

pub(crate) fn replay_supplemental_if_present(
    conn: &Connection,
    request: &super::ActiveMemoryWriteRequest,
) -> Result<Option<super::ActiveMemoryWriteResult>> {
    if request.route_kind != super::ActivationRouteKind::SupplementalSave {
        bail!("early supplemental replay requires the supplemental_save route");
    }
    let normalized_superseded_ids = super::validate_request(request)?;
    let existing = conn
        .query_row(
            "SELECT result_sha256, result_memory_id, claim_status, claim_id, claim_error,
                    local_copy_status, local_copy_path, local_copy_saved_at, local_copy_sha256
             FROM memory_activation_requests WHERE activation_id = ?1",
            [&request.activation_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            },
        )
        .optional()?;
    let Some((
        result_sha256,
        memory_id,
        claim_status,
        claim_id,
        claim_error,
        local_copy_status,
        local_copy_path,
        local_copy_saved_at,
        local_copy_sha256,
    )) = existing
    else {
        return Ok(None);
    };
    if !super::receipt::supplemental_caller_request_matches_receipt(
        conn,
        request,
        &normalized_superseded_ids,
    )? {
        return Err(super::ActivationIdConflictError {
            activation_id: request.activation_id.clone(),
        }
        .into());
    }
    validate_replayed_result(conn, &request.activation_id, memory_id, &result_sha256)?;
    Ok(Some(super::ActiveMemoryWriteResult {
        memory_id,
        replayed: true,
        supplemental_receipt: super::SupplementalSaveReceipt::from_columns(
            claim_status,
            claim_id,
            claim_error,
        )?,
        supplemental_local_copy_receipt: Some(super::SupplementalLocalCopyReceipt::from_columns(
            local_copy_status,
            local_copy_path,
            local_copy_saved_at,
            local_copy_sha256,
        )?),
    }))
}

pub(crate) fn replay_scope_cleanup_if_present(
    conn: &Connection,
    activation_id: &str,
    payload_sha256: &str,
    provenance_ref: &str,
    superseded_ids: &[i64],
) -> Result<Option<super::ActiveMemoryWriteResult>> {
    let normalized = superseded_ids
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    if normalized.iter().any(|id| *id <= 0) || normalized.len() != superseded_ids.len() {
        bail!("memory activation superseded ids must be unique positive integers");
    }
    let superseded_ids_json = serde_json::to_string(&normalized.into_iter().collect::<Vec<_>>())?;
    let existing = conn
        .query_row(
            "SELECT result_sha256, result_memory_id, route_kind, actor_kind,
                    source_operation, provenance_kind, provenance_ref, payload_sha256,
                    superseded_ids_json
             FROM memory_activation_requests WHERE activation_id = ?1",
            [activation_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                ))
            },
        )
        .optional()?;
    let Some((
        result_sha256,
        memory_id,
        route_kind,
        actor_kind,
        source_operation,
        provenance_kind,
        stored_provenance_ref,
        stored_payload_sha256,
        stored_superseded_ids,
    )) = existing
    else {
        return Ok(None);
    };
    if route_kind != "scope_cleanup"
        || actor_kind != "operator"
        || source_operation != "memory_cleanup"
        || provenance_kind != "scope_plan"
        || stored_provenance_ref != provenance_ref
        || stored_payload_sha256 != payload_sha256
        || stored_superseded_ids != superseded_ids_json
    {
        return Err(super::ActivationIdConflictError {
            activation_id: activation_id.to_string(),
        }
        .into());
    }
    validate_scope_cleanup_replayed_result(conn, activation_id, memory_id, &result_sha256)?;
    Ok(Some(super::ActiveMemoryWriteResult {
        memory_id,
        replayed: true,
        supplemental_receipt: None,
        supplemental_local_copy_receipt: None,
    }))
}

pub(crate) fn replay_dream_if_present(
    conn: &Connection,
    request: &super::ActiveMemoryWriteRequest,
) -> Result<Option<super::ActiveMemoryWriteResult>> {
    if request.route_kind != super::ActivationRouteKind::DreamConsolidation {
        bail!("early Dream replay requires the dream_consolidation route");
    }
    let normalized_superseded_ids = super::validate_request(request)?;
    let existing = conn
        .query_row(
            "SELECT request_sha256, result_sha256, result_memory_id
             FROM memory_activation_requests
             WHERE activation_id = ?1",
            [&request.activation_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((stored_request_sha256, stored_result_sha256, memory_id)) = existing else {
        return Ok(None);
    };
    let current_request_sha256 =
        super::receipt::current_request_sha256(request, &normalized_superseded_ids)?;
    if stored_request_sha256 != current_request_sha256
        && !super::receipt::request_identity_matches_receipt(
            conn,
            request,
            &normalized_superseded_ids,
        )?
    {
        return Err(super::ActivationIdConflictError {
            activation_id: request.activation_id.clone(),
        }
        .into());
    }
    validate_replayed_result(
        conn,
        &request.activation_id,
        memory_id,
        &stored_result_sha256,
    )?;
    Ok(Some(super::ActiveMemoryWriteResult {
        memory_id,
        replayed: true,
        supplemental_receipt: None,
        supplemental_local_copy_receipt: None,
    }))
}

struct ActivationReceipt {
    rowid: i64,
    result_sha256: String,
    project: String,
    branch_present: bool,
    branch: Option<String>,
    scope: String,
    owner_scope: String,
    owner_key: String,
    target_project: Option<String>,
    source_project: String,
    result_source_trust_class: String,
    poisoning_verdict: ActivationPoisoningVerdict,
}

pub(super) fn validate_replayed_result(
    conn: &Connection,
    activation_id: &str,
    memory_id: i64,
    historical_result_sha256: &str,
) -> Result<()> {
    validate_replayed_result_inner(conn, activation_id, memory_id, historical_result_sha256)
}

fn validate_scope_cleanup_replayed_result(
    conn: &Connection,
    activation_id: &str,
    memory_id: i64,
    historical_result_sha256: &str,
) -> Result<()> {
    validate_replayed_result_inner(conn, activation_id, memory_id, historical_result_sha256)
}

fn validate_replayed_result_inner(
    conn: &Connection,
    activation_id: &str,
    memory_id: i64,
    historical_result_sha256: &str,
) -> Result<()> {
    let historical = receipt_by_activation(conn, activation_id, memory_id)?;
    ensure!(
        historical.result_sha256 == historical_result_sha256,
        "memory activation historical result digest does not match its receipt"
    );
    let later = later_receipt(conn, historical.rowid, memory_id)?;
    let latest = later.as_ref().unwrap_or(&historical);
    let current = ExpectedActiveMemory::from_existing(conn, memory_id)?;
    ensure!(
        current.sha256() == latest.result_sha256,
        "memory activation latest result payload has drifted"
    );
    validate_latest_route(conn, memory_id, latest)?;
    super::payload::validate_replayed_poisoning_verdict(conn, memory_id, latest.poisoning_verdict)?;
    Ok(())
}

fn receipt_by_activation(
    conn: &Connection,
    activation_id: &str,
    memory_id: i64,
) -> Result<ActivationReceipt> {
    conn.query_row(
        "SELECT rowid, result_sha256, project, branch_present, branch, scope,
                owner_scope, owner_key, target_project, source_project,
                result_source_trust_class, poisoning_verdict
         FROM memory_activation_requests
         WHERE result_memory_id = ?1 AND activation_id = ?2",
        params![memory_id, activation_id],
        map_receipt,
    )
    .map_err(Into::into)
}

fn later_receipt(
    conn: &Connection,
    after_rowid: i64,
    memory_id: i64,
) -> Result<Option<ActivationReceipt>> {
    conn.query_row(
        "SELECT rowid, result_sha256, project, branch_present, branch, scope,
                owner_scope, owner_key, target_project, source_project,
                result_source_trust_class, poisoning_verdict
         FROM memory_activation_requests
         WHERE result_memory_id = ?1 AND rowid > ?2
         ORDER BY rowid DESC
         LIMIT 1",
        params![memory_id, after_rowid],
        map_receipt,
    )
    .optional()
    .map_err(Into::into)
}

fn map_receipt(row: &rusqlite::Row<'_>) -> rusqlite::Result<ActivationReceipt> {
    let verdict = parse_verdict(&row.get::<_, String>(11)?).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(11, rusqlite::types::Type::Text, error.into())
    })?;
    Ok(ActivationReceipt {
        rowid: row.get(0)?,
        result_sha256: row.get(1)?,
        project: row.get(2)?,
        branch_present: row.get::<_, i64>(3)? == 1,
        branch: row.get(4)?,
        scope: row.get(5)?,
        owner_scope: row.get(6)?,
        owner_key: row.get(7)?,
        target_project: row.get(8)?,
        source_project: row.get(9)?,
        result_source_trust_class: row.get(10)?,
        poisoning_verdict: verdict,
    })
}

fn validate_latest_route(
    conn: &Connection,
    memory_id: i64,
    latest: &ActivationReceipt,
) -> Result<()> {
    let (project, branch, scope, owner_scope, owner_key, target_project, source_project, trust, status): (
        String,
        Option<String>,
        String,
        String,
        String,
        Option<String>,
        String,
        String,
        String,
    ) = conn.query_row(
        "SELECT project, branch, COALESCE(scope, 'project'),
                COALESCE(owner_scope,
                    CASE WHEN COALESCE(scope, 'project') = 'global' THEN 'user' ELSE 'repo' END),
                COALESCE(owner_key,
                    CASE WHEN COALESCE(scope, 'project') = 'global' THEN 'user:default' ELSE project END),
                CASE
                    WHEN COALESCE(owner_scope,
                        CASE WHEN COALESCE(scope, 'project') = 'global' THEN 'user' ELSE 'repo' END) = 'repo'
                    THEN COALESCE(target_project, project)
                    ELSE target_project
                END,
                COALESCE(source_project, project), source_trust_class, status
         FROM memories WHERE id = ?1",
        [memory_id],
        |row| {
            Ok((
                row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?,
                row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?,
            ))
        },
    )?;
    ensure!(
        project == latest.project,
        "memory activation latest project has drifted"
    );
    ensure!(
        branch.is_some() == latest.branch_present && branch == latest.branch,
        "memory activation latest branch has drifted"
    );
    ensure!(
        scope == latest.scope,
        "memory activation latest scope has drifted"
    );
    ensure!(
        owner_scope == latest.owner_scope,
        "memory activation latest owner scope has drifted"
    );
    ensure!(
        owner_key == latest.owner_key,
        "memory activation latest owner key has drifted"
    );
    ensure!(
        target_project == latest.target_project,
        "memory activation latest target project has drifted"
    );
    ensure!(
        source_project == latest.source_project,
        "memory activation latest source project has drifted"
    );
    let current_trust = crate::memory::poisoning::SourceTrustClass::parse(&trust)
        .context("memory activation latest result trust is invalid")?;
    let expected_trust = if let Some(recorded) = latest
        .result_source_trust_class
        .strip_prefix("legacy_v086_source_")
    {
        SourceTrustClass::parse(recorded)
            .context("v086 activation receipt source trust is invalid")?
    } else {
        SourceTrustClass::parse(&latest.result_source_trust_class)
            .context("memory activation latest receipt result trust is invalid")?
    };
    ensure!(
        current_trust == expected_trust,
        "memory activation latest result trust has drifted"
    );
    ensure!(
        status == "active" || result_was_superseded_after(conn, memory_id, latest.rowid)?,
        "memory activation latest result is inactive without a superseding receipt"
    );
    Ok(())
}

fn parse_verdict(value: &str) -> Result<ActivationPoisoningVerdict> {
    match value {
        "clean" => Ok(ActivationPoisoningVerdict::Clean),
        "acknowledged" => Ok(ActivationPoisoningVerdict::Acknowledged),
        "upstream_validated" => Ok(ActivationPoisoningVerdict::UpstreamValidated),
        "exact_recovery" => Ok(ActivationPoisoningVerdict::ExactRecovery),
        _ => bail!("unrecognized activation poisoning verdict: {value}"),
    }
}

fn result_was_superseded_after(
    conn: &Connection,
    memory_id: i64,
    after_rowid: i64,
) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(
             SELECT 1
             FROM memory_activation_requests later,
                  json_each(later.superseded_ids_json) superseded
             WHERE later.rowid > ?2 AND superseded.value = ?1
         )",
        params![memory_id, after_rowid],
        |row| row.get::<_, i64>(0),
    )
    .map(|superseded| superseded == 1)
    .map_err(Into::into)
}
