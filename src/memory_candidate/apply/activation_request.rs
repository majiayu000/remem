use crate::memory::activation::{
    activation_id_from_key, payload_sha256, ActivationActorKind, ActivationPoisoningVerdict,
    ActivationProvenanceKind, ActivationRouteKind, ActiveMemoryRoute, ActiveMemoryWriteRequest,
};
use crate::memory::poisoning::SourceTrustClass;

use super::{CandidateRoute, ParsedMemoryCandidate};

#[allow(clippy::too_many_arguments)]
pub(super) fn build(
    source_project: &str,
    memory_project: &str,
    memory_scope: &str,
    candidate_id: i64,
    candidate: &ParsedMemoryCandidate,
    evidence_json: &str,
    route: &CandidateRoute,
    source_trust: SourceTrustClass,
    actor_kind: ActivationActorKind,
    superseded_ids: &[i64],
    review_binding: Option<&str>,
    acknowledged_pattern: Option<(&str, i64)>,
) -> anyhow::Result<ActiveMemoryWriteRequest> {
    let candidate_id_text = candidate_id.to_string();
    let confidence = candidate.confidence.to_string();
    let superseded_json = serde_json::to_string(superseded_ids)?;
    let payload_sha256 = payload_sha256(&[
        source_project,
        &candidate_id_text,
        &candidate.scope,
        &candidate.memory_type,
        &candidate.topic_key,
        &candidate.text,
        evidence_json,
        memory_project,
        memory_scope,
        &route.owner_scope,
        &route.owner_key,
        route.target_project.as_deref().unwrap_or(""),
        &confidence,
        &superseded_json,
        review_binding.unwrap_or("automatic"),
        acknowledged_pattern.map(|value| value.0).unwrap_or("clean"),
        &acknowledged_pattern
            .map(|value| value.1)
            .unwrap_or_default()
            .to_string(),
    ]);
    Ok(ActiveMemoryWriteRequest {
        activation_id: activation_id_from_key(
            "candidate",
            &format!("{source_project}:{candidate_id}"),
        ),
        route_kind: ActivationRouteKind::CandidatePromotion,
        actor_kind,
        source_operation: "candidate_promotion".to_string(),
        source_trust,
        result_source_trust: source_trust,
        source_project: source_project.to_string(),
        route: ActiveMemoryRoute {
            project: memory_project.to_string(),
            branch: None,
            scope: memory_scope.to_string(),
            owner_scope: route.owner_scope.clone(),
            owner_key: route.owner_key.clone(),
            target_project: route.target_project.clone(),
        },
        provenance_kind: ActivationProvenanceKind::Candidate,
        provenance_ref: format!(
            "candidate:{candidate_id}:evidence:{payload_sha256}:review:{}",
            review_binding.unwrap_or("automatic")
        ),
        payload_sha256,
        expected_memory: crate::memory::activation::ExpectedActiveMemory::new(
            &super::candidate_title(candidate),
            &candidate.text,
            &candidate.memory_type,
        )
        .with_topic_key(Some(&candidate.topic_key))
        .with_candidate_evidence(Some(evidence_json), Some(candidate_id)),
        poisoning_verdict: if acknowledged_pattern.is_some() {
            ActivationPoisoningVerdict::Acknowledged
        } else {
            ActivationPoisoningVerdict::UpstreamValidated
        },
        superseded_ids: superseded_ids.to_vec(),
    })
}
