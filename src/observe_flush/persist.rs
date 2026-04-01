use anyhow::Result;

use crate::db;
use crate::memory_format::ParsedObservation;

pub(crate) fn persist_flush_batch(
    conn: &mut rusqlite::Connection,
    session_id: &str,
    project: &str,
    lease_owner: &str,
    batch: &[db::PendingObservation],
    observations: &[ParsedObservation],
    usage: i64,
    branch: Option<&str>,
    commit_sha: Option<&str>,
) -> Result<()> {
    let ids: Vec<i64> = batch.iter().map(|pending| pending.id).collect();
    let per_obs_usage = usage / observations.len().max(1) as i64;

    let tx = conn.transaction()?;
    let memory_session_id = db::upsert_session(&tx, session_id, project, None)?;

    for obs in observations {
        let duplicate_id = if let Some(narrative) = &obs.narrative {
            crate::dedup::check_duplicate(&tx, project, narrative, None)?
        } else {
            None
        };
        if let Some(dup_id) = duplicate_id {
            crate::log::info(
                "flush",
                &format!("skip duplicate observation (matches id={})", dup_id),
            );
            continue;
        }

        let facts_json = (!obs.facts.is_empty())
            .then(|| serde_json::to_string(&obs.facts))
            .transpose()?;
        let concepts_json = (!obs.concepts.is_empty())
            .then(|| serde_json::to_string(&obs.concepts))
            .transpose()?;
        let files_read_json = (!obs.files_read.is_empty())
            .then(|| serde_json::to_string(&obs.files_read))
            .transpose()?;
        let files_modified_json = (!obs.files_modified.is_empty())
            .then(|| serde_json::to_string(&obs.files_modified))
            .transpose()?;

        let obs_id = db::insert_observation_with_branch(
            &tx,
            &memory_session_id,
            project,
            &obs.obs_type,
            obs.title.as_deref(),
            obs.subtitle.as_deref(),
            obs.narrative.as_deref(),
            facts_json.as_deref(),
            concepts_json.as_deref(),
            files_read_json.as_deref(),
            files_modified_json.as_deref(),
            None,
            per_obs_usage,
            branch,
            commit_sha,
        )?;

        if !obs.files_modified.is_empty() {
            let stale_count = db::mark_stale_by_files(&tx, obs_id, project, &obs.files_modified)?;
            if stale_count > 0 {
                crate::log::info(
                    "flush",
                    &format!("marked {} stale (file overlap)", stale_count),
                );
            }
        }
    }

    let deleted = db::delete_pending_claimed(&tx, lease_owner, &ids)?;
    if deleted != ids.len() {
        anyhow::bail!(
            "pending ack mismatch: expected {}, deleted {}",
            ids.len(),
            deleted
        );
    }

    tx.commit()?;
    Ok(())
}
