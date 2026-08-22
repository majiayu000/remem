use anyhow::{bail, Result};
use rusqlite::{params, Connection};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{
    ActivationActorKind, ActivationPoisoningVerdict, ActivationProvenanceKind, ActivationRouteKind,
    ActiveMemoryWriteRequest, ExpectedActiveMemory,
};

#[derive(Serialize)]
struct V086RequestFingerprint<'a> {
    route_kind: ActivationRouteKind,
    actor_kind: ActivationActorKind,
    source_operation: &'a str,
    source_trust_class: &'static str,
    source_project: &'a str,
    project: &'a str,
    branch_present: bool,
    branch: Option<&'a str>,
    scope: &'a str,
    owner_scope: &'a str,
    owner_key: &'a str,
    target_project: Option<&'a str>,
    provenance_kind: ActivationProvenanceKind,
    provenance_ref: &'a str,
    payload_sha256: &'a str,
    expected_memory: &'a ExpectedActiveMemory,
    poisoning_verdict: ActivationPoisoningVerdict,
    superseded_ids: &'a [i64],
}

#[derive(Serialize)]
struct V087RequestFingerprint<'a> {
    route_kind: ActivationRouteKind,
    actor_kind: ActivationActorKind,
    source_operation: &'a str,
    source_trust_class: &'static str,
    result_source_trust_class: &'static str,
    source_project: &'a str,
    project: &'a str,
    branch_present: bool,
    branch: Option<&'a str>,
    scope: &'a str,
    owner_scope: &'a str,
    owner_key: &'a str,
    target_project: Option<&'a str>,
    provenance_kind: ActivationProvenanceKind,
    provenance_ref: &'a str,
    payload_sha256: &'a str,
    expected_memory: &'a ExpectedActiveMemory,
    poisoning_verdict: ActivationPoisoningVerdict,
    superseded_ids: &'a [i64],
}

#[derive(Serialize)]
struct SupplementalRequestFingerprint<'a> {
    fingerprint_version: &'static str,
    route_kind: ActivationRouteKind,
    actor_kind: ActivationActorKind,
    source_operation: &'a str,
    source_trust_class: &'static str,
    source_project: &'a str,
    project: &'a str,
    branch_present: bool,
    branch: Option<&'a str>,
    scope: &'a str,
    owner_scope: &'a str,
    owner_key: &'a str,
    target_project: Option<&'a str>,
    provenance_kind: ActivationProvenanceKind,
    provenance_ref: &'a str,
    payload_sha256: &'a str,
    poisoning_verdict: ActivationPoisoningVerdict,
    superseded_ids: &'a [i64],
}

pub(super) fn current_request_sha256(
    request: &ActiveMemoryWriteRequest,
    superseded_ids: &[i64],
) -> Result<String> {
    if request.route_kind != ActivationRouteKind::SupplementalSave {
        return v087_request_sha256(request, superseded_ids);
    }
    digest(&SupplementalRequestFingerprint {
        fingerprint_version: "supplemental-request-v2",
        route_kind: request.route_kind,
        actor_kind: request.actor_kind,
        source_operation: &request.source_operation,
        source_trust_class: request.source_trust.as_str(),
        source_project: &request.source_project,
        project: &request.route.project,
        branch_present: request.route.branch.is_some(),
        branch: request.route.branch.as_deref(),
        scope: &request.route.scope,
        owner_scope: &request.route.owner_scope,
        owner_key: &request.route.owner_key,
        target_project: request.route.target_project.as_deref(),
        provenance_kind: request.provenance_kind,
        provenance_ref: &request.provenance_ref,
        payload_sha256: &request.payload_sha256,
        poisoning_verdict: request.poisoning_verdict,
        superseded_ids,
    })
}

pub(super) fn v087_request_sha256(
    request: &ActiveMemoryWriteRequest,
    superseded_ids: &[i64],
) -> Result<String> {
    digest(&V087RequestFingerprint {
        route_kind: request.route_kind,
        actor_kind: request.actor_kind,
        source_operation: &request.source_operation,
        source_trust_class: request.source_trust.as_str(),
        result_source_trust_class: request.result_source_trust.as_str(),
        source_project: &request.source_project,
        project: &request.route.project,
        branch_present: request.route.branch.is_some(),
        branch: request.route.branch.as_deref(),
        scope: &request.route.scope,
        owner_scope: &request.route.owner_scope,
        owner_key: &request.route.owner_key,
        target_project: request.route.target_project.as_deref(),
        provenance_kind: request.provenance_kind,
        provenance_ref: &request.provenance_ref,
        payload_sha256: &request.payload_sha256,
        expected_memory: &request.expected_memory,
        poisoning_verdict: request.poisoning_verdict,
        superseded_ids,
    })
}

