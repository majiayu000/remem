mod apply;
mod candidates;
mod constants;
mod merge;

use anyhow::{anyhow, Result};
use candidates::load_clusters;
pub(crate) use candidates::Cluster;
use merge::{merge_cluster, MergeDecision};
use rusqlite::Connection;
use std::future::Future;
use std::pin::Pin;

pub(crate) fn list_clusters(project: &str) -> Result<Vec<Cluster>> {
    let conn = crate::db::open_db()?;
    load_clusters(&conn, project)
}

pub async fn process_dream_job(project: &str) -> Result<()> {
    let mut conn = crate::db::open_db()?;
    let clusters = load_clusters(&conn, project)?;
    process_clusters(project, &mut conn, &clusters, |cluster, project| {
        Box::pin(merge_cluster(cluster, project))
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

    for cluster in clusters {
        let cluster_size = cluster.members.len();
        let cluster_first_id = cluster.members.first().map(|member| member.id);

        let decision = match merge_fn(cluster, project).await {
            Ok(decision) => decision,
            Err(error) => {
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
                if let Err(error) = apply::apply(conn, project, &result) {
                    return Err(anyhow!(
                        "project={} cluster_size={} cluster_first_id={:?} topic_key={} apply failed: {}",
                        project,
                        cluster_size,
                        cluster_first_id,
                        topic_key,
                        error
                    ));
                }
                merged += 1;
                crate::log::info(
                    "dream",
                    &format!("merged topic_key={} superseded={}", topic_key, superseded),
                );
            }
            MergeDecision::NoMerge => {
                skipped += 1;
            }
        }
    }

    crate::log::info(
        "dream",
        &format!(
            "project={} merged={} skipped={} merge_failures={}",
            project, merged, skipped, merge_failures
        ),
    );

    if merged == 0 && merge_failures > 0 {
        return Err(anyhow!(
            "project={} all {} cluster merge attempts failed",
            project,
            merge_failures
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
        .expect_err("dream processing should fail when every cluster merge fails");

        assert!(
            error
                .to_string()
                .contains("all 2 cluster merge attempts failed"),
            "error should report all-clusters-failed"
        );
    }

    #[tokio::test]
    async fn process_clusters_propagates_apply_failure() {
        let mut conn = Connection::open_in_memory().expect("in-memory db");
        setup_memory_schema(&conn);
        let project = "test-dream-apply-failure";
        let clusters = vec![make_cluster([101, 102], ["topic-a", "topic-b"])];

        let error = process_clusters(project, &mut conn, &clusters, |_cluster, _project| {
            Box::pin(async move {
                Ok(MergeDecision::Merge(merge::MergeResult {
                    topic_key: "merged-topic".to_owned(),
                    memory_type: "decision".to_owned(),
                    title: "Merged title".to_owned(),
                    content: "Merged content".to_owned(),
                    superseded_ids: vec![99999],
                }))
            })
        })
        .await
        .expect_err("dream processing should fail when apply fails");

        assert!(
            error.to_string().contains("apply failed"),
            "error should include the apply failure context"
        );

        let merged_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE project = ?1 AND topic_key = ?2",
                params![project, "merged-topic"],
                |row| row.get(0),
            )
            .expect("count merged rows");
        assert_eq!(merged_count, 0, "failed apply must roll back merged memory");
    }
}
