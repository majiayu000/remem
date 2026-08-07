//! End-to-end SessionStart compile: request -> plan -> candidates ->
//! bundle (GH-932).
//!
//! This is the first path that reads real data. Everything below the
//! request is deterministic given the database contents: the plan is a
//! pure function of the request, and the executor's selection is a pure
//! function of the plan plus the candidate set.

use anyhow::Result;
use rusqlite::Connection;

use crate::context::load_session_start_candidates;
use crate::retrieval_router::{plan, RetrievalPlan};

use super::domain::{ContextBundle, ContextIntent, ContextRequest};
use super::executor::{blocked_before_load, execute, ExecutorInputs};

/// Compile a SessionStart bundle from the database.
///
/// Returns `Err` only when the plan itself cannot be compiled (an invalid
/// request). A canonical load failure is *not* an error return: it
/// produces a `Blocked` bundle so the failure travels with the audit
/// instead of being swallowed or mistaken for an empty project.
pub fn compile_session_start_bundle(
    conn: &Connection,
    request: &ContextRequest,
    cwd: &str,
    current_branch: Option<&str>,
    enrichment_available: bool,
) -> Result<ContextBundle> {
    let compiled = plan(request, Some(ContextIntent::SessionStart))?;
    Ok(bundle_for_plan(
        conn,
        &compiled,
        &request.project.key,
        cwd,
        current_branch,
        enrichment_available,
    ))
}

#[allow(clippy::too_many_arguments)]
fn bundle_for_plan(
    conn: &Connection,
    compiled: &RetrievalPlan,
    project: &str,
    cwd: &str,
    current_branch: Option<&str>,
    enrichment_available: bool,
) -> ContextBundle {
    match load_session_start_candidates(conn, project, cwd, current_branch) {
        Ok(candidates) => execute(
            compiled,
            &ExecutorInputs {
                candidates,
                enrichment_available,
            },
        ),
        Err(error) => blocked_before_load(compiled, &error.to_string()),
    }
}
