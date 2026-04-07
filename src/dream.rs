mod apply;
mod candidates;
mod constants;
mod merge;

use anyhow::Result;
use candidates::load_clusters;
pub(crate) use candidates::Cluster;
use merge::{MergeDecision, merge_cluster};

pub(crate) fn list_clusters(project: &str) -> Result<Vec<Cluster>> {
    let conn = crate::db::open_db()?;
    load_clusters(&conn, project)
}

pub async fn process_dream_job(project: &str) -> Result<()> {
    let conn = crate::db::open_db()?;
    let clusters = load_clusters(&conn, project)?;

    if clusters.is_empty() {
        crate::log::info("dream", &format!("project={} no clusters to merge", project));
        return Ok(());
    }

    crate::log::info(
        "dream",
        &format!("project={} clusters={}", project, clusters.len()),
    );

    let mut merged = 0usize;
    let mut skipped = 0usize;

    for cluster in &clusters {
        match merge_cluster(cluster, project).await? {
            MergeDecision::Merge(result) => {
                apply::apply(&conn, project, &result)?;
                merged += 1;
                crate::log::info(
                    "dream",
                    &format!(
                        "merged topic_key={} superseded={}",
                        result.topic_key,
                        result.superseded_ids.len()
                    ),
                );
            }
            MergeDecision::NoMerge => {
                skipped += 1;
            }
        }
    }

    crate::log::info(
        "dream",
        &format!("project={} merged={} skipped={}", project, merged, skipped),
    );
    Ok(())
}
