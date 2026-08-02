use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DreamClusterMemberSnapshot {
    pub id: i64,
    pub version: i64,
    pub updated_at_epoch: i64,
    pub topic_key: Option<String>,
    pub title: String,
    pub content: String,
}

pub(crate) enum DreamDecisionPayload<'a> {
    Merge {
        topic_key: &'a str,
        memory_type: &'a str,
        title: &'a str,
        content: &'a str,
        intended_superseded_ids: &'a [i64],
    },
    NoMerge {
        reason: &'a str,
    },
    Conflict {
        conflicting_ids: &'a [i64],
        reason: &'a str,
    },
}

pub(crate) fn decision_payload_sha256(payload: DreamDecisionPayload<'_>) -> String {
    let mut hasher = Sha256::new();
    feed(&mut hasher, "dream-decision-payload-v1");
    match payload {
        DreamDecisionPayload::Merge {
            topic_key,
            memory_type,
            title,
            content,
            intended_superseded_ids,
        } => {
            feed(&mut hasher, "merge");
            feed(&mut hasher, topic_key);
            feed(&mut hasher, memory_type);
            feed(&mut hasher, title);
            feed(&mut hasher, content);
            feed_ids(&mut hasher, intended_superseded_ids);
        }
        DreamDecisionPayload::NoMerge { reason } => {
            feed(&mut hasher, "no_merge");
            feed(&mut hasher, reason);
            feed_ids(&mut hasher, &[]);
        }
        DreamDecisionPayload::Conflict {
            conflicting_ids,
            reason,
        } => {
            feed(&mut hasher, "conflict");
            feed(&mut hasher, reason);
            feed_ids(&mut hasher, conflicting_ids);
        }
    }
    format!("{:x}", hasher.finalize())
}

pub(crate) fn cluster_signature_sha256(
    project: &str,
    memory_type: &str,
    members: &[DreamClusterMemberSnapshot],
) -> String {
    let mut sorted = members.iter().collect::<Vec<_>>();
    sorted.sort_by_key(|member| member.id);

    let mut hasher = Sha256::new();
    feed(&mut hasher, "dream-cluster-v2");
    feed(&mut hasher, project);
    feed(&mut hasher, memory_type);
    feed(&mut hasher, &sorted.len().to_string());
    for member in sorted {
        feed(&mut hasher, &member.id.to_string());
        feed(&mut hasher, &member.version.to_string());
        feed(&mut hasher, &member.updated_at_epoch.to_string());
        feed_optional(&mut hasher, member.topic_key.as_deref());
        feed(&mut hasher, &member.title);
        feed(&mut hasher, &member.content);
    }
    format!("sha256:{:x}", hasher.finalize())
}

pub(crate) fn quarantine_semantic_discriminator_sha256(
    cluster_signature: &str,
    decision_payload_sha256: &str,
    generated_field: &str,
    pattern_id: &str,
    pattern_version: i64,
) -> String {
    let mut hasher = Sha256::new();
    feed(&mut hasher, "dream-quarantine-semantic-v1");
    feed(&mut hasher, cluster_signature);
    feed(&mut hasher, decision_payload_sha256);
    feed(&mut hasher, generated_field);
    feed(&mut hasher, pattern_id);
    feed(&mut hasher, &pattern_version.to_string());
    format!("{:x}", hasher.finalize())
}

fn feed_ids(hasher: &mut Sha256, ids: &[i64]) {
    hasher.update((ids.len() as u64).to_be_bytes());
    for id in ids {
        feed(hasher, &id.to_string());
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_hash_binds_field_boundaries_and_conflict_ids() {
        let first = decision_payload_sha256(DreamDecisionPayload::Merge {
            topic_key: "topic",
            memory_type: "decision",
            title: "a\nb",
            content: "c",
            intended_superseded_ids: &[1, 2],
        });
        let changed_boundary = decision_payload_sha256(DreamDecisionPayload::Merge {
            topic_key: "topic",
            memory_type: "decision",
            title: "a",
            content: "b\nc",
            intended_superseded_ids: &[1, 2],
        });
        let changed_ids = decision_payload_sha256(DreamDecisionPayload::Conflict {
            conflicting_ids: &[1, 3],
            reason: "same reason",
        });
        let original_ids = decision_payload_sha256(DreamDecisionPayload::Conflict {
            conflicting_ids: &[1, 2],
            reason: "same reason",
        });

        assert_ne!(first, changed_boundary);
        assert_ne!(changed_ids, original_ids);
    }

    #[test]
    fn cluster_hash_binds_member_version_and_canonical_payload() {
        let base = DreamClusterMemberSnapshot {
            id: 1,
            version: 1,
            updated_at_epoch: 100,
            topic_key: Some("topic".to_string()),
            title: "title".to_string(),
            content: "content".to_string(),
        };
        let original = cluster_signature_sha256("project", "decision", std::slice::from_ref(&base));
        let mut changed_version = base.clone();
        changed_version.version += 1;
        let mut changed_content = base;
        changed_content.content.push_str(" changed");

        assert_ne!(
            original,
            cluster_signature_sha256("project", "decision", &[changed_version])
        );
        assert_ne!(
            original,
            cluster_signature_sha256("project", "decision", &[changed_content])
        );
    }

    #[test]
    fn quarantine_semantic_hash_binds_pattern_and_decision() {
        let original = quarantine_semantic_discriminator_sha256(
            "sha256:cluster",
            "decision-a",
            "dream.title",
            "pattern",
            1,
        );
        assert_ne!(
            original,
            quarantine_semantic_discriminator_sha256(
                "sha256:cluster",
                "decision-b",
                "dream.title",
                "pattern",
                1,
            )
        );
        assert_ne!(
            original,
            quarantine_semantic_discriminator_sha256(
                "sha256:cluster",
                "decision-a",
                "dream.title",
                "pattern",
                2,
            )
        );
    }
}
