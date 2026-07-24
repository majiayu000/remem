mod claude;
mod codex;
pub(in crate::install) mod cursor;

pub(in crate::install) use claude::ClaudeHost;
pub(in crate::install) use codex::CodexHost;

use crate::install::host::{InstallHost, InstallTarget};

/// Resolve the concrete list of trait-driven hosts (Claude/Codex) to act on
/// for a given target. The Cursor host is deliberately not part of this
/// list: its two files plus runtime receipt are coordinated through
/// `hosts::cursor` (GH-824 B-009/B-010) instead of the two-phase
/// `install_mcp`/`install_hooks` trait flow.
///
/// - `Claude` / `Codex`: single explicit host (always acted upon, even if
///   config missing — the user asked for it).
/// - `Cursor`: no trait-driven hosts.
/// - `Auto`: only hosts whose config dir exists.
/// - `All`: every known trait-driven host.
pub(in crate::install) fn resolve_hosts(target: InstallTarget) -> Vec<Box<dyn InstallHost>> {
    match target {
        InstallTarget::Claude => vec![Box::new(ClaudeHost)],
        InstallTarget::Codex => vec![Box::new(CodexHost)],
        InstallTarget::Cursor => Vec::new(),
        InstallTarget::All => vec![Box::new(ClaudeHost), Box::new(CodexHost)],
        InstallTarget::Auto => {
            let all: Vec<Box<dyn InstallHost>> = vec![Box::new(ClaudeHost), Box::new(CodexHost)];
            all.into_iter().filter(|h| h.is_available()).collect()
        }
    }
}

/// Whether the Cursor host is part of the selection for `target` (B-001):
/// explicit `cursor`/`all` always select it; `auto` selects it only when it
/// is detected and the platform has an approved command renderer.
pub(in crate::install) fn cursor_selected(target: InstallTarget) -> bool {
    match target {
        InstallTarget::Cursor | InstallTarget::All => true,
        InstallTarget::Auto => cursor::cursor_detected() && cursor::cursor_renderer_supported(),
        InstallTarget::Claude | InstallTarget::Codex => false,
    }
}
