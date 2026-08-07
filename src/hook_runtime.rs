//! Process-wide hook invocation mode (GH-952).
//!
//! Host hooks (SessionStart context, session-init, observe, summarize) run
//! under tight host budgets, so embedding network calls made from a hook
//! process are capped to a short deadline. A capped call that times out
//! degrades through the configured embedding fallback chain with an error
//! log instead of stalling the hook.

use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{bail, Result};

pub const DEFAULT_HOOK_EMBEDDING_TIMEOUT_SECS: u64 = 2;
pub const ENV_HOOK_EMBEDDING_TIMEOUT_SECS: &str = "REMEM_EMBEDDINGS_HOOK_TIMEOUT_SECS";

static HOOK_RUNTIME_MODE: AtomicBool = AtomicBool::new(false);

/// Mark this process as a host hook invocation. One-way for the process
/// lifetime: hook entrypoints are always dedicated short-lived processes.
pub fn enter_hook_runtime_mode() {
    HOOK_RUNTIME_MODE.store(true, Ordering::Relaxed);
}

pub fn hook_runtime_mode() -> bool {
    HOOK_RUNTIME_MODE.load(Ordering::Relaxed)
}

/// The embedding network deadline cap for hook processes, in seconds.
pub fn hook_embedding_timeout_secs() -> Result<u64> {
    match std::env::var(ENV_HOOK_EMBEDDING_TIMEOUT_SECS) {
        Ok(raw) if !raw.trim().is_empty() => parse_hook_timeout_secs(raw.trim()),
        _ => Ok(DEFAULT_HOOK_EMBEDDING_TIMEOUT_SECS),
    }
}

fn parse_hook_timeout_secs(raw: &str) -> Result<u64> {
    match raw.parse::<u64>() {
        Ok(secs) if secs > 0 => Ok(secs),
        _ => bail!("{ENV_HOOK_EMBEDDING_TIMEOUT_SECS} must be a positive integer, got {raw:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_hook_timeout_secs;

    #[test]
    fn parses_positive_seconds() {
        assert_eq!(parse_hook_timeout_secs("1").unwrap(), 1);
        assert_eq!(parse_hook_timeout_secs("30").unwrap(), 30);
    }

    #[test]
    fn rejects_zero_and_garbage() {
        assert!(parse_hook_timeout_secs("0").is_err());
        assert!(parse_hook_timeout_secs("-1").is_err());
        assert!(parse_hook_timeout_secs("abc").is_err());
    }
}
