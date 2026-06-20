mod candidate_review;
mod candidates;
mod capabilities;
mod detail;
mod graph;
mod health;
mod list;
mod save;
mod search;
mod show;
mod stats;
mod status;
mod user_recall;

pub(super) use candidate_review::{
    handle_approve_candidate, handle_edit_candidate, handle_reject_candidate,
};
pub(super) use candidates::handle_list_candidates;
pub(super) use capabilities::handle_capabilities;
pub(super) use detail::handle_memory_detail;
pub(super) use graph::handle_graph;
pub(super) use health::handle_health;
pub(super) use list::handle_list_memories;
pub(super) use save::handle_save_memory;
pub(super) use search::handle_search;
#[cfg(test)]
pub(super) use search::search_request_from_params;
pub(super) use show::handle_get_memory;
pub(super) use stats::handle_stats;
pub(super) use status::handle_status;
pub(super) use user_recall::handle_user_recall;
