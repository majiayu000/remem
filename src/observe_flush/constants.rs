pub(crate) const OBSERVATION_PROMPT: &str = include_str!("../../prompts/observation.txt");
pub(crate) const TASK_OBSERVATION_PROMPT: &str = include_str!("../../prompts/task_observation.txt");

/// Max events per flush batch (prevents oversized AI input)
pub(crate) const FLUSH_BATCH_SIZE: usize = 15;
/// Max flush batches processed by one observation job before scheduling follow-up work.
pub(crate) const FLUSH_DRAIN_MAX_BATCHES: usize = 4;
/// Max wall-clock seconds spent draining pending observations in one observation job.
pub(crate) const FLUSH_DRAIN_MAX_SECS: u64 = 240;
/// Follow-up observation jobs must not starve summary jobs.
pub(crate) const OBSERVATION_FOLLOW_UP_PRIORITY: i64 = 150;
/// On AI timeout, split large batches recursively to improve success rate.
pub(crate) const FLUSH_RETRY_MIN_BATCH_SIZE: usize = 1;
/// Pending lease duration for a single flush worker.
pub(crate) const PENDING_LEASE_SECS: i64 = 240;
pub(crate) const PENDING_RETRY_MAX_SECS: i64 = 1800;

/// Min Task response length worth processing (skip empty/error results).
pub(crate) const MIN_TASK_RESPONSE_LEN: usize = 100;
