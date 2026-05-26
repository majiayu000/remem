pub mod claude_memory;
mod format;
mod host;
mod injection_gate;
mod invocation;
mod memory_traits;
mod policy;
mod query;
mod render;
mod sections;

#[cfg(test)]
mod tests;
mod types;

pub use render::{generate_context, generate_context_from_cli};
