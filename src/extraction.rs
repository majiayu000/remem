//! v2 extraction queue (`extraction_tasks`). Replaces the v1 split between
//! `pending_observations` and `jobs` with a single coalesced queue keyed by
//! `idempotency_key`. Lifts the lease / retry / scheduler shape from the
//! 5/5 SPEC implementation but uses the v2.1 §1 M4 progress invariant:
//! progress is event-range-based (`cursor_event_id` / `high_watermark_event_id`),
//! not observation-count-based.

pub mod claim;
pub mod enqueue;
pub mod query;
pub mod types;

pub use claim::{
    claim_ready_tasks, mark_task_delayed, mark_task_done, mark_task_failed,
    recover_expired_leases, ClaimedTask, DEFAULT_LEASE_SECS,
};
pub use enqueue::{enqueue_extraction_task, EnqueueRequest};
pub use query::{count_ready_for_identity, oldest_ready_epoch};
pub use types::{TaskKind, TaskStatus};
