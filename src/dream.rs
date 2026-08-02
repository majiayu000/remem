mod apply;
mod candidates;
mod conflict;
mod constants;
mod decisions;
#[cfg(test)]
mod exposure_tests;
mod freshness;
#[cfg(test)]
mod freshness_tests;
mod merge;
mod poisoning;
mod process;
mod provenance;
#[cfg(test)]
mod regression_tests;

use anyhow::Result;
use candidates::load_clusters;
pub(crate) use candidates::Cluster;
pub(crate) use constants::DREAM_COOLDOWN_SECS;
use decisions::load_cluster_plan;
use merge::merge_cluster;
use process::process_clusters;
pub(crate) use provenance::{
    cluster_signature_sha256, decision_payload_sha256, quarantine_semantic_discriminator_sha256,
    DreamClusterMemberSnapshot, DreamDecisionPayload,
};

#[derive(Debug)]
pub(crate) struct DreamClusterPlan {
    pub eligible: Vec<Cluster>,
    pub suppressed: usize,
}

pub(crate) fn list_cluster_plan(project: &str) -> Result<DreamClusterPlan> {
    let conn = crate::db::open_db()?;
    let clusters = load_clusters(&conn, project)?;
    let plan = decisions::load_cluster_plan(&conn, project, clusters)?;
    Ok(DreamClusterPlan {
        eligible: plan.eligible,
        suppressed: plan.suppressed,
    })
}

pub async fn process_dream_job(project: &str) -> Result<()> {
    let host = crate::runtime_config::default_host()?;
    process_dream_job_with_host(project, &host).await
}

pub async fn process_dream_job_with_host(project: &str, host: &str) -> Result<()> {
    process_dream_job_with_selection(project, Some(host), None).await
}

pub async fn process_dream_job_with_profile(project: &str, profile: Option<&str>) -> Result<()> {
    process_dream_job_with_selection(project, None, profile).await
}

