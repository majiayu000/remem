use crate::db;
use crate::memory_format::ParsedObservation;

pub(crate) fn clone_pending_batch(batch: &[&db::PendingObservation]) -> Vec<db::PendingObservation> {
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

pub(crate) fn split_timeout_range(start: usize, end: usize, min_batch_size: usize) -> Option<[(usize, usize); 2]> {
    let batch_len = end.checked_sub(start)?;
    if batch_len <= min_batch_size {
        return None;
    }

    let mid = start + (batch_len / 2);
    if mid > start && mid < end {
        Some([(start, mid), (mid, end)])
    } else {
        None
    }
}

pub(crate) fn collect_observation_titles(observations: &[ParsedObservation]) -> Vec<String> {
    observations
        .iter()
        .filter_map(|observation| observation.title.as_deref().map(str::to_string))
        .collect()
}
