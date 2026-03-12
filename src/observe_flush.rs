use anyhow::Result;

use crate::db;
use crate::memory_format::{
    self, xml_escape_attr, xml_escape_text, ParsedObservation, OBSERVATION_TYPES,
};

const OBSERVATION_PROMPT: &str = include_str!("../prompts/observation.txt");
const TASK_OBSERVATION_PROMPT: &str = include_str!("../prompts/task_observation.txt");

/// Max events per flush batch (prevents oversized AI input)
const FLUSH_BATCH_SIZE: usize = 15;
/// On AI timeout, split large batches recursively to improve success rate.
const FLUSH_RETRY_MIN_BATCH_SIZE: usize = 1;
/// Pending lease duration for a single flush worker.
const PENDING_LEASE_SECS: i64 = 240;

/// Min Task response length worth processing (skip empty/error results).
const MIN_TASK_RESPONSE_LEN: usize = 100;

fn build_existing_context(conn: &rusqlite::Connection, project: &str) -> Result<String> {
    let recent = db::query_observations(conn, project, OBSERVATION_TYPES, 10)?;
    if recent.is_empty() {
        return Ok(String::new());
    }

    let mut buf = String::from("<existing_memories>\n");
    for obs in &recent {
        buf.push_str(&format!(
            "<memory type=\"{}\">{}{}</memory>\n",
            xml_escape_attr(&obs.r#type),
            obs.title
                .as_deref()
                .map(|t| format!(" title=\"{}\"", xml_escape_attr(t)))
                .unwrap_or_default(),
            obs.subtitle
                .as_deref()
                .map(|s| format!(" — {}", xml_escape_text(s)))
                .unwrap_or_default(),
        ));
    }
    buf.push_str("</existing_memories>\n");
    Ok(buf)
}

fn build_session_events_xml(batch: &[db::PendingObservation]) -> String {
    let mut events = String::new();
    for (i, p) in batch.iter().enumerate() {
        events.push_str(&format!(
            "<event index=\"{}\">\n\
             <tool>{}</tool>\n\
             <working_directory>{}</working_directory>\n\
             <parameters>{}</parameters>\n\
             <outcome>{}</outcome>\n\
             </event>\n",
            i + 1,
            xml_escape_text(&p.tool_name),
            xml_escape_text(p.cwd.as_deref().unwrap_or(".")),
            xml_escape_text(p.tool_input.as_deref().unwrap_or("")),
            xml_escape_text(p.tool_response.as_deref().unwrap_or("")),
        ));
    }
    events
}

fn is_ai_timeout_error(err: &anyhow::Error) -> bool {
    err.to_string().to_lowercase().contains("timed out")
}

fn persist_flush_batch(
    conn: &mut rusqlite::Connection,
    session_id: &str,
    project: &str,
    lease_owner: &str,
    batch: &[db::PendingObservation],
    observations: &[ParsedObservation],
    usage: i64,
) -> Result<()> {
    let ids: Vec<i64> = batch.iter().map(|p| p.id).collect();
    let per_obs_usage = usage / observations.len().max(1) as i64;

    let tx = conn.transaction()?;
    let memory_session_id = db::upsert_session(&tx, session_id, project, None)?;

    for obs in observations {
        let facts_json = if obs.facts.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&obs.facts)?)
        };
        let concepts_json = if obs.concepts.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&obs.concepts)?)
        };
        let files_read_json = if obs.files_read.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&obs.files_read)?)
        };
        let files_modified_json = if obs.files_modified.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&obs.files_modified)?)
        };

        let obs_id = db::insert_observation(
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

/// Flush a single Task pending observation with the task-specific prompt.
/// Returns number of observations persisted (0 if skipped/failed).
async fn flush_single_task(
    conn: &mut rusqlite::Connection,
    session_id: &str,
    project: &str,
    lease_owner: &str,
    pending: &db::PendingObservation,
) -> Result<usize> {
    let response_text = pending.tool_response.as_deref().unwrap_or("");
    if response_text.len() < MIN_TASK_RESPONSE_LEN {
        crate::log::info(
            "flush-task",
            &format!(
                "skip Task id={} (response {}B < {}B)",
                pending.id,
                response_text.len(),
                MIN_TASK_RESPONSE_LEN
            ),
        );
        db::delete_pending_claimed(conn, lease_owner, &[pending.id])?;
        return Ok(0);
    }

    let existing_context = build_existing_context(conn, project).unwrap_or_default();
    let events = build_session_events_xml(std::slice::from_ref(pending));
    let user_message = format!(
        "{}<session_events>\n{}</session_events>",
        existing_context, events
    );

    let ai_start = std::time::Instant::now();
    let response = crate::ai::call_ai(
        TASK_OBSERVATION_PROMPT,
        &user_message,
        crate::ai::UsageContext {
            project: Some(project),
            operation: "flush-task",
        },
    )
    .await?;
    let ai_ms = ai_start.elapsed().as_millis();

    crate::log::info(
        "flush-task",
        &format!("AI response {}ms {}B", ai_ms, response.len()),
    );

    let observations = memory_format::parse_observations(&response);
    if observations.is_empty() {
        crate::log::info("flush-task", "no observations extracted from Task result");
        db::delete_pending_claimed(conn, lease_owner, &[pending.id])?;
        return Ok(0);
    }

    let usage = response.len() as i64 / 4;
    persist_flush_batch(
        conn,
        session_id,
        project,
        lease_owner,
        std::slice::from_ref(pending),
        &observations,
        usage,
    )?;

    Ok(observations.len())
}

/// Flush pending queue: batch action events, process Task events individually.
pub async fn flush_pending(session_id: &str, project: &str) -> Result<usize> {
    let mut conn = db::open_db()?;
    let lease_owner = format!(
        "flush-{}-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_millis(),
        crate::db::truncate_str(session_id, 8)
    );
    let pending = db::claim_pending(
        &conn,
        session_id,
        FLUSH_BATCH_SIZE,
        &lease_owner,
        PENDING_LEASE_SECS,
    )?;

    if pending.is_empty() {
        crate::log::info("flush", "no pending observations");
        return Ok(0);
    }

    let timer = crate::log::Timer::start(
        "flush",
        &format!("{} events project={}", pending.len(), project),
    );

    // Split into Task vs Action batches
    let (task_pending, action_pending): (Vec<_>, Vec<_>) = pending
        .iter()
        .enumerate()
        .partition::<Vec<_>, _>(|(_, p)| p.tool_name == "Task");
    let task_indices: Vec<usize> = task_pending.into_iter().map(|(i, _)| i).collect();
    let action_indices: Vec<usize> = action_pending.into_iter().map(|(i, _)| i).collect();

    let mut total_observations = 0usize;
    let mut titles: Vec<String> = Vec::new();

    // --- Flush Task events: each one independently ---
    for &idx in &task_indices {
        let p = &pending[idx];
        match flush_single_task(&mut conn, session_id, project, &lease_owner, p).await {
            Ok(n) => {
                total_observations += n;
                if n > 0 {
                    crate::log::info(
                        "flush-task",
                        &format!("Task id={} → {} observations", p.id, n),
                    );
                }
            }
            Err(e) => {
                crate::log::warn(
                    "flush-task",
                    &format!("Task id={} flush failed (continuing): {}", p.id, e),
                );
                // Delete the pending on failure to avoid infinite retry
                if let Err(del_err) = db::delete_pending_claimed(&conn, &lease_owner, &[p.id]) {
                    crate::log::warn(
                        "flush-task",
                        &format!("delete failed id={}: {}", p.id, del_err),
                    );
                }
            }
        }
    }

    // --- Flush Action events: batch processing with split-retry ---
    if !action_indices.is_empty() {
        let action_batch: Vec<&db::PendingObservation> =
            action_indices.iter().map(|&i| &pending[i]).collect();

        let mut ranges: Vec<(usize, usize)> = vec![(0, action_batch.len())];
        let mut _total_usage = 0i64;
        let mut split_retries = 0usize;

        while let Some((start, end)) = ranges.pop() {
            let batch: Vec<&db::PendingObservation> = action_batch[start..end].to_vec();
            if batch.is_empty() {
                continue;
            }

            let existing_context = match build_existing_context(&conn, project) {
                Ok(ctx) => ctx,
                Err(e) => {
                    crate::log::warn(
                        "flush",
                        &format!("existing context failed (continuing): {}", e),
                    );
                    String::new()
                }
            };

            // Build events XML from borrowed batch
            let batch_owned: Vec<db::PendingObservation> = batch
                .iter()
                .map(|p| db::PendingObservation {
                    id: p.id,
                    session_id: p.session_id.clone(),
                    project: p.project.clone(),
                    tool_name: p.tool_name.clone(),
                    tool_input: p.tool_input.clone(),
                    tool_response: p.tool_response.clone(),
                    cwd: p.cwd.clone(),
                    created_at_epoch: p.created_at_epoch,
                })
                .collect();
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
                Ok(r) => r,
                Err(e) => {
                    let can_split =
                        is_ai_timeout_error(&e) && batch.len() > FLUSH_RETRY_MIN_BATCH_SIZE;
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

                    if let Err(release_err) = db::release_pending_claims(&conn, &lease_owner) {
                        crate::log::warn(
                            "flush",
                            &format!("release claim failed: {}", release_err),
                        );
                    }
                    crate::log::warn("flush", &format!("AI call failed: {}", e));
                    timer.done(&format!("AI error: {}", e));
                    return Err(e);
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
                let ids: Vec<i64> = batch.iter().map(|p| p.id).collect();
                db::delete_pending_claimed(&conn, &lease_owner, &ids)?;
                continue;
            }

            let usage = response.len() as i64 / 4;
            persist_flush_batch(
                &mut conn,
                session_id,
                project,
                &lease_owner,
                &batch_owned,
                &observations,
                usage,
            )?;

            _total_usage += usage;
            total_observations += observations.len();
            titles.extend(
                observations
                    .iter()
                    .filter_map(|o| o.title.as_deref().map(str::to_string)),
            );
        }

        if split_retries > 0 {
            crate::log::info(
                "flush",
                &format!("action batch split_retries={}", split_retries),
            );
        }
    }

    if total_observations == 0 {
        timer.done("0 observations");
        return Ok(0);
    }

    timer.done(&format!(
        "{} events → {} observations [{}]",
        pending.len(),
        total_observations,
        titles.join(", "),
    ));

    Ok(total_observations)
}
