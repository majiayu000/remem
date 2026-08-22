use anyhow::{bail, ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use super::{ActivationPoisoningVerdict, ExpectedActiveMemory};
use crate::memory::poisoning::SourceTrustClass;

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
    super::payload::validate_poisoning_verdict(
        conn,
        memory_id,
        &current,
        latest.poisoning_verdict,
    )?;
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
