mod abstention;
mod audit;
pub mod claude_memory;
mod commit_signals;
mod debug;
mod diagnostics;
mod fact_labels;
mod filters;
mod format;
mod host;
mod hybrid_context;
mod implicit_query;
mod injection_gate;
mod invocation;
mod memory_selection;
mod memory_traits;
mod ownership;
mod policy;
mod prompt_submit;
mod query;
mod render;
mod render_inputs;
mod sections;
mod style;

use std::ffi::OsString;

#[cfg(test)]
mod tests;
mod types;

pub(crate) use prompt_submit::prompt_submit_additional_context;
pub(crate) use render::governance_eval_snapshot;
pub(crate) use render::session_start_eval_snapshot;
pub use render::{generate_context, generate_context_from_cli};

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
