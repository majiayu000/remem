pub(crate) const OBSERVATION_PROMPT: &str = include_str!("../../prompts/observation.txt");
pub(crate) const TASK_OBSERVATION_PROMPT: &str = include_str!("../../prompts/task_observation.txt");

/// Max events per flush batch (prevents oversized AI input)
pub(crate) const FLUSH_BATCH_SIZE: usize = 15;
/// On AI timeout, split large batches recursively to improve success rate.
pub(crate) const FLUSH_RETRY_MIN_BATCH_SIZE: usize = 1;
/// Pending lease duration for a single flush worker.
pub(crate) const PENDING_LEASE_SECS: i64 = 240;
pub(crate) const PENDING_RETRY_MAX_SECS: i64 = 1800;

/// Min Task response length worth processing (skip empty/error results).
pub(crate) const MIN_TASK_RESPONSE_LEN: usize = 100;
