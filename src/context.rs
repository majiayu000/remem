mod abstention;
mod audit;
mod bundle_candidates;
pub mod claude_memory;
mod commit_signals;
mod debug;
mod diagnostics;
mod fact_labels;
mod filters;
mod format;
mod hook_warning;
mod host;
mod hybrid_context;
mod implicit_query;
mod injection_gate;
pub(crate) use injection_gate::context_fingerprint as context_output_fingerprint;
mod invocation;
mod memory_selection;
mod memory_traits;
mod ownership;
mod poisoning;
mod policy;
mod prompt_submit;
mod query;
mod relevance;
mod render;
mod render_bundle;
mod render_error;
mod render_inputs;
mod sections;
mod style;
mod summary_query;

use std::ffi::OsString;

#[cfg(test)]
mod tests;
mod types;

#[cfg(test)]
pub(crate) use bundle_candidates::load_session_start_candidates;
pub(crate) use bundle_candidates::{
    load_session_start_candidates_with_limits, LoadedBundleCandidates,
};
pub(crate) use hybrid_context::{
    query_hybrid_context_memories_with_rank_signal_mode, InjectionRankSignalMode,
};
pub(crate) use policy::ContextLimits;
pub(crate) use prompt_submit::prompt_submit_additional_context;
pub(crate) use relevance::{
    build_sessionstart_relevance_plan, RelevanceCandidate, RelevanceSection,
    SessionStartRelevancePlan, SESSIONSTART_RELEVANCE_POLICY_VERSION,
};
pub(crate) use render::governance_eval_snapshot;
pub(crate) use render::session_start_benchmark_emission;
pub(crate) use render::session_start_eval_snapshot;
pub(crate) use render::RENDER_CONTRACT_VERSION;
pub use render::{
    generate_context, generate_context_from_cli, generate_cursor_context_from_bytes,
    generate_cursor_context_from_stdin,
};
pub(crate) use render_bundle::{
    context_bundle_render_mode, ContextBundleRenderMode, CONTEXT_BUNDLE_RENDER_MODE_ENV,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContextOutputGateContractSnapshot {
    pub(crate) injection_key: String,
    pub(crate) output_mode: String,
    pub(crate) emit_count: i64,
    pub(crate) suppress_count: i64,
    pub(crate) first_output_present: bool,
    pub(crate) second_output_present: bool,
}

pub(crate) fn output_gate_contract_snapshot(
    conn: &rusqlite::Connection,
    project: &str,
    session_id: &str,
    host_arg: &str,
    output: &str,
) -> anyhow::Result<ContextOutputGateContractSnapshot> {
    let invocation = invocation::ContextInvocation {
        cwd: project.to_string(),
        project: project.to_string(),
        session_id: Some(session_id.to_string()),
        transcript_path: None,
        source: Some("compact".to_string()),
        host: host::resolve_host_kind(Some(host_arg)),
        use_colors: false,
        debug: false,
        force: false,
        gate_mode: Some("auto".to_string()),
    };
    let _gated_hosts_restore =
        EnvRestore::set("REMEM_CONTEXT_GATE_HOSTS", invocation.host.as_env_value());
    let first = injection_gate::apply_context_gate_with_data_version(
        conn,
        &invocation,
        output.to_string(),
        Some("eval-output-gate"),
    );
    let second = injection_gate::apply_context_gate_with_data_version(
        conn,
        &invocation,
        output.to_string(),
        Some("eval-output-gate"),
    );
    let injection_key = first
        .key
        .clone()
        .or_else(|| second.key.clone())
        .ok_or_else(|| anyhow::anyhow!("output gate did not return an injection key"))?;
    let (output_mode, emit_count, suppress_count) = conn.query_row(
        "SELECT output_mode, emit_count, suppress_count
         FROM context_injections
         WHERE host = ?1
           AND injection_key = ?2",
        rusqlite::params![invocation.host.as_env_value(), &injection_key],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        },
    )?;

    Ok(ContextOutputGateContractSnapshot {
        injection_key,
        output_mode,
        emit_count,
        suppress_count,
        first_output_present: !first.output.is_empty(),
        second_output_present: !second.output.is_empty(),
    })
}

struct EnvRestore {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvRestore {
    fn set(key: &'static str, value: impl Into<OsString>) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value.into());
        Self { key, previous }
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.as_ref() {
            std::env::set_var(self.key, previous);
        } else {
            std::env::remove_var(self.key);
        }
    }
}
