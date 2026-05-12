//! Extraction queue (`extraction_tasks`). The queue is coalesced by
//! `idempotency_key`, has explicit leases/retries, and tracks progress by
//! event range (`cursor_event_id` / `high_watermark_event_id`).

pub mod claim;
pub mod enqueue;
pub mod query;
pub mod types;
pub mod worker;

pub use claim::{
    claim_next_ready_task, claim_ready_tasks, mark_task_delayed, mark_task_done, mark_task_failed,
    recover_expired_leases, ClaimedTask, DEFAULT_LEASE_SECS,
};
pub use enqueue::{enqueue_extraction_task, EnqueueRequest};
pub use query::{count_ready_for_identity, oldest_ready_epoch};
pub use types::{TaskKind, TaskStatus};
