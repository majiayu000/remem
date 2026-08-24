mod local_copy;
mod save;
#[cfg(test)]
mod save_poisoning_tests;
#[cfg(test)]
mod save_preference_rule_tests;
#[cfg(test)]
mod save_tests;
mod search;
#[cfg(test)]
mod tests;
mod types;

pub use crate::memory::current_state::{current_state, CurrentStateRequest, CurrentStateResult};
pub use local_copy::{resolve_local_note_path, sanitize_segment};
pub(crate) use save::save_memory_for_benchmark_fixture;
pub use save::{
    save_memory, save_memory_from_with_reference_time, save_memory_with_reference_time,
    LocalCopyError, SaveMemoryCaller, SaveMemoryIdempotencyConflictError,
    SaveMemoryValidationError,
};
pub use search::search_memories;
pub(crate) use search::{
    search_memories_with_explain_details, search_memories_with_explain_details_with_routing,
};
pub use types::{
    default_include_stale, default_include_suppressed, MultiHopMeta, SaveMemoryRequest,
    SaveMemoryResult, SearchRequest, SearchResultSet, SearchRoutingPolicy, SearchRoutingWeights,
};
