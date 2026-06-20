mod normalize;
mod render;
mod sources;
#[cfg(test)]
mod tests;
mod types;

use anyhow::Result;
use rusqlite::Connection;

use types::RecallState;
pub use types::{
    UserRecallCandidateCounts, UserRecallDiagnostics, UserRecallDroppedItem, UserRecallItem,
    UserRecallRequest, UserRecallResult,
};

pub fn recall_user_context(conn: &Connection, req: &UserRecallRequest) -> Result<UserRecallResult> {
    let normalized = normalize::normalize_request(req)?;
    let mut state = RecallState::default();

    sources::collect_summary(conn, &normalized, &mut state)?;
    sources::collect_claims(conn, &normalized, &mut state)?;
    sources::collect_current_state(conn, &normalized, &mut state)?;
    sources::collect_memories(conn, &normalized, &mut state)?;
    sources::collect_workstreams(conn, &normalized, &mut state)?;
    sources::collect_recent_sessions(conn, &normalized, &mut state)?;

    render::finalize(normalized, state)
}
