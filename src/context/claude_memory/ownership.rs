//! Claude `autoMemoryDirectory` ownership status (GH-852 §2, read-only).
//!
//! The native delivery bridge is **no-go** on every host/version until the
//! isolated real-host PoC (SP852-T1) proves startup-window loading, hook
//! failure propagation, and capacity behavior, and a human records a per-host
//! go decision. Until then remem stays in `hook_only` mode: this module only
//! *reports* the effective user-scope setting and conflicts — it never writes
//! Claude settings or takes over a directory.
//!
//! Product policy (B-002): remem would only ever accept user, policy, or an
//! explicitly chosen `--settings` source; project/local scopes are rejected
//! outright, so this status probe reads the user scope only.

use std::path::PathBuf;

/// Delivery mode of the Claude native-memory bridge.
/// `native_active` / `inconsistent` states become reachable only after a
/// PoC-backed go decision ships the takeover; today the only value the
/// runtime can produce is `hook_only`.
pub(crate) const NATIVE_BRIDGE_STATE_HOOK_ONLY: &str = "hook_only";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeBridgeStatus {
    pub state: &'static str,
    /// `autoMemoryDirectory` value found in user-scope Claude settings, if
    /// any. remem does not own this value; a non-empty value is a conflict
    /// that any future takeover dry-run must surface, never overwrite.
    pub user_auto_memory_directory: Option<String>,
    pub reason: &'static str,
}

pub(crate) fn native_bridge_status() -> NativeBridgeStatus {
    let settings = user_settings_path();
    NativeBridgeStatus {
        state: NATIVE_BRIDGE_STATE_HOOK_ONLY,
        user_auto_memory_directory: read_auto_memory_directory(&settings),
        reason: "native delivery bridge is no-go pending SP852-T1 real-host PoC evidence; \
                 SessionStart injection remains the only delivery path",
    }
}

fn user_settings_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join("settings.json")
}

fn read_auto_memory_directory(settings_path: &std::path::Path) -> Option<String> {
    let raw = std::fs::read_to_string(settings_path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    value
        .get("autoMemoryDirectory")
        .and_then(|entry| entry.as_str())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_stays_hook_only_without_poc_go_decision() {
        let status = native_bridge_status();
        assert_eq!(status.state, NATIVE_BRIDGE_STATE_HOOK_ONLY);
        assert!(status.reason.contains("no-go"));
    }

    #[test]
    fn reads_user_scope_auto_memory_directory_when_present() -> anyhow::Result<()> {
        let dir = std::env::temp_dir().join(format!(
            "remem-ownership-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir)?;
        let settings = dir.join("settings.json");
        std::fs::write(
            &settings,
            r#"{"autoMemoryDirectory": "~/custom-memory", "unknownKey": {"keep": true}}"#,
        )?;

        assert_eq!(
            read_auto_memory_directory(&settings).as_deref(),
            Some("~/custom-memory")
        );
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn missing_or_invalid_settings_reads_as_unset() {
        assert_eq!(
            read_auto_memory_directory(std::path::Path::new("/nonexistent/settings.json")),
            None
        );
    }
}
