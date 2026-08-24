use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::memory::activation::{
    ActivationActorKind, ActivationPoisoningVerdict, ActivationProvenanceKind, ActivationRouteKind,
    ActiveMemoryRoute, ActiveMemoryWriteRequest, ExpectedActiveMemory,
};
use crate::memory::poisoning::SourceTrustClass;

use super::super::merge::MergeResult;

pub(super) struct PayloadIdentities {
    pub(super) current: String,
    pub(super) replay_candidates: Vec<String>,
    pub(super) caller_superseded_ids: Vec<i64>,
}

pub(super) fn payload_identities(project: &str, result: &MergeResult) -> Result<PayloadIdentities> {
    let mut seen_ids = std::collections::HashSet::new();
    let ordered_ids = result
        .superseded_ids
        .iter()
        .copied()
        .filter(|id| seen_ids.insert(*id))
        .collect::<Vec<_>>();
    if ordered_ids.iter().any(|id| *id <= 0) {
        anyhow::bail!("dream superseded memory ids must be positive integers");
    }
    let sorted_ids = ordered_ids
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let current = payload_sha256_for_ids(project, result, &sorted_ids)?;
    let mut replay_candidates = vec![current.clone()];
    let mut seen_payloads = std::collections::HashSet::from([current.clone()]);
    for excluded in std::iter::once(None).chain(ordered_ids.iter().copied().map(Some)) {
        let legacy_ids = ordered_ids
            .iter()
            .copied()
            .filter(|id| Some(*id) != excluded)
            .collect::<Vec<_>>();
        let payload = payload_sha256_for_ids(project, result, &legacy_ids)?;
        if seen_payloads.insert(payload.clone()) {
            replay_candidates.push(payload);
        }
    }
    Ok(PayloadIdentities {
        current,
        replay_candidates,
        caller_superseded_ids: sorted_ids.into_iter().collect(),
    })
}

fn payload_sha256_for_ids(
    project: &str,
    result: &MergeResult,
    superseded_ids: &impl serde::Serialize,
) -> Result<String> {
    let superseded_json = serde_json::to_string(superseded_ids)?;
    Ok(crate::memory::activation::payload_sha256(&[
        project,
        &result.topic_key,
        &result.title,
        &result.content,
        &result.memory_type,
        &superseded_json,
    ]))
}

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
