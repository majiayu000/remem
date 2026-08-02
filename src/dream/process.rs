use std::future::Future;
use std::pin::Pin;

use anyhow::{anyhow, Result};
use rusqlite::{Connection, TransactionBehavior};

use super::merge::MergeDecision;
use super::{apply, conflict, decisions, poisoning, Cluster};

type MergeFuture<'a> = Pin<Box<dyn Future<Output = Result<MergeDecision>> + 'a>>;

fn generated_text_metadata(field: &str, value: &str) -> String {
    format!(
        "{field}_bytes={} {field}_sha256={}",
        value.len(),
        crate::db::content_identity_hash(value.as_bytes())
    )
}

fn error_metadata(error_code: &str, error: &anyhow::Error) -> String {
    let payload = error.to_string();
    format!(
        "error_code={error_code} error_bytes={} error_sha256={}",
        payload.len(),
        crate::db::content_identity_hash(payload.as_bytes())
    )
}

fn conflict_error_metadata(conflicting_ids: &[i64], error: &anyhow::Error) -> String {
    let mut metadata = error_metadata("conflict_record_failed", error);
    if conflicting_ids.len() < 2 {
        metadata.push_str(" failure_class=\"dream conflict requires at least two memory ids\"");
    }
    metadata
}

fn apply_and_record_merged(
    conn: &mut Connection,
    project: &str,
    cluster: &Cluster,
    result: &super::merge::MergeResult,
) -> Result<apply::ApplyOutcome> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let outcome = apply::apply_in_transaction(&tx, project, cluster, result)?;
    decisions::record_merged(&tx, project, cluster, outcome)?;
    tx.commit()?;
    Ok(outcome)
}

fn record_no_merge(
    conn: &mut Connection,
    project: &str,
    cluster: &Cluster,
    reason: Option<&str>,
) -> Result<()> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    super::freshness::validate_cluster_snapshot(&tx, project, cluster)?;
    decisions::record_no_merge(&tx, project, cluster, reason)?;
    tx.commit()?;
    Ok(())
}

pub(super) async fn process_clusters(
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
                let diagnostic = error_metadata("merge_failed", &error);
                decisions::record_failed(conn, project, cluster, &diagnostic)?;
                merge_failures += 1;
                crate::log::warn(
                    "dream",
                    &format!(
                        "project={} cluster_size={} cluster_first_id={:?} {}",
                        project, cluster_size, cluster_first_id, diagnostic
                    ),
                );
                continue;
            }
        };

        match poisoning::quarantine_if_needed(conn, project, cluster, &decision) {
            Ok(true) => {
                skipped += 1;
                continue;
            }
            Ok(false) => {}
            Err(error) => {
                let diagnostic = error_metadata("poison_quarantine_failed", &error);
                decisions::record_failed(conn, project, cluster, &diagnostic)?;
                apply_failures += 1;
                crate::log::error(
                    "dream",
                    &format!(
                        "project={} cluster_size={} cluster_first_id={:?} {}",
                        project, cluster_size, cluster_first_id, diagnostic
                    ),
                );
                continue;
            }
        }

        match decision {
            MergeDecision::Merge(result) => {
                let topic_metadata = generated_text_metadata("topic_key", &result.topic_key);
                let superseded = result.superseded_ids.len();
                let outcome = match apply_and_record_merged(conn, project, cluster, &result) {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        let diagnostic = error_metadata("apply_or_decision_failed", &error);
                        decisions::record_failed(conn, project, cluster, &diagnostic)?;
                        apply_failures += 1;
                        crate::log::warn(
                            "dream",
                            &format!(
                                "project={} cluster_size={} cluster_first_id={:?} {} {}",
                                project, cluster_size, cluster_first_id, topic_metadata, diagnostic
                            ),
                        );
                        continue;
                    }
                };
                merged += 1;
                crate::log::info(
                    "dream",
                    &format!(
                        "merged merged_id={} operation_id={} superseded={} {}",
                        outcome.merged_id, outcome.operation_id, superseded, topic_metadata
                    ),
                );
            }
            MergeDecision::NoMerge { reason } => {
                match record_no_merge(conn, project, cluster, reason.as_deref()) {
                    Ok(()) => skipped += 1,
                    Err(error) => {
                        let diagnostic = error_metadata("no_merge_record_failed", &error);
                        decisions::record_failed(conn, project, cluster, &diagnostic)?;
                        apply_failures += 1;
                        crate::log::warn(
                            "dream",
                            &format!(
                                "project={} cluster_size={} cluster_first_id={:?} {}",
                                project, cluster_size, cluster_first_id, diagnostic
                            ),
                        );
                        continue;
                    }
                }
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
                    let diagnostic = conflict_error_metadata(&conflicting_ids, &error);
                    decisions::record_failed(conn, project, cluster, &diagnostic)?;
                    apply_failures += 1;
                    crate::log::warn(
                        "dream",
                        &format!(
                            "project={} cluster_size={} cluster_first_id={:?} {}",
                            project, cluster_size, cluster_first_id, diagnostic
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

    #[test]
    fn generated_payload_metadata_never_contains_raw_text() {
        let sentinel = "RAW_MODEL_TOPIC_SENTINEL";
        let metadata = generated_text_metadata("topic_key", sentinel);
        assert!(!metadata.contains(sentinel));
        assert!(metadata.contains("topic_key_bytes=24"));
        assert!(metadata.contains("topic_key_sha256=sha256:content-v1:"));
    }

    #[test]
    fn model_error_metadata_never_contains_raw_error() {
        let sentinel = "RAW_MODEL_ERROR_SENTINEL";
        let metadata = error_metadata("merge_failed", &anyhow!(sentinel));
        assert!(!metadata.contains(sentinel));
        assert!(metadata.contains("error_code=merge_failed"));
        assert!(metadata.contains("error_bytes=24"));
        assert!(metadata.contains("error_sha256=sha256:content-v1:"));
    }
}
