use std::time::Instant;

use super::policy::ContextPolicy;
use super::query::load_context_data_with_policy;
use super::types::{ContextLoadError, ContextRequest, LoadedContext};

pub(in crate::context) struct ContextRenderInputs {
    pub(in crate::context) loaded: LoadedContext,
    pub(in crate::context) preference_output: String,
    pub(in crate::context) preference_details: crate::memory::preference::PreferenceRenderDetails,
    pub(in crate::context) load_timing: crate::perf::PhaseTiming,
    pub(in crate::context) preference_timing: crate::perf::PhaseTiming,
}

pub(in crate::context) fn load_context_render_inputs(
    conn: &rusqlite::Connection,
    request: &ContextRequest,
    debug: bool,
    policy: &ContextPolicy,
) -> ContextRenderInputs {
    let load_start = Instant::now();
    let mut loaded = load_context_data_with_policy(
        conn,
        &request.project,
        request.current_branch.as_deref(),
        policy,
        debug,
    );
    let load_timing = crate::perf::PhaseTiming::elapsed("load_context_data", load_start);

    let preference_start = Instant::now();
    let (preference_output, preference_details) =
        match render_preferences_to_buffer(conn, &request.project, &request.cwd, policy) {
            Ok(rendered) => rendered,
            Err(error) => {
                let message = format!(
                    "failed to render preferences for {}: {error}",
                    request.project
                );
                crate::log::error("context", &message);
                loaded
                    .errors
                    .push(ContextLoadError::new("preferences", message));
                (
                    String::new(),
                    crate::memory::preference::PreferenceRenderDetails::default(),
                )
            }
        };
    let preference_timing =
        crate::perf::PhaseTiming::elapsed("render_preferences", preference_start);

    ContextRenderInputs {
        loaded,
        preference_output,
        preference_details,
        load_timing,
        preference_timing,
    }
}

pub(in crate::context) fn render_preferences_to_buffer(
    conn: &rusqlite::Connection,
    project: &str,
    cwd: &str,
    policy: &ContextPolicy,
) -> anyhow::Result<(String, crate::memory::preference::PreferenceRenderDetails)> {
    let mut output = String::new();
    let limits = &policy.limits;
    let details = crate::memory::preference::render_preferences_with_context_details(
        &mut output,
        conn,
        project,
        cwd,
        limits.preference_project_limit,
        limits.preference_global_limit,
        limits.preference_char_limit,
    )?;
    Ok((output, details))
}
