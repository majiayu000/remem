use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::memory::activation::{
    ActivationActorKind, ActivationPoisoningVerdict, ActivationProvenanceKind, ActivationRouteKind,
    ActiveMemoryRoute, ActiveMemoryWriteRequest, ExpectedActiveMemory,
};
use crate::memory::poisoning::SourceTrustClass;

pub(super) fn activation_request(
    project: &str,
    payload_sha256: String,
    expected_memory: ExpectedActiveMemory,
    superseded_ids: Vec<i64>,
) -> ActiveMemoryWriteRequest {
    ActiveMemoryWriteRequest {
        activation_id: crate::memory::activation::activation_id_from_key(
            "dream-consolidation",
            &payload_sha256,
        ),
        route_kind: ActivationRouteKind::DreamConsolidation,
        actor_kind: ActivationActorKind::AutomaticWorker,
        source_operation: "dream_consolidation".to_string(),
        source_trust: SourceTrustClass::ExternalContent,
        result_source_trust: SourceTrustClass::ExternalContent,
        source_project: project.to_string(),
        route: ActiveMemoryRoute {
            project: project.to_string(),
            branch: None,
            scope: "project".to_string(),
            owner_scope: "repo".to_string(),
            owner_key: project.to_string(),
            target_project: Some(project.to_string()),
        },
        provenance_kind: ActivationProvenanceKind::Generated,
        provenance_ref: format!("dream-generated:{payload_sha256}"),
        payload_sha256,
        expected_memory,
        poisoning_verdict: ActivationPoisoningVerdict::UpstreamValidated,
        superseded_ids,
    }
}

pub(super) fn reason(activation_id: &str) -> String {
    format!("dream consolidation applied activation={activation_id}")
}

pub(super) fn id_for_activation(
    conn: &Connection,
    memory_id: i64,
    activation_id: &str,
) -> Result<i64> {
    let reason = reason(activation_id);
    conn.query_row(
        "SELECT id FROM memory_operation_log
         WHERE source = 'dream' AND result_memory_id = ?1 AND reason = ?2",
        params![memory_id, reason],
        |row| row.get(0),
    )
    .context("dream replay is missing its activation-bound operation audit")
}
