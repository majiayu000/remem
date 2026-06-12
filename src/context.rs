pub mod claude_memory;
mod debug;
mod diagnostics;
mod filters;
mod format;
mod host;
mod injection_gate;
mod invocation;
mod memory_traits;
mod ownership;
mod policy;
mod query;
mod render;
mod sections;
mod style;

#[cfg(test)]
mod tests;
mod types;

pub(crate) use render::governance_eval_snapshot;
pub(crate) use render::session_start_eval_snapshot;
pub use render::{generate_context, generate_context_from_cli};
