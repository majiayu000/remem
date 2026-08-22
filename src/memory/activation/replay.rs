use anyhow::{bail, ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use super::{ActivationPoisoningVerdict, ActiveMemoryWriteRequest, ExpectedActiveMemory};

struct LaterReceipt {
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

pub(super) fn validate_later_activation_result(
    conn: &Connection,
    activation_id: &str,
    memory_id: i64,
    historical_request: &ActiveMemoryWriteRequest,
    historical_result_sha256: &str,
) -> Result<bool> {
    let Some(latest) = later_receipt(conn, activation_id, memory_id)? else {
        return Ok(false);
    };
    ensure!(
        historical_result_sha256 == historical_request.expected_memory.sha256(),
        "memory activation historical result digest does not match its request"
    );
    let current = ExpectedActiveMemory::from_existing(conn, memory_id)?;
    ensure!(
        current.sha256() == latest.result_sha256,
        "memory activation latest result payload has drifted"
    );
    validate_latest_route(conn, memory_id, &latest)?;
    super::payload::validate_poisoning_verdict(
        conn,
        memory_id,
        &current,
        latest.poisoning_verdict,
    )?;
    Ok(true)
}

fn later_receipt(
    conn: &Connection,
    activation_id: &str,
    memory_id: i64,
) -> Result<Option<LaterReceipt>> {
    conn.query_row(
        "SELECT later.result_sha256, later.project, later.branch_present, later.branch,
                later.scope, later.owner_scope, later.owner_key, later.target_project,
                later.source_project, later.result_source_trust_class, later.poisoning_verdict
         FROM memory_activation_requests later
         WHERE later.result_memory_id = ?1
           AND later.rowid > (
               SELECT current.rowid FROM memory_activation_requests current
               WHERE current.activation_id = ?2
           )
         ORDER BY later.rowid DESC
         LIMIT 1",
        params![memory_id, activation_id],
        |row| {
            let verdict = parse_verdict(&row.get::<_, String>(10)?).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    10,
                    rusqlite::types::Type::Text,
                    error.into(),
                )
            })?;
            Ok(LaterReceipt {
                result_sha256: row.get(0)?,
                project: row.get(1)?,
                branch_present: row.get::<_, i64>(2)? == 1,
                branch: row.get(3)?,
                scope: row.get(4)?,
                owner_scope: row.get(5)?,
                owner_key: row.get(6)?,
                target_project: row.get(7)?,
                source_project: row.get(8)?,
                result_source_trust_class: row.get(9)?,
                poisoning_verdict: verdict,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

fn validate_latest_route(conn: &Connection, memory_id: i64, latest: &LaterReceipt) -> Result<()> {
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
    ensure!(
        latest.result_source_trust_class != "legacy_unrecorded",
        "memory activation latest receipt predates authenticated result trust; replay under a new activation id"
    );
    let expected_trust =
        crate::memory::poisoning::SourceTrustClass::parse(&latest.result_source_trust_class)
            .context("memory activation latest receipt result trust is invalid")?;
    ensure!(
        current_trust == expected_trust,
        "memory activation latest result trust has drifted"
    );
    ensure!(
        status == "active" || result_was_superseded(conn, memory_id)?,
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

pub(super) fn result_was_superseded(conn: &Connection, memory_id: i64) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(
             SELECT 1
             FROM memory_activation_requests later,
                  json_each(later.superseded_ids_json) superseded
             WHERE superseded.value = ?1
         )",
        [memory_id],
        |row| row.get::<_, i64>(0),
    )
    .map(|superseded| superseded == 1)
    .map_err(Into::into)
}
