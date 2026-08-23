use crate::memory::activation::{
    activation_id_from_key, ephemeral_activation_id, payload_sha256, ActivationActorKind,
    ActivationPoisoningVerdict, ActivationProvenanceKind, ActivationRouteKind, ActiveMemoryRoute,
    ActiveMemoryWriteRequest,
};
use crate::memory::poisoning::SourceTrustClass;
use crate::memory::{lifecycle::MemoryLifecycleOp, operation::MemoryOperationPlan};
use anyhow::Result;
use rusqlite::Connection;

use super::super::types::SaveMemoryRequest;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveMemoryCaller {
    RustApi,
    McpAgent,
    RestAgent,
}

impl SaveMemoryCaller {
    fn actor_kind(self) -> ActivationActorKind {
        match self {
            Self::RustApi => ActivationActorKind::RustApi,
            Self::McpAgent | Self::RestAgent => ActivationActorKind::Agent,
        }
    }

    pub(super) fn source_trust(self) -> SourceTrustClass {
        match self {
            Self::RustApi => SourceTrustClass::LocalToolOutput,
            Self::McpAgent | Self::RestAgent => SourceTrustClass::ExternalContent,
        }
    }

    fn provenance_ref(self) -> &'static str {
        match self {
            Self::RustApi => "rust-api:unattested",
            Self::McpAgent => "mcp:agent-unattested",
            Self::RestAgent => "rest:agent-unattested",
        }
    }

    fn namespace(self) -> &'static str {
        match self {
            Self::RustApi => "save-rust",
            Self::McpAgent => "save-mcp",
            Self::RestAgent => "save-rest",
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_request(
    req: &SaveMemoryRequest,
    caller: SaveMemoryCaller,
    project: &str,
    title: &str,
    memory_type: &str,
    scope: &str,
    effective_topic_key: Option<&str>,
    reference_time_epoch: Option<i64>,
    acknowledged_pattern: bool,
) -> ActiveMemoryWriteRequest {
    let files = req
        .files
        .as_ref()
        .map(|files| serde_json::to_string(files).unwrap_or_default());
    let created_at = req.created_at_epoch.map(|value| value.to_string());
    let reference_time = reference_time_epoch.map(|value| value.to_string());
    let payload_sha256 = payload_sha256(&[
        project,
        title,
        &req.text,
        memory_type,
        scope,
        if effective_topic_key.is_some() {
            "1"
        } else {
            "0"
        },
        effective_topic_key.unwrap_or(""),
        if req.branch.is_some() { "1" } else { "0" },
        req.branch.as_deref().unwrap_or(""),
        if req.session_id.is_some() { "1" } else { "0" },
        req.session_id.as_deref().unwrap_or(""),
        if req.host.is_some() { "1" } else { "0" },
        req.host.as_deref().unwrap_or(""),
        if files.is_some() { "1" } else { "0" },
        files.as_deref().unwrap_or(""),
        if created_at.is_some() { "1" } else { "0" },
        created_at.as_deref().unwrap_or(""),
        if reference_time.is_some() { "1" } else { "0" },
        reference_time.as_deref().unwrap_or(""),
        if req.acknowledge_pattern.is_some() {
            "1"
        } else {
            "0"
        },
        req.acknowledge_pattern.as_deref().unwrap_or(""),
        if req.local_path.is_some() { "1" } else { "0" },
        req.local_path.as_deref().unwrap_or(""),
        match req.local_copy_enabled {
            Some(true) => "true",
            Some(false) => "false",
            None => "default",
        },
        match req.claim_enabled {
            Some(true) => "true",
            Some(false) => "false",
            None => "default",
        },
        if req.claim_source.is_some() { "1" } else { "0" },
        req.claim_source.as_deref().unwrap_or(""),
    ]);
    let activation_id = req
        .idempotency_key
        .as_deref()
        .map(|key| activation_id_from_key(caller.namespace(), key))
        .unwrap_or_else(|| ephemeral_activation_id(caller.namespace(), &payload_sha256));

    ActiveMemoryWriteRequest {
        activation_id,
        route_kind: ActivationRouteKind::SupplementalSave,
        actor_kind: caller.actor_kind(),
        source_operation: "save_memory".to_string(),
        source_trust: caller.source_trust(),
        result_source_trust: caller.source_trust(),
        source_project: project.to_string(),
        route: ActiveMemoryRoute::default_for(project, req.branch.as_deref(), scope),
        provenance_kind: ActivationProvenanceKind::SupplementalSave,
        provenance_ref: caller.provenance_ref().to_string(),
        payload_sha256,
        expected_memory: crate::memory::activation::ExpectedActiveMemory::new(
            title,
            &req.text,
            memory_type,
        )
        .with_topic_key(effective_topic_key)
        .with_files(files.as_deref()),
        poisoning_verdict: if acknowledged_pattern {
            ActivationPoisoningVerdict::Acknowledged
        } else {
            ActivationPoisoningVerdict::Clean
        },
        superseded_ids: Vec::new(),
    }
}

pub(super) fn bind_existing_target_provenance(
    conn: &Connection,
    request: &mut ActiveMemoryWriteRequest,
    plan: &MemoryOperationPlan,
    memory_type: &str,
    incoming_text: &str,
) -> Result<()> {
    let Some(memory_id) = plan.target_memory_id else {
        return Ok(());
    };
    let existing_route = crate::memory::activation::load_existing_route(conn, memory_id)?;
    request.route = existing_route.route;
    request.source_project = existing_route.source_project;
    let existing = crate::memory::activation::ExpectedActiveMemory::from_existing(conn, memory_id)?;
    if plan.op == MemoryLifecycleOp::Noop && memory_type != "lesson" {
        request.expected_memory = existing;
        return Ok(());
    }
    let source_candidate_id = if memory_type == "preference"
        && !crate::memory::preference::reinforcement::cleanup_preserves_candidate_provenance(
            &existing.content,
            incoming_text,
        ) {
        None
    } else {
        existing.source_candidate_id
    };
    request.expected_memory.evidence_event_ids = existing.evidence_event_ids;
    request.expected_memory.source_candidate_id = source_candidate_id;
    Ok(())
}
