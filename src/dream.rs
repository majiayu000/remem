mod apply;
mod candidates;
mod conflict;
mod constants;
mod decisions;
mod merge;

use anyhow::{anyhow, Result};
use candidates::load_clusters;
pub(crate) use candidates::Cluster;
pub(crate) use constants::DREAM_COOLDOWN_SECS;
use decisions::{load_cluster_plan, record_failed, record_merged, record_no_merge};
use merge::{merge_cluster, MergeDecision};
use rusqlite::Connection;
use std::future::Future;
use std::pin::Pin;

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

type MergeFuture<'a> = Pin<Box<dyn Future<Output = Result<MergeDecision>> + 'a>>;

async fn process_clusters(
    project: &str,
    conn: &mut Connection,
    clusters: &[Cluster],
    merge_fn: impl for<'a> Fn(&'a Cluster, &'a str) -> MergeFuture<'a>,
) -> Result<()> {
    if clusters.is_empty() {
        crate::log::info(
            "dream",
            &format!("project={} no clusters to merge", project),
        );
        return Ok(());
    }

    crate::log::info(
        "dream",
        &format!("project={} clusters={}", project, clusters.len()),
    );

    let mut merged = 0usize;
    let mut skipped = 0usize;
    let mut merge_failures = 0usize;
    let mut apply_failures = 0usize;

    for cluster in clusters {
        let cluster_size = cluster.members.len();
        let cluster_first_id = cluster.members.first().map(|member| member.id);

        let decision = match merge_fn(cluster, project).await {
            Ok(decision) => decision,
            Err(error) => {
                record_failed(
                    conn,
                    project,
                    cluster,
                    &format!(
                        "merge failed: {}",
                        crate::db::truncate_str(&error.to_string(), 500)
                    ),
                )?;
                merge_failures += 1;
                crate::log::warn(
                    "dream",
                    &format!(
                        "project={} cluster_size={} cluster_first_id={:?} merge failed: {}",
                        project, cluster_size, cluster_first_id, error
                    ),
                );
                continue;
            }
        };

        match decision {
            MergeDecision::Merge(result) => {
                let topic_key = result.topic_key.clone();
                let superseded = result.superseded_ids.len();
                let outcome = match apply::apply(conn, project, &result) {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        record_failed(
                            conn,
                            project,
                            cluster,
                            &format!(
                                "apply failed: {}",
                                crate::db::truncate_str(&error.to_string(), 500)
                            ),
                        )?;
                        apply_failures += 1;
                        crate::log::warn(
                            "dream",
                            &format!(
                                "project={} cluster_size={} cluster_first_id={:?} topic_key={} apply failed: {}",
                                project, cluster_size, cluster_first_id, topic_key, error
                            ),
                        );
                        continue;
                    }
                };
                if let Err(error) = record_merged(conn, project, cluster, outcome) {
                    apply_failures += 1;
                    crate::log::warn(
                        "dream",
                        &format!(
                            "project={} cluster_size={} cluster_first_id={:?} topic_key={} decision record failed: {}",
                            project, cluster_size, cluster_first_id, topic_key, error
                        ),
                    );
                    continue;
                }
                merged += 1;
                crate::log::info(
                    "dream",
                    &format!("merged topic_key={} superseded={}", topic_key, superseded),
                );
            }
            MergeDecision::NoMerge { reason } => {
                record_no_merge(conn, project, cluster, reason.as_deref())?;
                skipped += 1;
            }
            MergeDecision::Conflict {
                conflicting_ids,
                reason,
            } => match conflict::record_conflict(
                conn,
                project,
                cluster,
                &conflicting_ids,
                reason.as_deref(),
            ) {
                Ok(outcome) => {
                    skipped += 1;
                    crate::log::info(
                        "dream",
                        &format!(
                            "deferred conflict ids={:?} operation_id={} edge_count={}",
                            conflicting_ids, outcome.operation_id, outcome.edge_count
                        ),
                    );
                }
                Err(error) => {
                    record_failed(
                        conn,
                        project,
                        cluster,
                        &format!(
                            "conflict record failed: {}",
                            crate::db::truncate_str(&error.to_string(), 500)
                        ),
                    )?;
                    apply_failures += 1;
                    crate::log::warn(
                        "dream",
                        &format!(
                            "project={} cluster_size={} cluster_first_id={:?} conflict record failed: {}",
                            project, cluster_size, cluster_first_id, error
                        ),
                    );
                    continue;
                }
            },
        }
    }

    crate::log::info(
        "dream",
        &format!(
            "project={} merged={} skipped={} merge_failures={} apply_failures={}",
            project, merged, skipped, merge_failures, apply_failures
        ),
    );

    let total_failures = merge_failures + apply_failures;
    if merged == 0 && skipped == 0 && total_failures > 0 {
        return Err(anyhow!(
            "project={} all {} cluster attempts failed (merge_failures={} apply_failures={})",
            project,
            total_failures,
            merge_failures,
            apply_failures
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::insert_memory;
    use crate::memory::tests_helper::setup_memory_schema;
    use anyhow::anyhow;
    use rusqlite::params;

    fn make_cluster(ids: [i64; 2], topic_keys: [&str; 2]) -> Cluster {
        Cluster {
            members: vec![
                candidates::MemoryCandidate {
                    id: ids[0],
                    topic_key: Some(topic_keys[0].to_owned()),
                    title: format!("title-{}", ids[0]),
                    content: format!("content-{}", ids[0]),
                    memory_type: "decision".to_owned(),
                    updated_at_epoch: 1,
                },
                candidates::MemoryCandidate {
                    id: ids[1],
                    topic_key: Some(topic_keys[1].to_owned()),
                    title: format!("title-{}", ids[1]),
                    content: format!("content-{}", ids[1]),
                    memory_type: "decision".to_owned(),
                    updated_at_epoch: 2,
                },
            ],
        }
    }

    #[tokio::test]
    async fn process_clusters_continues_after_cluster_failure() {
        let mut conn = Connection::open_in_memory().expect("in-memory db");
        setup_memory_schema(&conn);
        let project = "test-dream-process";

        let stale_id = insert_memory(
            &conn,
            Some("sess-1"),
            project,
            None,
            "old title",
            "old content",
            "decision",
            None,
        )
        .expect("insert");

        let failing_cluster = make_cluster([101, 102], ["broken-topic-a", "broken-topic-b"]);
        let success_cluster = make_cluster([201, 202], ["good-topic-a", "good-topic-b"]);
        let clusters = vec![failing_cluster, success_cluster];

        process_clusters(project, &mut conn, &clusters, |cluster, _project| {
            let should_fail = cluster.members[0].id == 101;
            Box::pin(async move {
                if should_fail {
                    return Err(anyhow!("synthetic merge failure"));
                }
                Ok(MergeDecision::Merge(merge::MergeResult {
                    topic_key: "merged-topic".to_owned(),
                    memory_type: "decision".to_owned(),
                    title: "Merged title".to_owned(),
                    content: "Merged content".to_owned(),
                    superseded_ids: vec![stale_id],
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

        let stale_status: String = conn
            .query_row(
                "SELECT status FROM memories WHERE id = ?1",
                params![stale_id],
                |row| row.get(0),
            )
            .expect("read stale status");
        assert_eq!(stale_status, "stale");
    }

    #[tokio::test]
    async fn process_clusters_fails_when_all_cluster_merges_fail() {
        let mut conn = Connection::open_in_memory().expect("in-memory db");
        setup_memory_schema(&conn);
        let project = "test-dream-all-fail";
        let clusters = vec![
            make_cluster([101, 102], ["broken-topic-a", "broken-topic-b"]),
            make_cluster([201, 202], ["broken-topic-c", "broken-topic-d"]),
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
        let clusters = vec![make_cluster([101, 102], ["topic-a", "topic-b"])];

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
        let clusters = vec![Cluster {
            members: vec![
                candidates::MemoryCandidate {
                    id: first_id,
                    topic_key: Some("conflict-a".to_string()),
                    title: "Use provider A".to_string(),
                    content: "Use provider A for embeddings.".to_string(),
                    memory_type: "decision".to_string(),
                    updated_at_epoch: 1,
                },
                candidates::MemoryCandidate {
                    id: second_id,
                    topic_key: Some("conflict-b".to_string()),
                    title: "Use provider B".to_string(),
                    content: "Use provider B for embeddings.".to_string(),
                    memory_type: "decision".to_string(),
                    updated_at_epoch: 2,
                },
            ],
        }];

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
        assert_eq!(conflict_edge_count, 1);
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
    async fn process_clusters_continues_after_apply_failure() {
        let mut conn = Connection::open_in_memory().expect("in-memory db");
        setup_memory_schema(&conn);
        let project = "test-dream-apply-failure";
        let stale_id = insert_memory(
            &conn,
            Some("sess-1"),
            project,
            None,
            "old title",
            "old content",
            "decision",
            None,
        )
        .expect("insert");
        let clusters = vec![
            make_cluster([101, 102], ["bad-topic-a", "bad-topic-b"]),
            make_cluster([201, 202], ["good-topic-a", "good-topic-b"]),
        ];

        process_clusters(project, &mut conn, &clusters, |cluster, _project| {
            let should_fail_apply = cluster.members[0].id == 101;
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
                        vec![stale_id]
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

        let stale_status: String = conn
            .query_row(
                "SELECT status FROM memories WHERE id = ?1",
                params![stale_id],
                |row| row.get(0),
            )
            .expect("read stale status");
        assert_eq!(stale_status, "stale");
    }
}
