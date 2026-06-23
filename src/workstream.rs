mod identity;
mod lifecycle;
mod matcher;
mod merge;
mod query;
#[cfg(test)]
mod tests;
mod types;
mod write;

pub use lifecycle::{
    auto_abandon_all_inactive, auto_abandon_all_inactive_at, auto_abandon_inactive,
    auto_pause_all_inactive, auto_pause_all_inactive_at, auto_pause_inactive,
    count_auto_abandon_all_inactive_at, count_auto_pause_all_inactive_at,
    DEFAULT_AUTO_ABANDON_DAYS, DEFAULT_AUTO_PAUSE_DAYS,
};
pub use matcher::find_matching_workstream;
pub use merge::merge_workstreams_manual;
pub use query::{query_active_workstreams, query_workstreams};
pub use types::{
    ParsedWorkStream, WorkStream, WorkStreamMergeResult, WorkStreamStatus, WorkStreamUpsertResult,
};
pub use write::{update_workstream_manual, upsert_workstream, upsert_workstream_with_match};
