use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use super::apply::ApplyOutcome;
use super::candidates::Cluster;
use super::constants::DREAM_NO_MERGE_REVIEW_SECS;

#[derive(Debug)]
pub(super) struct ClusterPlan {
    pub eligible: Vec<Cluster>,
    pub suppressed: usize,
}

pub(super) fn load_cluster_plan(
    conn: &Connection,
    project: &str,
    clusters: Vec<Cluster>,
) -> Result<ClusterPlan> {
    let now = chrono::Utc::now().timestamp();
    let mut eligible = Vec::new();
    let mut suppressed = 0usize;
    for cluster in clusters {
        if is_suppressed(conn, project, &cluster, now)? {
            suppressed += 1;
        } else {
            eligible.push(cluster);
        }
    }
    Ok(ClusterPlan {
        eligible,
        suppressed,
    })
}

pub(super) fn record_no_merge(
    conn: &Connection,
    project: &str,
    cluster: &Cluster,
    reason: Option<&str>,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    upsert_decision(
        conn,
        project,
        cluster,
        "no_merge",
        reason.unwrap_or("no merge returned by dream"),
        Some(now + DREAM_NO_MERGE_REVIEW_SECS),
        None,
    )
}

pub(super) fn record_failed(
    conn: &Connection,
    project: &str,
    cluster: &Cluster,
    reason: &str,
) -> Result<()> {
    upsert_decision(conn, project, cluster, "failed", reason, None, None)
}

pub(super) fn record_merged(
    conn: &Connection,
    project: &str,
    cluster: &Cluster,
    outcome: ApplyOutcome,
) -> Result<()> {
    upsert_decision(
        conn,
        project,
        cluster,
        "merged",
        "dream consolidation merged cluster",
        None,
        Some(outcome),
    )
}

fn is_suppressed(conn: &Connection, project: &str, cluster: &Cluster, now: i64) -> Result<bool> {
    let Some(memory_type) = cluster_memory_type(cluster) else {
        return Ok(false);
    };
    let signature = cluster_signature(project, cluster);
    let row: Option<(String, Option<i64>)> = conn
        .query_row(
            "SELECT decision, next_review_epoch
             FROM dream_cluster_decisions
             WHERE project = ?1
               AND memory_type = ?2
               AND cluster_signature = ?3",
            params![project, memory_type, signature],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((decision, next_review_epoch)) = row else {
        return Ok(false);
    };
    Ok(matches!(decision.as_str(), "no_merge" | "defer")
        && next_review_epoch.is_none_or(|epoch| epoch > now))
}

fn upsert_decision(
    conn: &Connection,
    project: &str,
    cluster: &Cluster,
    decision: &str,
    reason: &str,
    next_review_epoch: Option<i64>,
    outcome: Option<ApplyOutcome>,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let Some(memory_type) = cluster_memory_type(cluster) else {
        return Ok(());
    };
    let signature = cluster_signature(project, cluster);
    let member_ids_json = serde_json::to_string(&cluster_member_ids(cluster))?;
    conn.execute(
        "INSERT INTO dream_cluster_decisions
         (project, memory_type, cluster_signature, decision, reason, member_ids_json,
          cluster_size, next_review_epoch, source_memory_id, source_operation_id,
          created_at_epoch, updated_at_epoch, last_seen_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11, ?11)
         ON CONFLICT(project, memory_type, cluster_signature)
         DO UPDATE SET
             decision = excluded.decision,
             reason = excluded.reason,
             member_ids_json = excluded.member_ids_json,
             cluster_size = excluded.cluster_size,
             next_review_epoch = excluded.next_review_epoch,
             source_memory_id = excluded.source_memory_id,
             source_operation_id = excluded.source_operation_id,
             updated_at_epoch = excluded.updated_at_epoch,
             last_seen_epoch = excluded.last_seen_epoch",
        params![
            project,
            memory_type,
            signature,
            decision,
            crate::db::truncate_str(reason, 1000),
            member_ids_json,
            cluster.members.len() as i64,
            next_review_epoch,
            outcome.map(|value| value.merged_id),
            outcome.map(|value| value.operation_id),
            now
        ],
    )?;
    Ok(())
}

fn cluster_memory_type(cluster: &Cluster) -> Option<&str> {
    cluster
        .members
        .first()
        .map(|member| member.memory_type.as_str())
}

fn cluster_member_ids(cluster: &Cluster) -> Vec<i64> {
    let mut ids = cluster
        .members
        .iter()
        .map(|member| member.id)
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids
}

fn cluster_signature(project: &str, cluster: &Cluster) -> String {
    let mut parts = cluster
        .members
        .iter()
        .map(|member| format!("{}:{}", member.id, member.updated_at_epoch))
        .collect::<Vec<_>>();
    parts.sort();
    let memory_type = cluster_memory_type(cluster).unwrap_or("unknown");
    let raw = format!(
        "dream-cluster-v1\0{}\0{}\0{}",
        project,
        memory_type,
        parts.join("\0")
    );
    format!("{:016x}", crate::db::deterministic_hash(raw.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::tests_helper::setup_memory_schema;
    use rusqlite::Connection;

    fn member(id: i64, updated_at_epoch: i64) -> super::super::candidates::MemoryCandidate {
        super::super::candidates::MemoryCandidate {
            id,
            topic_key: Some("shared-topic".to_string()),
            title: format!("title-{id}"),
            content: format!("content-{id}"),
            memory_type: "decision".to_string(),
            updated_at_epoch,
        }
    }

    fn cluster(updated_at_epoch: i64) -> Cluster {
        Cluster {
            members: vec![member(1, updated_at_epoch), member(2, updated_at_epoch)],
        }
    }

    fn setup_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db");
        setup_memory_schema(&conn);
        conn
    }

    #[test]
    fn no_merge_decision_suppresses_unchanged_cluster_until_review_epoch() -> Result<()> {
        let conn = setup_conn();
        let project = "test-project";
        let cluster = cluster(100);

        record_no_merge(&conn, project, &cluster, Some("different decisions"))?;
        let plan = load_cluster_plan(&conn, project, vec![cluster])?;

        assert_eq!(plan.eligible.len(), 0);
        assert_eq!(plan.suppressed, 1);
        Ok(())
    }

    #[test]
    fn changed_cluster_signature_is_eligible_again() -> Result<()> {
        let conn = setup_conn();
        let project = "test-project";

        record_no_merge(&conn, project, &cluster(100), Some("different decisions"))?;
        let plan = load_cluster_plan(&conn, project, vec![cluster(200)])?;

        assert_eq!(plan.eligible.len(), 1);
        assert_eq!(plan.suppressed, 0);
        Ok(())
    }

    #[test]
    fn expired_no_merge_decision_is_eligible_again() -> Result<()> {
        let conn = setup_conn();
        let project = "test-project";
        let cluster = cluster(100);
        record_no_merge(&conn, project, &cluster, Some("different decisions"))?;
        conn.execute(
            "UPDATE dream_cluster_decisions SET next_review_epoch = 1",
            [],
        )?;

        let plan = load_cluster_plan(&conn, project, vec![cluster])?;

        assert_eq!(plan.eligible.len(), 1);
        assert_eq!(plan.suppressed, 0);
        Ok(())
    }
}
