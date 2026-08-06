use std::collections::BTreeSet;

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

const DREAM_SOURCE_KIND: &str = "dream_model_output";
const DREAM_SOURCE_OPERATION: &str = "dream";
const DREAM_TRUST_CLASS: &str = "external_content";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DreamQuarantineArtifact {
    pub artifact_id: i64,
    pub version: i64,
    pub project: String,
    pub cluster_signature: String,
    pub member_ids: Vec<i64>,
    pub decision_kind: String,
    pub decision_ids: Vec<i64>,
    pub decision_payload_sha256: String,
    pub intended_superseded_ids: Vec<i64>,
    pub generated_topic_key: Option<String>,
    pub generated_memory_type: Option<String>,
    pub generated_title: Option<String>,
    pub generated_content: Option<String>,
    pub generated_field: String,
    pub pattern_id: String,
    pub pattern_version: i64,
    pub source_operation: String,
    pub source_trust_class: String,
    pub occurrence_count: i64,
    pub created_at_epoch: i64,
    pub updated_at_epoch: i64,
    /// GH-990: set only on stock-backfill artifacts; binds the artifact to
    /// the pre-v076 Dream-merged memory it retired pending review.
    pub backfill_memory_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DreamQuarantineProvenance {
    pub artifacts: Vec<DreamQuarantineArtifact>,
    pub authorized_supersede_ids: Vec<i64>,
    pub merge_payload: Option<DreamMergePayload>,
    pub review_token: Option<String>,
    pub blocked_reasons: Vec<String>,
    /// Distinct pre-v076 stock memories bound by these artifacts (GH-990).
    /// Empty for forward-path candidates; exactly one id for a restorable
    /// stock-backfill candidate.
    pub backfill_memory_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DreamMergePayload {
    pub topic_key: String,
    pub memory_type: String,
    pub title: String,
    pub content: String,
}

impl DreamQuarantineProvenance {
    pub(crate) fn approval_blocked_reasons(&self) -> Vec<String> {
        let mut reasons = Vec::new();
        for artifact in &self.artifacts {
            match artifact.decision_kind.as_str() {
                "merge" => {}
                "no_merge" => {
                    push_provenance_reason(&mut reasons, "dream_decision_no_merge_not_approvable")
                }
                "conflict" => {
                    push_provenance_reason(&mut reasons, "dream_decision_conflict_not_approvable")
                }
                _ => push_provenance_reason(&mut reasons, "dream_decision_kind_not_approvable"),
            }
        }
        reasons
    }
}

#[derive(Debug)]
struct CandidateIdentity {
    project: Option<String>,
    source_project: Option<String>,
    target_project: Option<String>,
    owner_scope: Option<String>,
    owner_key: Option<String>,
    memory_type: String,
    topic_key: String,
    text: String,
    source_kind: Option<String>,
    source_trust_class: String,
    quarantine_pattern_id: Option<String>,
    quarantine_pattern_version: Option<i64>,
    review_status: String,
    version: i64,
}

#[derive(Debug)]
struct RawDreamArtifact {
    artifact_id: i64,
    version: i64,
    project: String,
    cluster_signature: String,
    member_ids_json: String,
    decision_kind: String,
    decision_ids_json: String,
    decision_payload_sha256: String,
    intended_superseded_ids_json: String,
    generated_topic_key: Option<String>,
    generated_memory_type: Option<String>,
    generated_title: Option<String>,
    generated_content: Option<String>,
    generated_field: String,
    pattern_id: String,
    pattern_version: i64,
    source_operation: String,
    source_trust_class: String,
    occurrence_count: i64,
    created_at_epoch: i64,
    updated_at_epoch: i64,
    backfill_memory_id: Option<i64>,
}

pub(crate) fn load_dream_quarantine_provenance(
    conn: &Connection,
    candidate_id: i64,
) -> Result<Option<DreamQuarantineProvenance>> {
    let identity = conn
        .query_row(
            "SELECT p.project_path, c.source_project, c.target_project,
                    c.owner_scope, c.owner_key, c.memory_type, c.topic_key,
                    c.text, c.source_kind, c.source_trust_class,
                    c.quarantine_pattern_id, c.quarantine_pattern_version,
                    c.review_status, c.version
             FROM memory_candidates c
             LEFT JOIN projects p ON p.id = c.project_id
             WHERE c.id = ?1",
            params![candidate_id],
            |row| {
                Ok(CandidateIdentity {
                    project: row.get(0)?,
                    source_project: row.get(1)?,
                    target_project: row.get(2)?,
                    owner_scope: row.get(3)?,
                    owner_key: row.get(4)?,
                    memory_type: row.get(5)?,
                    topic_key: row.get(6)?,
                    text: row.get(7)?,
                    source_kind: row.get(8)?,
                    source_trust_class: row.get(9)?,
                    quarantine_pattern_id: row.get(10)?,
                    quarantine_pattern_version: row.get(11)?,
                    review_status: row.get(12)?,
                    version: row.get(13)?,
                })
            },
        )
        .optional()
        .context("load Dream candidate identity")?;
    let Some(identity) = identity else {
        return Ok(None);
    };
    if identity.source_kind.as_deref() != Some(DREAM_SOURCE_KIND) {
        return Ok(None);
    }

    let mut blocked_reasons = Vec::new();
    let candidate_project = identity.project.as_deref();
    if candidate_project.is_none()
        || identity.source_project.as_deref() != candidate_project
        || identity.target_project.as_deref() != candidate_project
        || identity.owner_scope.as_deref() != Some("repo")
        || identity.owner_key.as_deref() != candidate_project
        || identity.source_trust_class != DREAM_TRUST_CLASS
    {
        push_provenance_reason(&mut blocked_reasons, "dream_provenance_project_mismatch");
    }

    let mut stmt = conn.prepare(
        "SELECT id, version, project, cluster_signature, member_ids_json,
                decision_kind, decision_ids_json, decision_payload_sha256,
                intended_superseded_ids_json, generated_topic_key,
                generated_memory_type, generated_title, generated_content,
                generated_field, pattern_id, pattern_version, source_operation,
                source_trust_class, occurrence_count, created_at_epoch,
                updated_at_epoch, backfill_memory_id
         FROM dream_quarantine_artifacts
         WHERE source_candidate_id = ?1
         ORDER BY id",
    )?;
    let rows = stmt.query_map(params![candidate_id], |row| {
        Ok(RawDreamArtifact {
            artifact_id: row.get(0)?,
            version: row.get(1)?,
            project: row.get(2)?,
            cluster_signature: row.get(3)?,
            member_ids_json: row.get(4)?,
            decision_kind: row.get(5)?,
            decision_ids_json: row.get(6)?,
            decision_payload_sha256: row.get(7)?,
            intended_superseded_ids_json: row.get(8)?,
            generated_topic_key: row.get(9)?,
            generated_memory_type: row.get(10)?,
            generated_title: row.get(11)?,
            generated_content: row.get(12)?,
            generated_field: row.get(13)?,
            pattern_id: row.get(14)?,
            pattern_version: row.get(15)?,
            source_operation: row.get(16)?,
            source_trust_class: row.get(17)?,
            occurrence_count: row.get(18)?,
            created_at_epoch: row.get(19)?,
            updated_at_epoch: row.get(20)?,
            backfill_memory_id: row.get(21)?,
        })
    })?;

    let mut artifacts = Vec::new();
    let mut authorized_ids = BTreeSet::new();
    let mut merge_payload: Option<DreamMergePayload> = None;
    for row in rows {
        let row = row?;
        let member_ids = match parse_positive_unique_ids(&row.member_ids_json, false) {
            Ok(ids) => ids,
            Err(()) => {
                push_provenance_reason(&mut blocked_reasons, "dream_provenance_malformed");
                Vec::new()
            }
        };
        let intended_superseded_ids =
            match parse_positive_unique_ids(&row.intended_superseded_ids_json, true) {
                Ok(ids) => ids,
                Err(()) => {
                    push_provenance_reason(&mut blocked_reasons, "dream_provenance_malformed");
                    Vec::new()
                }
            };
        let decision_ids = match parse_positive_unique_ids(&row.decision_ids_json, true) {
            Ok(ids) => ids,
            Err(()) => {
                push_provenance_reason(&mut blocked_reasons, "dream_provenance_malformed");
                Vec::new()
            }
        };
        let member_id_set = member_ids.iter().copied().collect::<BTreeSet<_>>();
        let generated_merge_fields = (
            row.generated_topic_key.as_deref(),
            row.generated_memory_type.as_deref(),
            row.generated_title.as_deref(),
            row.generated_content.as_deref(),
        );
        let decision_shape_valid = match row.decision_kind.as_str() {
            "merge" => {
                !intended_superseded_ids.is_empty()
                    && decision_ids == intended_superseded_ids
                    && intended_superseded_ids
                        .iter()
                        .all(|id| member_id_set.contains(id))
                    && matches!(generated_merge_fields, (Some(_), Some(_), Some(_), Some(_)))
            }
            "no_merge" => {
                intended_superseded_ids.is_empty()
                    && decision_ids.is_empty()
                    && generated_merge_fields == (None, None, None, None)
            }
            "conflict" => {
                intended_superseded_ids.is_empty()
                    && decision_ids.len() >= 2
                    && decision_ids.iter().all(|id| member_id_set.contains(id))
                    && generated_merge_fields == (None, None, None, None)
            }
            _ => false,
        };
        let payload_matches = decision_shape_valid
            && decision_payload_matches_candidate(&row, &decision_ids, &identity);
        if row.version <= 0
            || row.project.trim().is_empty()
            || row.cluster_signature.trim().is_empty()
            || !valid_generated_field(&row.generated_field)
            || row.pattern_id.trim().is_empty()
            || row.pattern_version <= 0
            || row.source_operation != DREAM_SOURCE_OPERATION
            || row.source_trust_class != DREAM_TRUST_CLASS
            || row.occurrence_count <= 0
            || !decision_shape_valid
            || !payload_matches
            || row.created_at_epoch <= 0
            || row.updated_at_epoch < row.created_at_epoch
        {
            push_provenance_reason(&mut blocked_reasons, "dream_provenance_malformed");
        }
        if Some(row.project.as_str()) != candidate_project {
            push_provenance_reason(&mut blocked_reasons, "dream_provenance_project_mismatch");
        }
        if identity.quarantine_pattern_id.as_deref() != Some(row.pattern_id.as_str())
            || identity.quarantine_pattern_version != Some(row.pattern_version)
        {
            push_provenance_reason(&mut blocked_reasons, "dream_provenance_pattern_mismatch");
        }
        if let Some(backfill_id) = row.backfill_memory_id {
            // A stock-backfill artifact binds exactly one member: the retired
            // pre-v076 memory itself (GH-990).
            if member_ids != vec![backfill_id] {
                push_provenance_reason(&mut blocked_reasons, "dream_provenance_malformed");
            }
        }
        let mut member_snapshots = Vec::with_capacity(member_ids.len());
        for member_id in &member_ids {
            if let Some(snapshot) = validate_member(
                conn,
                *member_id,
                &row.project,
                &identity.memory_type,
                row.backfill_memory_id.is_some(),
                &mut blocked_reasons,
            )? {
                member_snapshots.push(snapshot);
            }
        }
        // The forward-path staleness probe recomputes the signature from
        // current member snapshots. A backfill artifact's only member is the
        // retired stock memory, whose version and timestamps necessarily moved
        // when the backfill archived it, so the signature can never match;
        // restore-time payload comparison covers integrity instead (GH-990).
        if row.backfill_memory_id.is_none()
            && member_snapshots.len() == member_ids.len()
            && crate::dream::cluster_signature_sha256(
                &row.project,
                &identity.memory_type,
                &member_snapshots,
            ) != row.cluster_signature
        {
            push_provenance_reason(&mut blocked_reasons, "dream_provenance_stale");
        }
        if row.decision_kind == "merge" && decision_shape_valid && payload_matches {
            authorized_ids.extend(intended_superseded_ids.iter().copied());
            let current_payload = match (
                row.generated_topic_key.clone(),
                row.generated_memory_type.clone(),
                row.generated_title.clone(),
                row.generated_content.clone(),
            ) {
                (Some(topic_key), Some(memory_type), Some(title), Some(content)) => {
                    Some(DreamMergePayload {
                        topic_key,
                        memory_type,
                        title,
                        content,
                    })
                }
                _ => None,
            };
            if current_payload.is_none() {
                push_provenance_reason(&mut blocked_reasons, "dream_provenance_malformed");
            } else if merge_payload
                .as_ref()
                .zip(current_payload.as_ref())
                .is_some_and(|(existing, current)| existing != current)
            {
                push_provenance_reason(&mut blocked_reasons, "dream_provenance_payload_mismatch");
            } else if merge_payload.is_none() {
                merge_payload = current_payload;
            }
        }
        artifacts.push(DreamQuarantineArtifact {
            artifact_id: row.artifact_id,
            version: row.version,
            project: row.project,
            cluster_signature: row.cluster_signature,
            member_ids,
            decision_kind: row.decision_kind,
            decision_ids,
            decision_payload_sha256: row.decision_payload_sha256,
            intended_superseded_ids,
            generated_topic_key: row.generated_topic_key,
            generated_memory_type: row.generated_memory_type,
            generated_title: row.generated_title,
            generated_content: row.generated_content,
            generated_field: row.generated_field,
            pattern_id: row.pattern_id,
            pattern_version: row.pattern_version,
            source_operation: row.source_operation,
            source_trust_class: row.source_trust_class,
            occurrence_count: row.occurrence_count,
            created_at_epoch: row.created_at_epoch,
            updated_at_epoch: row.updated_at_epoch,
            backfill_memory_id: row.backfill_memory_id,
        });
    }
    if artifacts.is_empty() {
        push_provenance_reason(&mut blocked_reasons, "dream_provenance_missing");
    }

    let authorized_supersede_ids = authorized_ids.into_iter().collect::<Vec<_>>();
    let backfill_memory_ids = artifacts
        .iter()
        .filter_map(|artifact| artifact.backfill_memory_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let backfill_artifact_count = artifacts
        .iter()
        .filter(|artifact| artifact.backfill_memory_id.is_some())
        .count();
    if !backfill_memory_ids.is_empty()
        && (backfill_memory_ids.len() != 1
            || backfill_artifact_count != 1
            || artifacts
                .iter()
                .any(|artifact| artifact.backfill_memory_id.is_none()))
    {
        push_provenance_reason(&mut blocked_reasons, "dream_backfill_provenance_duplicate");
    }
    let review_token = (blocked_reasons.is_empty()
        && matches!(
            identity.review_status.as_str(),
            "pending_review" | "quarantined"
        ))
    .then(|| canonical_review_token(candidate_id, identity.version, &artifacts));
    Ok(Some(DreamQuarantineProvenance {
        artifacts,
        authorized_supersede_ids,
        merge_payload,
        review_token,
        blocked_reasons,
        backfill_memory_ids,
    }))
}

fn parse_positive_unique_ids(raw: &str, allow_empty: bool) -> std::result::Result<Vec<i64>, ()> {
    let values: Vec<serde_json::Value> = serde_json::from_str(raw).map_err(|_| ())?;
    if values.is_empty() && !allow_empty {
        return Err(());
    }
    let mut ids = BTreeSet::new();
    for value in values {
        let id = value.as_i64().filter(|id| *id > 0).ok_or(())?;
        if !ids.insert(id) {
            return Err(());
        }
    }
    Ok(ids.into_iter().collect())
}

fn decision_payload_matches_candidate(
    artifact: &RawDreamArtifact,
    decision_ids: &[i64],
    candidate: &CandidateIdentity,
) -> bool {
    let expected = match artifact.decision_kind.as_str() {
        "merge" => {
            let (Some(topic_key), Some(memory_type), Some(title), Some(content)) = (
                artifact.generated_topic_key.as_deref(),
                artifact.generated_memory_type.as_deref(),
                artifact.generated_title.as_deref(),
                artifact.generated_content.as_deref(),
            ) else {
                return false;
            };
            if candidate.topic_key != topic_key
                || candidate.memory_type != memory_type
                || candidate.text != format!("{title}\n{content}")
            {
                return false;
            }
            crate::dream::decision_payload_sha256(crate::dream::DreamDecisionPayload::Merge {
                topic_key,
                memory_type,
                title,
                content,
                intended_superseded_ids: decision_ids,
            })
        }
        "no_merge" => {
            crate::dream::decision_payload_sha256(crate::dream::DreamDecisionPayload::NoMerge {
                reason: &candidate.text,
            })
        }
        "conflict" => {
            crate::dream::decision_payload_sha256(crate::dream::DreamDecisionPayload::Conflict {
                conflicting_ids: decision_ids,
                reason: &candidate.text,
            })
        }
        _ => return false,
    };
    artifact.decision_payload_sha256 == expected
}

fn validate_member(
    conn: &Connection,
    member_id: i64,
    project: &str,
    memory_type: &str,
    backfill: bool,
    blocked_reasons: &mut Vec<String>,
) -> Result<Option<crate::dream::DreamClusterMemberSnapshot>> {
    let row = conn
        .query_row(
            "SELECT project, memory_type, status, version, updated_at_epoch,
                    topic_key, title, content
             FROM memories WHERE id = ?1",
            params![member_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .optional()?;
    let Some((
        member_project,
        member_type,
        status,
        version,
        updated_at_epoch,
        topic_key,
        title,
        content,
    )) = row
    else {
        push_provenance_reason(blocked_reasons, "dream_provenance_member_missing");
        return Ok(None);
    };
    if member_project != project {
        push_provenance_reason(blocked_reasons, "dream_provenance_member_out_of_scope");
    }
    if member_type != memory_type {
        push_provenance_reason(blocked_reasons, "dream_provenance_member_type_mismatch");
    }
    if backfill {
        // The backfill retired this memory pending review; that retirement is
        // the expected state. Anything else (restored, decayed, deleted from
        // the active surface by another path) means the binding no longer
        // describes reality.
        if status != "archived" {
            push_provenance_reason(blocked_reasons, "dream_provenance_stale");
        }
    } else {
        if status != "active" {
            push_provenance_reason(blocked_reasons, "dream_provenance_stale");
        }
        if version <= 0 {
            push_provenance_reason(blocked_reasons, "dream_provenance_stale");
        }
        if !member_is_canonically_current(conn, member_id)? {
            push_provenance_reason(blocked_reasons, "dream_provenance_stale");
        }
    }
    Ok(Some(crate::dream::DreamClusterMemberSnapshot {
        id: member_id,
        version,
        updated_at_epoch,
        topic_key,
        title,
        content,
    }))
}

fn member_is_canonically_current(conn: &Connection, member_id: i64) -> Result<bool> {
    let current_filter =
        crate::memory::memory_current_filter_sql("m.status", "m.expires_at_epoch", false);
    let state_filter = crate::memory::memory_state_key_current_filter_sql("m");
    let policy_filter = crate::memory::suppression::memory_policy_filter_sql("m");
    let sql = format!(
        "SELECT EXISTS(
             SELECT 1 FROM memories m
             WHERE m.id = ?1
               AND {current_filter}
               AND {state_filter}
               AND {policy_filter}
         )"
    );
    conn.query_row(&sql, params![member_id], |row| row.get::<_, i64>(0))
        .map(|exists| exists != 0)
        .map_err(Into::into)
}

fn canonical_review_token(
    candidate_id: i64,
    candidate_version: i64,
    artifacts: &[DreamQuarantineArtifact],
) -> String {
    let mut hasher = Sha256::new();
    feed(&mut hasher, "dream-review-v4");
    feed(&mut hasher, &candidate_id.to_string());
    feed(&mut hasher, &candidate_version.to_string());
    feed(&mut hasher, &artifacts.len().to_string());
    for artifact in artifacts {
        feed(&mut hasher, &artifact.artifact_id.to_string());
        feed(&mut hasher, &artifact.version.to_string());
        feed(&mut hasher, &artifact.project);
        feed(&mut hasher, &artifact.cluster_signature);
        feed(&mut hasher, &artifact.member_ids.len().to_string());
        for member_id in &artifact.member_ids {
            feed(&mut hasher, &member_id.to_string());
        }
        feed(&mut hasher, &artifact.decision_kind);
        feed(&mut hasher, &artifact.decision_ids.len().to_string());
        for memory_id in &artifact.decision_ids {
            feed(&mut hasher, &memory_id.to_string());
        }
        feed(&mut hasher, &artifact.decision_payload_sha256);
        feed(
            &mut hasher,
            &artifact.intended_superseded_ids.len().to_string(),
        );
        for memory_id in &artifact.intended_superseded_ids {
            feed(&mut hasher, &memory_id.to_string());
        }
        feed_optional(&mut hasher, artifact.generated_topic_key.as_deref());
        feed_optional(&mut hasher, artifact.generated_memory_type.as_deref());
        feed_optional(&mut hasher, artifact.generated_title.as_deref());
        feed_optional(&mut hasher, artifact.generated_content.as_deref());
        feed(&mut hasher, &artifact.generated_field);
        feed(&mut hasher, &artifact.pattern_id);
        feed(&mut hasher, &artifact.pattern_version.to_string());
        feed(&mut hasher, &artifact.source_operation);
        feed(&mut hasher, &artifact.source_trust_class);
        feed(&mut hasher, &artifact.occurrence_count.to_string());
        feed(&mut hasher, &artifact.created_at_epoch.to_string());
        feed(&mut hasher, &artifact.updated_at_epoch.to_string());
        feed_optional_i64(&mut hasher, artifact.backfill_memory_id);
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn valid_generated_field(field: &str) -> bool {
    field.strip_prefix("dream.").is_some_and(|suffix| {
        !suffix.is_empty()
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
    })
}

fn feed(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value.as_bytes());
}

fn feed_optional(hasher: &mut Sha256, value: Option<&str>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            feed(hasher, value);
        }
        None => hasher.update([0]),
    }
}

fn feed_optional_i64(hasher: &mut Sha256, value: Option<i64>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            feed(hasher, &value.to_string());
        }
        None => hasher.update([0]),
    }
}

fn push_provenance_reason(reasons: &mut Vec<String>, reason: &str) {
    if !reasons.iter().any(|existing| existing == reason) {
        reasons.push(reason.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn merge_artifact() -> DreamQuarantineArtifact {
        DreamQuarantineArtifact {
            artifact_id: 7,
            version: 1,
            project: "/tmp/remem".to_string(),
            cluster_signature: "cluster-1".to_string(),
            member_ids: vec![11, 12],
            decision_kind: "merge".to_string(),
            decision_ids: vec![11],
            decision_payload_sha256: crate::dream::decision_payload_sha256(
                crate::dream::DreamDecisionPayload::Merge {
                    topic_key: "topic",
                    memory_type: "decision",
                    title: "title",
                    content: "content",
                    intended_superseded_ids: &[11],
                },
            ),
            intended_superseded_ids: vec![11],
            generated_topic_key: Some("topic".to_string()),
            generated_memory_type: Some("decision".to_string()),
            generated_title: Some("title".to_string()),
            generated_content: Some("content".to_string()),
            generated_field: "dream.title".to_string(),
            pattern_id: "override_previous_instructions".to_string(),
            pattern_version: 1,
            source_operation: "dream".to_string(),
            source_trust_class: "external_content".to_string(),
            occurrence_count: 1,
            created_at_epoch: 10,
            updated_at_epoch: 10,
            backfill_memory_id: None,
        }
    }

    #[test]
    fn review_token_binds_decision_kind_and_intended_supersede_ids() {
        let artifact = merge_artifact();
        let original = canonical_review_token(3, 4, std::slice::from_ref(&artifact));

        let mut changed_decision = artifact.clone();
        changed_decision.decision_kind = "no_merge".to_string();
        changed_decision.intended_superseded_ids.clear();
        assert_ne!(canonical_review_token(3, 4, &[changed_decision]), original);

        let mut changed_intended = artifact;
        changed_intended.intended_superseded_ids = vec![12];
        assert_ne!(canonical_review_token(3, 4, &[changed_intended]), original);
    }

    #[test]
    fn review_token_binds_structured_payload_and_decision_digest() {
        let artifact = merge_artifact();
        let original = canonical_review_token(3, 4, std::slice::from_ref(&artifact));

        let mut changed_title = artifact.clone();
        changed_title.generated_title = Some("changed".to_string());
        assert_ne!(canonical_review_token(3, 4, &[changed_title]), original);

        let mut changed_digest = artifact;
        changed_digest.decision_payload_sha256 = "0".repeat(64);
        assert_ne!(canonical_review_token(3, 4, &[changed_digest]), original);
    }

    #[test]
    fn review_token_binds_backfill_memory_id() {
        let artifact = merge_artifact();
        let original = canonical_review_token(3, 4, std::slice::from_ref(&artifact));

        let mut changed_binding = artifact;
        changed_binding.backfill_memory_id = Some(11);
        assert_ne!(canonical_review_token(3, 4, &[changed_binding]), original);
    }
}
