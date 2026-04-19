use anyhow::Result;
use rusqlite::Connection;

use super::constants::{
    DREAM_MAX_CLUSTERS, DREAM_MIN_CLUSTER_SIZE, DREAM_RECENCY_GUARD_SECS, TOPIC_KEY_PREFIX_LEN,
};

#[derive(Debug, Clone)]
pub(crate) struct MemoryCandidate {
    pub id: i64,
    pub topic_key: Option<String>,
    pub title: String,
    pub content: String,
    pub memory_type: String,
    #[allow(dead_code)]
    pub updated_at_epoch: i64,
}

/// A group of memories that are candidates for merging.
#[derive(Debug)]
pub(crate) struct Cluster {
    pub members: Vec<MemoryCandidate>,
}

/// Load active memories for a project and group them into merge candidates.
pub(super) fn load_clusters(conn: &Connection, project: &str) -> Result<Vec<Cluster>> {
    let cutoff = chrono::Utc::now().timestamp() - DREAM_RECENCY_GUARD_SECS;

    let mut stmt = conn.prepare(
        "SELECT id, topic_key, title, content, memory_type, updated_at_epoch
         FROM memories
         WHERE project = ?1
           AND status = 'active'
           AND updated_at_epoch < ?2
         ORDER BY memory_type, topic_key, updated_at_epoch DESC",
    )?;

    let candidates: Vec<MemoryCandidate> = stmt
        .query_map([project, &cutoff.to_string()], |row| {
            Ok(MemoryCandidate {
                id: row.get(0)?,
                topic_key: row.get(1)?,
                title: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                content: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                memory_type: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                updated_at_epoch: row.get(5)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();

    Ok(cluster_candidates(candidates))
}

fn cluster_candidates(candidates: Vec<MemoryCandidate>) -> Vec<Cluster> {
    use std::collections::HashMap;

    // Group by cluster key: topic_key prefix (or memory_type for NULL topic_key)
    let mut groups: HashMap<String, Vec<MemoryCandidate>> = HashMap::new();

    for c in candidates {
        let group_key = match &c.topic_key {
            Some(key) if !key.is_empty() => {
                // Truncate to prefix length for grouping
                key.chars().take(TOPIC_KEY_PREFIX_LEN).collect::<String>()
            }
            // NULL or empty topic_key: group by memory_type
            _ => format!("__unkeyed__{}", c.memory_type),
        };
        groups.entry(group_key).or_default().push(c);
    }

    // Keep only groups with ≥ MIN_CLUSTER_SIZE, limit total clusters
    let mut clusters: Vec<Cluster> = groups
        .into_values()
        .filter(|g| g.len() >= DREAM_MIN_CLUSTER_SIZE)
        .map(|members| Cluster { members })
        .collect();

    // Sort by cluster size descending (biggest benefit first)
    clusters.sort_by_key(|b| std::cmp::Reverse(b.members.len()));
    clusters.truncate(DREAM_MAX_CLUSTERS);
    clusters
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make(id: i64, topic_key: Option<&str>, memory_type: &str) -> MemoryCandidate {
        MemoryCandidate {
            id,
            topic_key: topic_key.map(str::to_owned),
            title: format!("title-{}", id),
            content: format!("content-{}", id),
            memory_type: memory_type.to_owned(),
            updated_at_epoch: 1000 + id,
        }
    }

    #[test]
    fn test_cluster_by_topic_key_prefix() {
        let candidates = vec![
            make(1, Some("auth-middleware-design-v1"), "decision"),
            make(2, Some("auth-middleware-design-v2"), "decision"),
            make(3, Some("totally-different-topic"), "decision"),
        ];
        let clusters = cluster_candidates(candidates);
        // auth-middleware-design-v1 and v2 share a 20-char prefix
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].members.len(), 2);
    }

    #[test]
    fn test_cluster_null_topic_key_by_type() {
        let candidates = vec![
            make(1, None, "preference"),
            make(2, None, "preference"),
            make(3, None, "decision"),
        ];
        let clusters = cluster_candidates(candidates);
        // 2 preference + 1 decision; only preference group has ≥ 2
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].members.len(), 2);
    }

    #[test]
    fn test_single_member_cluster_excluded() {
        let candidates = vec![
            make(1, Some("unique-key-aaa"), "decision"),
            make(2, Some("unique-key-bbb"), "decision"),
        ];
        let clusters = cluster_candidates(candidates);
        assert!(clusters.is_empty());
    }

    #[test]
    fn test_max_clusters_respected() {
        let candidates: Vec<MemoryCandidate> = (0..200)
            .map(|i| make(i, Some(&format!("topic-{:04}-suffix", i / 2)), "decision"))
            .collect();
        let clusters = cluster_candidates(candidates);
        assert!(clusters.len() <= DREAM_MAX_CLUSTERS);
    }
}
