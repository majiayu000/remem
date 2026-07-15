mod claim;
mod enqueue;
mod state;
#[cfg(test)]
mod tests;

pub use crate::db::models::{Job, JobType};

pub use claim::claim_next_job;
pub use enqueue::{enqueue_job, maybe_enqueue_dream_job, DreamEnqueueDecision};
pub(crate) use enqueue::{enqueue_job_in_transaction, maybe_enqueue_dream_job_in_transaction};
pub use state::{
    mark_job_done, mark_job_exhausted, mark_job_failed, mark_job_failed_or_retry,
    release_expired_job_leases, requeue_stuck_jobs, ExpiredJobLeaseBatch, ExpiredJobLeaseOutcome,
    JobIdentityKind, JobTransitionOutcome,
};

pub(crate) fn dream_profile_key(payload_json: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(payload_json)
        .ok()
        .and_then(|value| {
            value
                .get(crate::runtime_config::MEMORY_AI_PROFILE_FIELD)
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|profile| !profile.is_empty())
                .map(str::to_string)
        })
}
