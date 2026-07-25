mod config;
pub(crate) mod cursor_config;
pub(crate) mod duplicates;
mod host;
mod hosts;
mod json_io;
mod paths;
mod runtime;
#[cfg(test)]
mod tests;

pub use host::InstallTarget;
pub(crate) use paths::{claude_mcp_paths, codex_memories_dir, is_codex_rollout_summary_filename};
pub use runtime::{install, uninstall};
