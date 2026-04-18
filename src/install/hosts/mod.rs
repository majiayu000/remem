mod claude;
mod codex;

pub(in crate::install) use claude::ClaudeHost;
pub(in crate::install) use codex::CodexHost;

use crate::install::host::{InstallHost, InstallTarget};

/// Resolve the concrete list of hosts to act on for a given target.
///
/// - `Claude` / `Codex`: single explicit host (always acted upon, even if
///   config missing — the user asked for it).
/// - `Auto`: only hosts whose config dir exists.
/// - `All`: every known host, regardless of whether it's currently installed.
pub(in crate::install) fn resolve_hosts(target: InstallTarget) -> Vec<Box<dyn InstallHost>> {
    match target {
        InstallTarget::Claude => vec![Box::new(ClaudeHost)],
        InstallTarget::Codex => vec![Box::new(CodexHost)],
        InstallTarget::All => vec![Box::new(ClaudeHost), Box::new(CodexHost)],
        InstallTarget::Auto => {
            let all: Vec<Box<dyn InstallHost>> =
                vec![Box::new(ClaudeHost), Box::new(CodexHost)];
            all.into_iter().filter(|h| h.is_available()).collect()
        }
    }
}
