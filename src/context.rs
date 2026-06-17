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

#[cfg(test)]
mod tests;
mod types;

pub(crate) use prompt_submit::prompt_submit_additional_context;
pub(crate) use render::governance_eval_snapshot;
pub(crate) use render::session_start_eval_snapshot;
pub use render::{generate_context, generate_context_from_cli};
