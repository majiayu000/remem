use anyhow::Result;

use crate::db;
use crate::memory_format;

use super::constants::{FLUSH_RETRY_MIN_BATCH_SIZE, OBSERVATION_PROMPT};
use super::context::{build_existing_context, build_session_events_xml};
use super::persist::persist_flush_batch;
use super::runtime::{is_ai_timeout_error, pending_retry_backoff_secs};

pub(crate) struct ActionFlushOutcome {
    pub total_observations: usize,
    pub titles: Vec<String>,
    pub split_retries: usize,
}

fn clone_pending_batch(batch: &[&db::PendingObservation]) -> Vec<db::PendingObservation> {
    batch
        .iter()
        .map(|pending| db::PendingObservation {
            id: pending.id,
            session_id: pending.session_id.clone(),
            project: pending.project.clone(),
            tool_name: pending.tool_name.clone(),
            tool_input: pending.tool_input.clone(),
            tool_response: pending.tool_response.clone(),
            cwd: pending.cwd.clone(),
            created_at_epoch: pending.created_at_epoch,
            updated_at_epoch: pending.updated_at_epoch,
            status: pending.status.clone(),
            attempt_count: pending.attempt_count,
            next_retry_epoch: pending.next_retry_epoch,
            last_error: pending.last_error.clone(),
        })
        .collect()
}

pub(crate) async fn flush_action_batches(
    conn: &mut rusqlite::Connection,
    session_id: &str,
    project: &str,
    lease_owner: &str,
    pending: &[db::PendingObservation],
    action_indices: &[usize],
) -> Result<ActionFlushOutcome> {
    let action_batch: Vec<&db::PendingObservation> =
        action_indices.iter().map(|&i| &pending[i]).collect();
    let mut ranges: Vec<(usize, usize)> = vec![(0, action_batch.len())];
    let mut total_observations = 0usize;
    let mut titles = Vec::new();
    let mut split_retries = 0usize;
    let mut _total_usage = 0i64;

    while let Some((start, end)) = ranges.pop() {
        let batch: Vec<&db::PendingObservation> = action_batch[start..end].to_vec();
        if batch.is_empty() {
            continue;
        }

        let existing_context = match build_existing_context(conn, project) {
            Ok(context) => context,
            Err(err) => {
                crate::log::warn(
                    "flush",
                    &format!("existing context failed (continuing): {}", err),
                );
                String::new()
            }
        };

        let batch_owned = clone_pending_batch(&batch);
        let events = build_session_events_xml(&batch_owned);
        let user_message = format!(
            "{}<session_events>\n{}</session_events>",
            existing_context, events
        );

        let ai_start = std::time::Instant::now();
        let response = match crate::ai::call_ai(
            OBSERVATION_PROMPT,
            &user_message,
            crate::ai::UsageContext {
                project: Some(project),
                operation: "flush",
            },
        )
        .await
        {
            Ok(response) => response,
            Err(err) => {
                let can_split =
                    is_ai_timeout_error(&err) && batch.len() > FLUSH_RETRY_MIN_BATCH_SIZE;
                if can_split {
                    let mid = start + (batch.len() / 2);
                    if mid > start && mid < end {
                        split_retries += 1;
                        crate::log::warn(
                            "flush",
                            &format!(
                                "AI timeout on {} events, splitting into {} + {}",
                                batch.len(),
                                mid - start,
                                end - mid
                            ),
                        );
                        ranges.push((mid, end));
                        ranges.push((start, mid));
                        continue;
                    }
                }

                let ids: Vec<i64> = batch.iter().map(|pending| pending.id).collect();
                let max_attempt = batch
                    .iter()
                    .map(|pending| pending.attempt_count)
                    .max()
                    .unwrap_or(1);
                let backoff = pending_retry_backoff_secs(max_attempt);
                let err_msg = format!("action flush ai call failed: {}", err);
                if let Err(retry_err) =
                    db::retry_pending_claimed(conn, lease_owner, &ids, &err_msg, backoff)
                {
                    crate::log::warn("flush", &format!("retry mark failed: {}", retry_err));
                }
                crate::log::warn("flush", &format!("AI call failed: {}", err));
                return Err(err);
            }
        };
        let ai_ms = ai_start.elapsed().as_millis();
        crate::log::info(
            "flush",
            &format!(
                "AI response {}ms {}B (batch {} events)",
                ai_ms,
                response.len(),
                batch.len()
            ),
        );

        let observations = memory_format::parse_observations(&response);
        if observations.is_empty() {
            crate::log::info(
                "flush",
                &format!(
                    "no observations extracted from batch ({} events)",
                    batch.len()
                ),
            );
            let ids: Vec<i64> = batch.iter().map(|pending| pending.id).collect();
            db::fail_pending_claimed(
                conn,
                lease_owner,
                &ids,
                "no observations extracted from action batch",
            )?;
            continue;
        }

        let usage = response.len() as i64 / 4;
        let batch_cwd = batch.first().and_then(|pending| pending.cwd.as_deref());
        let batch_branch = batch_cwd.and_then(db::detect_git_branch);
        let batch_commit = batch_cwd.and_then(db::detect_git_commit);
        persist_flush_batch(
            conn,
            session_id,
            project,
            lease_owner,
            &batch_owned,
            &observations,
            usage,
            batch_branch.as_deref(),
            batch_commit.as_deref(),
        )?;

        _total_usage += usage;
        total_observations += observations.len();
        titles.extend(
            observations
                .iter()
                .filter_map(|obs| obs.title.as_deref().map(str::to_string)),
        );
    }

    Ok(ActionFlushOutcome {
        total_observations,
        titles,
        split_retries,
    })
}