async fn process_dream_job_with_selection(
    project: &str,
    host: Option<&str>,
    profile: Option<&str>,
) -> Result<()> {
    let host = host.map(str::to_string);
    let profile = profile.map(str::to_string);
    let mut conn = crate::db::open_db()?;
    let clusters = load_clusters(&conn, project)?;
    let plan = load_cluster_plan(&conn, project, clusters)?;
    if plan.suppressed > 0 {
        crate::log::info(
            "dream",
            &format!(
                "project={} suppressed={} cluster(s) by durable dream decisions",
                project, plan.suppressed
            ),
        );
    }
    process_clusters(project, &mut conn, &plan.eligible, |cluster, project| {
        Box::pin(merge_cluster(
            cluster,
            project,
            host.clone(),
            profile.clone(),
        ))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::merge::MergeDecision;
    use super::*;
    use crate::memory::insert_memory;
    use crate::memory::tests_helper::setup_memory_schema;
    use anyhow::anyhow;
    use rusqlite::{params, Connection};

    fn snapshot_cluster(conn: &Connection, ids: &[i64]) -> Cluster {
        let members = ids
            .iter()
            .map(|id| {
                conn.query_row(
                    "SELECT id, version, topic_key, title, content, memory_type, updated_at_epoch
                     FROM memories WHERE id = ?1",
                    params![id],
                    |row| {
                        Ok(candidates::MemoryCandidate {
                            id: row.get(0)?,
                            version: row.get(1)?,
                            topic_key: row.get(2)?,
                            title: row.get(3)?,
                            content: row.get(4)?,
                            memory_type: row.get(5)?,
                            updated_at_epoch: row.get(6)?,
                        })
                    },
                )
                .expect("cluster member snapshot")
            })
            .collect();
        Cluster { members }
    }

    fn make_cluster(
        conn: &Connection,
        project: &str,
        ids: [i64; 2],
        topic_keys: [&str; 2],
    ) -> Cluster {
        for (offset, (id, topic_key)) in ids.into_iter().zip(topic_keys).enumerate() {
            let epoch = offset as i64 + 1;
            conn.execute(
                "INSERT INTO memories
                 (id, session_id, project, topic_key, title, content, memory_type,
                  created_at_epoch, updated_at_epoch, status, scope, source_project,
                  target_project, owner_scope, owner_key, context_class)
                 VALUES (?1, 'dream-test', ?2, ?3, ?4, ?5, 'decision', ?6, ?6,
                         'active', 'project', ?2, ?2, 'repo', ?2, 'startup_core')",
                params![
                    id,
                    project,
                    topic_key,
                    format!("title-{id}"),
                    format!("content-{id}"),
                    epoch
                ],
            )
            .expect("insert cluster member");
        }
        snapshot_cluster(conn, &ids)
    }

    #[tokio::test]
    async fn process_clusters_continues_after_cluster_failure() {
        let mut conn = Connection::open_in_memory().expect("in-memory db");
        setup_memory_schema(&conn);
        let project = "test-dream-process";

        let failing_cluster = make_cluster(
            &conn,
            project,
            [101, 102],
            ["broken-topic-a", "broken-topic-b"],
        );
        let success_cluster =
            make_cluster(&conn, project, [201, 202], ["good-topic-a", "good-topic-b"]);
        let clusters = vec![failing_cluster, success_cluster];

        process_clusters(project, &mut conn, &clusters, |cluster, _project| {
            let should_fail = cluster.members[0].id == 101;
            let superseded_ids = cluster.members.iter().map(|member| member.id).collect();
            Box::pin(async move {
                if should_fail {
                    return Err(anyhow!("synthetic merge failure"));
                }
                Ok(MergeDecision::Merge(merge::MergeResult {
                    topic_key: "merged-topic".to_owned(),
                    memory_type: "decision".to_owned(),
                    title: "Merged title".to_owned(),
                    content: "Merged content".to_owned(),
                    superseded_ids,
                }))
            })
        })
        .await
        .expect("dream processing should continue after a cluster failure");

        let merged_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE project = ?1 AND topic_key = ?2",
                params![project, "merged-topic"],
                |row| row.get(0),
            )
            .expect("count merged rows");
        assert_eq!(merged_count, 1, "later clusters should still be applied");

        let stale_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE id IN (201, 202) AND status = 'stale'",
                [],
                |row| row.get(0),
            )
            .expect("read stale count");
        assert_eq!(stale_count, 2);
    }

    #[tokio::test]
    async fn process_clusters_fails_when_all_cluster_merges_fail() {
        let mut conn = Connection::open_in_memory().expect("in-memory db");
        setup_memory_schema(&conn);
        let project = "test-dream-all-fail";
        let clusters = vec![
            make_cluster(
                &conn,
                project,
                [101, 102],
                ["broken-topic-a", "broken-topic-b"],
            ),
            make_cluster(
                &conn,
                project,
                [201, 202],
                ["broken-topic-c", "broken-topic-d"],
            ),
        ];

        let error = process_clusters(project, &mut conn, &clusters, |_cluster, _project| {
            Box::pin(async move { Err(anyhow!("synthetic merge failure")) })
        })
        .await
        .expect_err("dream processing should fail when every cluster attempt fails");

        assert!(
            error.to_string().contains("all 2 cluster attempts failed"),
            "error should report total failure: {error}"
        );
    }

    #[tokio::test]
    async fn process_clusters_persists_no_merge_decision() {
        let mut conn = Connection::open_in_memory().expect("in-memory db");
        setup_memory_schema(&conn);
        let project = "test-dream-no-merge";
        let clusters = vec![make_cluster(
            &conn,
            project,
            [101, 102],
            ["topic-a", "topic-b"],
        )];

        process_clusters(project, &mut conn, &clusters, |_cluster, _project| {
            Box::pin(async move {
                Ok(MergeDecision::NoMerge {
                    reason: Some("entries cover different decisions".to_string()),
                })
            })
        })
        .await
        .expect("no-merge should persist and complete");

        let row: (String, String, String) = conn
            .query_row(
                "SELECT decision, reason, member_ids_json
                 FROM dream_cluster_decisions
                 WHERE project = ?1",
                params![project],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("decision row should load");
        assert_eq!(row.0, "no_merge");
        assert_eq!(row.1, "entries cover different decisions");
        assert_eq!(row.2, "[101,102]");
    }

    #[tokio::test]
    async fn process_clusters_persists_conflict_defer_without_merging() -> Result<()> {
        let mut conn = Connection::open_in_memory()?;
        setup_memory_schema(&conn);
        let project = "test-dream-conflict";
        let first_id = insert_memory(
            &conn,
            Some("sess-1"),
            project,
            Some("conflict-a"),
            "Use provider A",
            "Use provider A for embeddings.",
            "decision",
            None,
        )?;
        let second_id = insert_memory(
            &conn,
            Some("sess-1"),
            project,
            Some("conflict-b"),
            "Use provider B",
            "Use provider B for embeddings.",
            "decision",
            None,
        )?;
        let clusters = vec![snapshot_cluster(&conn, &[first_id, second_id])];

        process_clusters(project, &mut conn, &clusters, |_cluster, _project| {
            Box::pin(async move {
                Ok(MergeDecision::Conflict {
                    conflicting_ids: vec![second_id, first_id],
                    reason: Some("embedding provider is unresolved".to_string()),
                })
            })
        })
        .await?;

        let active_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memories
             WHERE id IN (?1, ?2) AND status = 'active'",
            params![first_id, second_id],
            |row| row.get(0),
        )?;
        assert_eq!(active_count, 2);
        let memory_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE project = ?1",
            params![project],
            |row| row.get(0),
        )?;
        assert_eq!(memory_count, 2);

        let (operation, conflicting_json, defer_reason): (String, String, Option<String>) = conn
            .query_row(
                "SELECT operation, conflicting_ids, defer_reason
                 FROM memory_operation_log
                 ORDER BY id DESC
                 LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
        assert_eq!(operation, "conflict");
        let conflicting_ids: Vec<i64> = serde_json::from_str(&conflicting_json)?;
        assert_eq!(conflicting_ids, vec![first_id, second_id]);
        assert_eq!(
            defer_reason.as_deref(),
            Some("embedding provider is unresolved")
        );

        let conflict_edge_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_edges WHERE edge_type = 'conflicts'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(conflict_edge_count, 2);
        let merged_edge_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_edges WHERE edge_type = 'merged_into'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(merged_edge_count, 0);

        let (decision, reason, operation_id): (String, String, Option<i64>) = conn.query_row(
            "SELECT decision, reason, source_operation_id
             FROM dream_cluster_decisions
             WHERE project = ?1",
            params![project],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_eq!(decision, "defer");
        assert_eq!(reason, "embedding provider is unresolved");
        assert!(operation_id.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn process_clusters_defer_records_only_conflicting_subset() -> Result<()> {
        let mut conn = Connection::open_in_memory()?;
        setup_memory_schema(&conn);
        let project = "test-dream-conflict-subset";
        let first_id = insert_memory(
            &conn,
            Some("sess-1"),
            project,
            Some("conflict-a"),
            "Use provider A",
            "Use provider A for embeddings.",
            "decision",
            None,
        )?;
        let second_id = insert_memory(
            &conn,
            Some("sess-1"),
            project,
            Some("conflict-b"),
            "Keep provider B as backup",
            "Keep provider B as a backup option.",
            "decision",
            None,
        )?;
        let third_id = insert_memory(
            &conn,
            Some("sess-1"),
            project,
            Some("conflict-c"),
            "Use provider C",
            "Use provider C for embeddings.",
            "decision",
            None,
        )?;
        let clusters = vec![snapshot_cluster(&conn, &[first_id, second_id, third_id])];

        process_clusters(project, &mut conn, &clusters, |_cluster, _project| {
            Box::pin(async move {
                Ok(MergeDecision::Conflict {
                    conflicting_ids: vec![third_id, first_id],
                    reason: Some("provider choice is unresolved".to_string()),
                })
            })
        })
        .await?;

        let (member_ids_json, cluster_size): (String, i64) = conn.query_row(
            "SELECT member_ids_json, cluster_size
             FROM dream_cluster_decisions
             WHERE project = ?1",
            params![project],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let member_ids: Vec<i64> = serde_json::from_str(&member_ids_json)?;
        assert_eq!(member_ids, vec![first_id, third_id]);
        assert_eq!(cluster_size, 2);

        let conflict_edge_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_edges WHERE edge_type = 'conflicts'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(conflict_edge_count, 2);

        let operation_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_operation_log WHERE operation = 'conflict'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(operation_count, 1);

        let plan = decisions::load_cluster_plan(
            &conn,
            project,
            vec![Cluster {
                members: clusters[0].members.clone(),
            }],
        )?;
        assert_eq!(plan.eligible.len(), 1);
        assert_eq!(plan.suppressed, 0);

        process_clusters(project, &mut conn, &clusters, |_cluster, _project| {
            Box::pin(async move {
                Ok(MergeDecision::Conflict {
                    conflicting_ids: vec![third_id, first_id],
                    reason: Some("provider choice is unresolved".to_string()),
                })
            })
        })
        .await?;

        let conflict_edge_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_edges WHERE edge_type = 'conflicts'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(conflict_edge_count, 2);
        let operation_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_operation_log WHERE operation = 'conflict'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(operation_count, 1);
        Ok(())
    }

    #[tokio::test]
    async fn process_clusters_records_failed_for_invalid_conflict_ids() -> Result<()> {
        let mut conn = Connection::open_in_memory()?;
        setup_memory_schema(&conn);
        let project = "test-dream-invalid-conflict";
        let clusters = vec![make_cluster(
            &conn,
            project,
            [101, 102],
            ["topic-a", "topic-b"],
        )];

        let Err(error) = process_clusters(project, &mut conn, &clusters, |_cluster, _project| {
            Box::pin(async move {
                Ok(MergeDecision::Conflict {
                    conflicting_ids: vec![101],
                    reason: Some("invalid conflict ids".to_string()),
                })
            })
        })
        .await
        else {
            panic!("invalid conflict should fail the only cluster attempt");
        };
        assert!(
            error.to_string().contains("all 1 cluster attempts failed"),
            "unexpected error: {error}"
        );
        let (decision, reason): (String, String) = conn.query_row(
            "SELECT decision, reason
             FROM dream_cluster_decisions
             WHERE project = ?1",
            params![project],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(decision, "failed");
        assert!(
            reason.contains("dream conflict requires at least two memory ids"),
            "unexpected failed reason: {reason}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn process_clusters_continues_after_apply_failure() {
        let mut conn = Connection::open_in_memory().expect("in-memory db");
        setup_memory_schema(&conn);
        let project = "test-dream-apply-failure";
        let clusters = vec![
            make_cluster(&conn, project, [101, 102], ["bad-topic-a", "bad-topic-b"]),
            make_cluster(&conn, project, [201, 202], ["good-topic-a", "good-topic-b"]),
        ];

        process_clusters(project, &mut conn, &clusters, |cluster, _project| {
            let should_fail_apply = cluster.members[0].id == 101;
            let superseded_ids = cluster.members.iter().map(|member| member.id).collect();
            Box::pin(async move {
                Ok(MergeDecision::Merge(merge::MergeResult {
                    topic_key: if should_fail_apply {
                        "failed-apply-topic".to_owned()
                    } else {
                        "merged-topic".to_owned()
                    },
                    memory_type: "decision".to_owned(),
                    title: "Merged title".to_owned(),
                    content: "Merged content".to_owned(),
                    superseded_ids: if should_fail_apply {
                        vec![99999]
                    } else {
                        superseded_ids
                    },
                }))
            })
        })
        .await
        .expect("dream processing should continue after an apply failure");

        let merged_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE project = ?1 AND topic_key = ?2",
                params![project, "merged-topic"],
                |row| row.get(0),
            )
            .expect("count merged rows");
        assert_eq!(
            merged_count, 1,
            "later clusters should still merge after apply failure"
        );

        let failed_apply_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE project = ?1 AND topic_key = ?2",
                params![project, "failed-apply-topic"],
                |row| row.get(0),
            )
            .expect("count failed apply rows");
        assert_eq!(
            failed_apply_count, 0,
            "failed apply must still roll back its own transaction"
        );

        let stale_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE id IN (201, 202) AND status = 'stale'",
                [],
                |row| row.get(0),
            )
            .expect("read stale count");
        assert_eq!(stale_count, 2);
    }
}
