use anyhow::Result;
use serde_json::json;

use super::super::types::Status;
use super::check_cursor_install;
use crate::install::cursor_config::plan::build_receipt;
use crate::install::cursor_config::plan::managed_cursor_mcp_entry;

struct DoctorTestEnv {
    _guard: crate::runtime_config::TestEnvGuard,
    previous_home: Option<std::ffi::OsString>,
    previous_config: Option<std::ffi::OsString>,
    home: std::path::PathBuf,
}

impl DoctorTestEnv {
    fn new(label: &str, create_cursor_dir: bool) -> Result<Self> {
        let guard = crate::runtime_config::TEST_ENV_LOCK
            .lock()
            .expect("env lock should acquire");
        let home = std::env::temp_dir().join(format!(
            "remem-cursor-doctor-{label}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let _ = std::fs::remove_dir_all(&home);
        if create_cursor_dir {
            std::fs::create_dir_all(home.join(".cursor"))?;
        } else {
            std::fs::create_dir_all(&home)?;
        }
        let previous_home = std::env::var_os("HOME");
        let previous_config = std::env::var_os("REMEM_CONFIG");
        std::env::set_var("HOME", &home);
        std::env::set_var("REMEM_CONFIG", home.join("config.toml"));
        Ok(Self {
            _guard: guard,
            previous_home,
            previous_config,
            home,
        })
    }

    fn write_receipt(&self, bin: &str, hooks_only: bool) -> Result<()> {
        let receipt = build_receipt(bin, hooks_only);
        let receipt_json = serde_json::to_string(&receipt)?;
        std::fs::write(
            self.home.join("config.toml"),
            format!(
                "[memory_ai.hosts.cursor]\nmemory_profile = \"codex\"\ncapture_adapter = \"cursor\"\ninstall_receipt = {}\n",
                toml_edit::Value::from(receipt_json)
            ),
        )?;
        Ok(())
    }

    fn write_mcp(&self, doc: &serde_json::Value) -> Result<()> {
        std::fs::write(
            self.home.join(".cursor").join("mcp.json"),
            serde_json::to_string_pretty(doc)?,
        )?;
        Ok(())
    }
}

impl Drop for DoctorTestEnv {
    fn drop(&mut self) {
        match self.previous_home.as_ref() {
            Some(previous) => std::env::set_var("HOME", previous),
            None => std::env::remove_var("HOME"),
        }
        match self.previous_config.as_ref() {
            Some(previous) => std::env::set_var("REMEM_CONFIG", previous),
            None => std::env::remove_var("REMEM_CONFIG"),
        }
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

const BIN: &str = "/opt/remem/bin/remem";

fn assert_mandatory_capability_lines(detail: &str) {
    // B-018: the exact session-init line and the failure-policy /
    // per-capability effective dimensions appear in every Cursor state.
    assert!(
        detail
            .lines()
            .any(|line| line == "session-init: not supported on cursor"),
        "{detail}"
    );
    assert!(detail.contains("session_init: unsupported"), "{detail}");
    assert!(
        detail.contains("hook_failure_policy: host_continues"),
        "{detail}"
    );
    assert!(detail.contains("postToolUse_delivery: proven"), "{detail}");
    assert!(
        detail.contains("postToolUse_managed_context: not_configured"),
        "{detail}"
    );
    assert!(detail.contains("sessionStart: blocked"), "{detail}");
    assert!(detail.contains("stop: unknown"), "{detail}");
    assert!(detail.contains("preCompact: unknown"), "{detail}");
}

#[test]
fn undetected_cursor_reports_detected_false_with_capability_lines() -> Result<()> {
    let _env = DoctorTestEnv::new("undetected", false)?;
    let check = check_cursor_install();
    assert!(check.detail.contains("detected=false"), "{}", check.detail);
    assert_mandatory_capability_lines(&check.detail);
    Ok(())
}

#[test]
fn detected_but_unconfigured_warns_not_fails() -> Result<()> {
    let _env = DoctorTestEnv::new("unconfigured", true)?;
    let check = check_cursor_install();
    assert_eq!(check.status, Status::Warn, "{}", check.detail);
    assert!(
        check.detail.contains("configured_mode=none"),
        "{}",
        check.detail
    );
    assert_mandatory_capability_lines(&check.detail);
    Ok(())
}

#[test]
fn full_install_with_matching_receipt_is_configured() -> Result<()> {
    let env = DoctorTestEnv::new("full", true)?;
    env.write_mcp(&json!({ "mcpServers": { "remem": managed_cursor_mcp_entry(BIN) } }))?;
    env.write_receipt(BIN, false)?;
    let check = check_cursor_install();
    assert_eq!(check.status, Status::Ok, "{}", check.detail);
    assert!(
        check
            .detail
            .contains("configured=true configured_mode=full"),
        "{}",
        check.detail
    );
    assert_mandatory_capability_lines(&check.detail);
    Ok(())
}

#[test]
fn intentional_hooks_only_is_not_partial_state() -> Result<()> {
    let env = DoctorTestEnv::new("hooks-only", true)?;
    env.write_receipt(BIN, true)?;
    let check = check_cursor_install();
    assert_eq!(check.status, Status::Ok, "{}", check.detail);
    assert!(
        check.detail.contains("configured_mode=hooks_only"),
        "{}",
        check.detail
    );
    assert!(!check.detail.contains("partial_state:"), "{}", check.detail);
    assert_mandatory_capability_lines(&check.detail);
    Ok(())
}

#[test]
fn receipt_full_without_mcp_entry_is_partial_state() -> Result<()> {
    let env = DoctorTestEnv::new("partial", true)?;
    env.write_receipt(BIN, false)?;
    let check = check_cursor_install();
    assert_eq!(check.status, Status::Fail, "{}", check.detail);
    assert!(check.detail.contains("partial_state:"), "{}", check.detail);
    assert!(
        check.detail.contains("configured_mode=unknown"),
        "{}",
        check.detail
    );
    assert_mandatory_capability_lines(&check.detail);
    Ok(())
}

#[test]
fn malformed_hooks_file_fails_with_path_and_capability_lines() -> Result<()> {
    let env = DoctorTestEnv::new("malformed", true)?;
    std::fs::write(env.home.join(".cursor").join("hooks.json"), "{ not json")?;
    let check = check_cursor_install();
    assert_eq!(check.status, Status::Fail, "{}", check.detail);
    assert!(check.detail.contains("malformed:"), "{}", check.detail);
    assert!(check.detail.contains("hooks.json"), "{}", check.detail);
    assert_mandatory_capability_lines(&check.detail);
    Ok(())
}

#[test]
fn foreign_entry_on_managed_key_reports_collision() -> Result<()> {
    let env = DoctorTestEnv::new("collision", true)?;
    env.write_mcp(&json!({
        "mcpServers": { "remem": { "url": "https://example.invalid/other" } }
    }))?;
    env.write_receipt(BIN, false)?;
    let check = check_cursor_install();
    assert_eq!(check.status, Status::Fail, "{}", check.detail);
    assert!(check.detail.contains("collision:"), "{}", check.detail);
    assert_mandatory_capability_lines(&check.detail);
    Ok(())
}
