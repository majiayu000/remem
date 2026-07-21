mod candidate_detail;
mod candidate_review;
mod candidate_safe_review;
mod candidates;
mod capabilities;
mod detail;
mod events;
mod graph;
mod health;
mod list;
mod memory_governance;
mod observations;
mod save;
mod search;
mod sessions;
mod show;
mod stats;
mod status;
mod tasks;
mod user_recall;
mod workstreams;

pub(super) use candidate_detail::handle_candidate_detail;
pub(super) use candidate_review::{
    handle_approve_candidate, handle_edit_candidate, handle_reject_candidate,
};
#[cfg(test)]
pub(super) use candidate_safe_review::execute_safe_review_for_test;
pub(super) use candidate_safe_review::{
    handle_safe_approve_candidate, handle_safe_edit_candidate, handle_safe_reject_candidate,
};
pub(super) use candidates::{handle_blocked_candidates, handle_list_candidates};
pub(super) use capabilities::handle_capabilities;
pub(super) use detail::handle_memory_detail;
pub(super) use events::{handle_event_detail, handle_list_events};
pub(super) use graph::handle_graph;
pub(super) use health::handle_health;
pub(super) use list::handle_list_memories;
#[cfg(test)]
pub(super) use memory_governance::execute_memory_governance_for_test;
pub(super) use memory_governance::{handle_archive_memory, handle_restore_memory};
pub(super) use observations::{handle_list_observations, handle_observation_detail};
pub(super) use save::handle_save_memory;
pub(super) use search::handle_search;
#[cfg(test)]
pub(super) use search::search_request_from_params;
pub(super) use sessions::{handle_list_sessions, handle_session_detail};
pub(super) use show::handle_get_memory;
pub(super) use stats::handle_stats;
pub(super) use status::handle_status;
pub(super) use tasks::{handle_list_tasks, handle_task_detail};
pub(super) use user_recall::handle_user_recall;
pub(super) use workstreams::{handle_list_workstreams, handle_workstream_detail};
