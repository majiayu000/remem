use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::poisoning::SourceTrustClass;

mod batch;
mod payload;
mod receipt;
mod replay;
pub(crate) use replay::replay_dream_if_present;
mod route;
pub(crate) use batch::execute_add_batch;
pub(crate) use payload::ExpectedActiveMemory;
pub(crate) use receipt::{SupplementalLocalCopyReceipt, SupplementalSaveReceipt};
pub(crate) use route::load_existing_route;

static ACTIVATION_NONCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ActivationRouteKind {
    RustApi,
    SupplementalSave,
    CandidatePromotion,
    DreamConsolidation,
    PackImport,
    BackupImport,
    ScopeCleanup,
    WebRestore,
    ExactRecovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ActivationActorKind {
    RustApi,
    Agent,
    AutomaticWorker,
    Operator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ActivationProvenanceKind {
    SupplementalSave,
    Candidate,
    Generated,
    Pack,
    Backup,
    ScopePlan,
    WebArchive,
    ExactRecovery,
    RustApi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ActivationPoisoningVerdict {
    Clean,
    Acknowledged,
    UpstreamValidated,
    ExactRecovery,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActiveMemoryRoute {
    pub project: String,
    pub branch: Option<String>,
    pub scope: String,
    pub owner_scope: String,
    pub owner_key: String,
    pub target_project: Option<String>,
}

impl ActiveMemoryRoute {
    pub(crate) fn default_for(project: &str, branch: Option<&str>, scope: &str) -> Self {
        let ownership = super::store::default_ownership(project, scope);
        Self {
            project: project.to_string(),
            branch: branch.map(str::to_string),
            scope: scope.to_string(),
            owner_scope: ownership.owner_scope.to_string(),
            owner_key: ownership.owner_key.to_string(),
            target_project: ownership.target_project.map(str::to_string),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActiveMemoryWriteRequest {
    pub activation_id: String,
    pub route_kind: ActivationRouteKind,
    pub actor_kind: ActivationActorKind,
    pub source_operation: String,
    pub source_trust: SourceTrustClass,
    pub result_source_trust: SourceTrustClass,
    pub source_project: String,
    pub route: ActiveMemoryRoute,
    pub provenance_kind: ActivationProvenanceKind,
    pub provenance_ref: String,
    pub payload_sha256: String,
    pub expected_memory: ExpectedActiveMemory,
    pub poisoning_verdict: ActivationPoisoningVerdict,
    pub superseded_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActiveMemoryWriteResult {
    pub memory_id: i64,
    pub replayed: bool,
    pub supplemental_receipt: Option<SupplementalSaveReceipt>,
    pub supplemental_local_copy_receipt: Option<SupplementalLocalCopyReceipt>,
}

#[derive(Debug)]
pub(crate) struct ActivationIdConflictError {
    activation_id: String,
}

impl std::fmt::Display for ActivationIdConflictError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "memory activation id reused with different request: {}",
            self.activation_id
        )
    }
}

impl std::error::Error for ActivationIdConflictError {}

pub(crate) struct ActiveMemoryWritePermit {
    _private: (),
}

pub(crate) fn execute_one(
    conn: &Connection,
    request: &ActiveMemoryWriteRequest,
    write: impl FnOnce(&ActiveMemoryWritePermit) -> Result<i64>,
) -> Result<ActiveMemoryWriteResult> {
    if request.route_kind == ActivationRouteKind::SupplementalSave {
        bail!("supplemental_save activations must use the durable receipt path");
    }
    execute_one_inner(conn, request, |permit| {
        write(permit).map(|memory_id| (memory_id, None, None))
    })
}

pub(crate) fn execute_supplemental_save(
    conn: &Connection,
    request: &ActiveMemoryWriteRequest,
    write: impl FnOnce(
        &ActiveMemoryWritePermit,
    ) -> Result<(i64, SupplementalSaveReceipt, SupplementalLocalCopyReceipt)>,
) -> Result<ActiveMemoryWriteResult> {
    if request.route_kind != ActivationRouteKind::SupplementalSave {
        bail!("supplemental save receipt requires the supplemental_save route");
    }
    let result = execute_one_inner(conn, request, |permit| {
        write(permit).map(|(memory_id, claim_receipt, local_copy_receipt)| {
            (memory_id, Some(claim_receipt), Some(local_copy_receipt))
        })
    })?;
    if result.supplemental_receipt.is_none() || result.supplemental_local_copy_receipt.is_none() {
        bail!("supplemental save activation is missing a durable response receipt");
    }
    Ok(result)
}

fn execute_one_inner(
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
        record_activation_receipt(
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

fn record_activation_receipt(
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

pub(crate) fn payload_sha256(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"remem-active-memory-payload-v1\0");
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

pub(crate) fn activation_id_from_key(namespace: &str, key: &str) -> String {
    format!("{namespace}:{}", payload_sha256(&[key]))
}

pub(crate) fn ephemeral_activation_id(namespace: &str, payload_hash: &str) -> String {
    let counter = ACTIVATION_NONCE.fetch_add(1, Ordering::Relaxed);
    let nanos = chrono::Utc::now()
        .timestamp_nanos_opt()
        .unwrap_or_else(|| chrono::Utc::now().timestamp() * 1_000_000_000);
    activation_id_from_key(
        namespace,
        &format!("{}:{nanos}:{counter}:{payload_hash}", std::process::id()),
    )
}

fn validate_request(request: &ActiveMemoryWriteRequest) -> Result<Vec<i64>> {
    for (name, value) in [
        ("activation_id", request.activation_id.as_str()),
        ("source_operation", request.source_operation.as_str()),
        ("source_project", request.source_project.as_str()),
        ("project", request.route.project.as_str()),
        ("scope", request.route.scope.as_str()),
        ("owner_scope", request.route.owner_scope.as_str()),
        ("owner_key", request.route.owner_key.as_str()),
        ("provenance_ref", request.provenance_ref.as_str()),
    ] {
        if value.trim().is_empty() || value.contains('\0') {
            bail!("memory activation {name} must be nonblank and contain no NUL");
        }
    }
    if request
        .route
        .branch
        .as_deref()
        .is_some_and(|value| value.trim().is_empty() || value.contains('\0'))
    {
        bail!("memory activation branch must contain no NUL");
    }
    if request
        .route
        .target_project
        .as_deref()
        .is_some_and(|value| value.trim().is_empty() || value.contains('\0'))
    {
        bail!("memory activation target_project must be absent or canonical nonblank text");
    }
    if !is_sha256(&request.payload_sha256) {
        bail!("memory activation payload_sha256 must be lowercase hex64");
    }
    validate_route_identity(request)?;
    validate_route_policy(request)?;
    if !matches!(
        request.route.owner_scope.as_str(),
        "repo" | "user" | "tool" | "domain" | "workstream" | "session" | "workspace"
    ) {
        bail!("memory activation owner_scope is not recognized");
    }
    if request.route.owner_scope == "repo" && request.route.target_project.is_none() {
        bail!("repo-owned memory activation requires target_project");
    }
    let normalized = request
        .superseded_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if normalized.iter().any(|id| *id <= 0) || normalized.len() != request.superseded_ids.len() {
        bail!("memory activation superseded ids must be unique positive integers");
    }
    Ok(normalized.into_iter().collect())
}

fn enum_json(value: impl Serialize) -> Result<String> {
    let encoded = serde_json::to_string(&value)?;
    Ok(encoded.trim_matches('"').to_string())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_supersede_routes(
    conn: &Connection,
    request: &ActiveMemoryWriteRequest,
    superseded_ids: &[i64],
) -> Result<()> {
    for memory_id in superseded_ids {
        let matches: i64 = conn.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM memories
                 WHERE id = ?1 AND status = 'active'
                   AND (?5 = 'user' OR project = ?2)
                   AND branch IS ?3 AND COALESCE(scope, 'project') = ?4
                   AND COALESCE(owner_scope,
                       CASE WHEN COALESCE(scope, 'project') = 'global'
                            THEN 'user' ELSE 'repo' END) = ?5
                   AND COALESCE(owner_key,
                       CASE WHEN COALESCE(scope, 'project') = 'global'
                            THEN 'user:default' ELSE project END) = ?6
                   AND CASE
                       WHEN COALESCE(owner_scope,
                           CASE WHEN COALESCE(scope, 'project') = 'global'
                                THEN 'user' ELSE 'repo' END) = 'repo'
                       THEN COALESCE(target_project, project)
                       ELSE target_project
                   END IS ?7
             )",
            params![
                memory_id,
                request.route.project,
                request.route.branch,
                request.route.scope,
                request.route.owner_scope,
                request.route.owner_key,
                request.route.target_project,
            ],
            |row| row.get(0),
        )?;
        if matches != 1 {
            bail!(
                "memory activation supersede target is missing, inactive, or outside route: {memory_id}"
            );
        }
    }
    Ok(())
}

fn validate_result_route(
    conn: &Connection,
    memory_id: i64,
    request: &ActiveMemoryWriteRequest,
    require_active: bool,
) -> Result<()> {
    let matches: i64 = conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM memories
             WHERE id = ?1 AND project = ?2 AND branch IS ?3
               AND COALESCE(scope, 'project') = ?4
               AND COALESCE(owner_scope,
                   CASE WHEN COALESCE(scope, 'project') = 'global'
                        THEN 'user' ELSE 'repo' END) = ?5
               AND COALESCE(owner_key,
                   CASE WHEN COALESCE(scope, 'project') = 'global'
                        THEN 'user:default' ELSE project END) = ?6
               AND CASE
                   WHEN COALESCE(owner_scope,
                       CASE WHEN COALESCE(scope, 'project') = 'global'
                            THEN 'user' ELSE 'repo' END) = 'repo'
                   THEN COALESCE(target_project, project)
                   ELSE target_project
               END IS ?7
               AND source_trust_class = ?8
               AND COALESCE(source_project, project) = ?9
               AND (?10 = 0 OR status = 'active')
         )",
        params![
            memory_id,
            request.route.project,
            request.route.branch,
            request.route.scope,
            request.route.owner_scope,
            request.route.owner_key,
            request.route.target_project,
            request.result_source_trust.as_str(),
            request.source_project,
            i64::from(require_active),
        ],
        |row| row.get(0),
    )?;
    if matches != 1 {
        bail!("memory activation result failed active route/trust postcondition");
    }
    Ok(())
}

fn validate_active_delta(
    result_memory_id: i64,
    superseded_ids: &[i64],
    before: &std::collections::BTreeMap<i64, String>,
    after: &std::collections::BTreeMap<i64, String>,
) -> Result<()> {
    let removed = before
        .keys()
        .filter(|id| !after.contains_key(id))
        .copied()
        .collect::<BTreeSet<_>>();
    let declared = superseded_ids.iter().copied().collect::<BTreeSet<_>>();
    if removed != declared {
        bail!(
            "memory activation active-set removal drift: declared={declared:?} actual={removed:?}"
        );
    }
    if declared.contains(&result_memory_id) {
        bail!("memory activation cannot supersede its result memory");
    }
    let added = after
        .keys()
        .filter(|id| !before.contains_key(id))
        .copied()
        .collect::<BTreeSet<_>>();
    if added.iter().any(|id| *id != result_memory_id) {
        bail!("memory activation created or reactivated undeclared active rows: {added:?}");
    }
    let changed = before
        .iter()
        .filter_map(|(id, digest)| {
            after
                .get(id)
                .filter(|after_digest| *after_digest != digest)
                .map(|_| *id)
        })
        .filter(|id| *id != result_memory_id)
        .collect::<Vec<_>>();
    if !changed.is_empty() {
        bail!("memory activation modified unrelated active rows: {changed:?}");
    }
    Ok(())
}

fn active_memory_snapshot(conn: &Connection) -> Result<std::collections::BTreeMap<i64, String>> {
    let mut stmt = conn.prepare("SELECT * FROM memories WHERE status = 'active' ORDER BY id")?;
    let column_count = stmt.column_count();
    let rows = stmt.query_map([], |row| {
        let id = row.get::<_, i64>(0)?;
        let mut hasher = Sha256::new();
        for index in 0..column_count {
            match row.get_ref(index)? {
                rusqlite::types::ValueRef::Null => hasher.update([0]),
                rusqlite::types::ValueRef::Integer(value) => {
                    hasher.update([1]);
                    hasher.update(value.to_be_bytes());
                }
                rusqlite::types::ValueRef::Real(value) => {
                    hasher.update([2]);
                    hasher.update(value.to_bits().to_be_bytes());
                }
                rusqlite::types::ValueRef::Text(value) => {
                    hasher.update([3]);
                    hasher.update((value.len() as u64).to_be_bytes());
                    hasher.update(value);
                }
                rusqlite::types::ValueRef::Blob(value) => {
                    hasher.update([4]);
                    hasher.update((value.len() as u64).to_be_bytes());
                    hasher.update(value);
                }
            }
        }
        Ok((id, format!("{:x}", hasher.finalize())))
    })?;
    crate::db::query::collect_rows(rows).map(|rows| rows.into_iter().collect())
}

fn validate_route_identity(request: &ActiveMemoryWriteRequest) -> Result<()> {
    match request.route.scope.as_str() {
        "global"
            if request.route.owner_scope == "user" && request.route.target_project.is_none() => {}
        "project" if request.route.owner_scope != "user" => {}
        _ => bail!("memory activation route has inconsistent scope/owner/target identity"),
    }
    match request.route.owner_scope.as_str() {
        "repo" if request.route.target_project.is_some() => {}
        "repo" => bail!("repo-owned memory activation must bind a target project"),
        "user" if request.route.scope == "global" && request.route.target_project.is_none() => {}
        "tool" | "domain" | "workstream" | "session" | "workspace"
            if request.route.scope == "project" && request.route.target_project.is_none() => {}
        _ => bail!("memory activation route has unsupported owner/scope/target combination"),
    }
    Ok(())
}

fn validate_route_policy(request: &ActiveMemoryWriteRequest) -> Result<()> {
    use ActivationActorKind::{Agent, AutomaticWorker, Operator, RustApi};
    use ActivationPoisoningVerdict::{Acknowledged, Clean, ExactRecovery, UpstreamValidated};
    use ActivationProvenanceKind::{
        Backup, Candidate, Generated, Pack, ScopePlan, SupplementalSave,
    };
    use ActivationRouteKind::{CandidatePromotion, DreamConsolidation, PackImport, ScopeCleanup};

    let valid = match request.route_kind {
        ActivationRouteKind::RustApi => {
            request.actor_kind == RustApi
                && request.provenance_kind == ActivationProvenanceKind::RustApi
                && request.source_trust == SourceTrustClass::LocalToolOutput
                && request.poisoning_verdict == Clean
        }
        ActivationRouteKind::SupplementalSave => match request.actor_kind {
            Agent => {
                request.provenance_kind == SupplementalSave
                    && request.source_trust == SourceTrustClass::ExternalContent
                    && request.poisoning_verdict == Clean
            }
            Operator => {
                request.provenance_kind == SupplementalSave
                    && matches!(
                        request.source_trust,
                        SourceTrustClass::RepoFile | SourceTrustClass::UserPrompt
                    )
                    && matches!(request.poisoning_verdict, Clean | Acknowledged)
            }
            RustApi => {
                request.provenance_kind == SupplementalSave
                    && request.source_trust == SourceTrustClass::LocalToolOutput
                    && matches!(request.poisoning_verdict, Clean | Acknowledged)
            }
            AutomaticWorker => false,
        },
        CandidatePromotion => {
            request.provenance_kind == Candidate
                && matches!(request.actor_kind, AutomaticWorker | Operator)
                && matches!(request.poisoning_verdict, UpstreamValidated | Acknowledged)
                && (request.actor_kind != AutomaticWorker
                    || request.source_trust.allows_auto_promote())
        }
        DreamConsolidation => {
            request.actor_kind == AutomaticWorker
                && request.provenance_kind == Generated
                && request.source_trust == SourceTrustClass::ExternalContent
                && request.poisoning_verdict == UpstreamValidated
        }
        PackImport => {
            request.actor_kind == Operator
                && request.provenance_kind == Pack
                && request.source_trust == SourceTrustClass::Pack
                && request.poisoning_verdict == UpstreamValidated
        }
        ActivationRouteKind::BackupImport => {
            request.actor_kind == Operator
                && request.provenance_kind == Backup
                && matches!(
                    request.source_trust,
                    SourceTrustClass::ExternalContent | SourceTrustClass::RepoFile
                )
                && request.poisoning_verdict == Clean
        }
        ScopeCleanup => {
            request.actor_kind == Operator
                && request.provenance_kind == ScopePlan
                && request.poisoning_verdict == UpstreamValidated
        }
        ActivationRouteKind::WebRestore => {
            request.actor_kind == Operator
                && request.provenance_kind == ActivationProvenanceKind::WebArchive
                && request.poisoning_verdict == ExactRecovery
        }
        ActivationRouteKind::ExactRecovery => {
            request.actor_kind == Operator
                && request.provenance_kind == ActivationProvenanceKind::ExactRecovery
                && request.poisoning_verdict == ExactRecovery
        }
    };
    if !valid {
        bail!("memory activation route has invalid actor/trust/provenance/poisoning policy");
    }
    Ok(())
}

#[cfg(test)]
mod batch_tests;
#[cfg(test)]
mod tests;