pub(super) fn v086_request_sha256(
    request: &ActiveMemoryWriteRequest,
    superseded_ids: &[i64],
) -> Result<String> {
    digest(&V086RequestFingerprint {
        route_kind: request.route_kind,
        actor_kind: request.actor_kind,
        source_operation: &request.source_operation,
        source_trust_class: request.source_trust.as_str(),
        source_project: &request.source_project,
        project: &request.route.project,
        branch_present: request.route.branch.is_some(),
        branch: request.route.branch.as_deref(),
        scope: &request.route.scope,
        owner_scope: &request.route.owner_scope,
        owner_key: &request.route.owner_key,
        target_project: request.route.target_project.as_deref(),
        provenance_kind: request.provenance_kind,
        provenance_ref: &request.provenance_ref,
        payload_sha256: &request.payload_sha256,
        expected_memory: &request.expected_memory,
        poisoning_verdict: request.poisoning_verdict,
        superseded_ids,
    })
}

fn digest(value: &impl Serialize) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}

pub(super) fn supplemental_request_matches_receipt(
    conn: &Connection,
    request: &ActiveMemoryWriteRequest,
    superseded_ids: &[i64],
) -> Result<bool> {
    let superseded_ids_json = serde_json::to_string(superseded_ids)?;
    conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM memory_activation_requests
             WHERE activation_id = ?1 AND route_kind = ?2 AND actor_kind = ?3
               AND source_operation = ?4 AND source_trust_class = ?5
               AND source_project = ?6 AND project = ?7
               AND branch_present = ?8 AND branch IS ?9 AND scope = ?10
               AND owner_scope = ?11 AND owner_key = ?12 AND target_project IS ?13
               AND provenance_kind = ?14 AND provenance_ref = ?15
               AND payload_sha256 = ?16 AND poisoning_verdict = ?17
               AND superseded_ids_json = ?18
         )",
        params![
            request.activation_id,
            super::enum_json(request.route_kind)?,
            super::enum_json(request.actor_kind)?,
            request.source_operation,
            request.source_trust.as_str(),
            request.source_project,
            request.route.project,
            i64::from(request.route.branch.is_some()),
            request.route.branch,
            request.route.scope,
            request.route.owner_scope,
            request.route.owner_key,
            request.route.target_project,
            super::enum_json(request.provenance_kind)?,
            request.provenance_ref,
            request.payload_sha256,
            super::enum_json(request.poisoning_verdict)?,
            superseded_ids_json,
        ],
        |row| row.get::<_, i64>(0),
    )
    .map(|matches| matches == 1)
    .map_err(Into::into)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SupplementalSaveReceipt {
    Saved { claim_id: i64 },
    Disabled,
    Failed { error: String },
}

impl SupplementalSaveReceipt {
    pub(crate) fn saved(claim_id: i64) -> Result<Self> {
        if claim_id <= 0 {
            bail!("supplemental save receipt claim id must be positive");
        }
        Ok(Self::Saved { claim_id })
    }

    pub(crate) fn failed(error: impl Into<String>) -> Result<Self> {
        let error = error.into();
        if error.trim().is_empty() || error.contains('\0') {
            bail!("supplemental save receipt failure must be nonblank and contain no NUL");
        }
        Ok(Self::Failed { error })
    }

    pub(crate) fn status(&self) -> &'static str {
        match self {
            Self::Saved { .. } => "saved",
            Self::Disabled => "disabled",
            Self::Failed { .. } => "failed",
        }
    }

    pub(crate) fn claim_id(&self) -> Option<i64> {
        match self {
            Self::Saved { claim_id } => Some(*claim_id),
            Self::Disabled | Self::Failed { .. } => None,
        }
    }

    pub(crate) fn error(&self) -> Option<&str> {
        match self {
            Self::Failed { error } => Some(error),
            Self::Saved { .. } | Self::Disabled => None,
        }
    }

    pub(super) fn from_columns(
        status: Option<String>,
        claim_id: Option<i64>,
        error: Option<String>,
    ) -> Result<Option<Self>> {
        match (status.as_deref(), claim_id, error) {
            (None, None, None) => Ok(None),
            (Some("saved"), Some(claim_id), None) => Self::saved(claim_id).map(Some),
            (Some("disabled"), None, None) => Ok(Some(Self::Disabled)),
            (Some("failed"), None, Some(error)) => Self::failed(error).map(Some),
            _ => bail!("stored supplemental save receipt has an invalid field combination"),
        }
    }
}
