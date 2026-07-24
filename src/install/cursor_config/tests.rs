use anyhow::Result;
use serde_json::{json, Value};

use super::plan::{
    build_receipt, classify_remem_mcp, managed_cursor_mcp_entry, managed_hook_components,
    CursorConfigPlan, CursorOperation, FileAction, McpOwnership,
};
use super::schema::{validate_hooks_document, validate_mcp_document};
use super::writer::apply_plan;
use super::{canonical_json_digest, CursorCapabilityGates, CURRENT_CAPABILITY_GATES};

// ---------------------------------------------------------------------------
// Isolated HOME + REMEM_CONFIG environment for plan/apply tests.
// ---------------------------------------------------------------------------

struct CursorTestEnv {
    _guard: crate::runtime_config::TestEnvGuard,
    previous_home: Option<std::ffi::OsString>,
    previous_config: Option<std::ffi::OsString>,
    home: std::path::PathBuf,
}

impl CursorTestEnv {
    fn new(label: &str) -> Result<Self> {
        let guard = crate::runtime_config::TEST_ENV_LOCK
            .lock()
            .expect("env lock should acquire");
        let home = std::env::temp_dir().join(format!(
            "remem-cursor-install-{label}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".cursor"))?;
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

    fn hooks_path(&self) -> std::path::PathBuf {
        self.home.join(".cursor").join("hooks.json")
    }

    fn mcp_path(&self) -> std::path::PathBuf {
        self.home.join(".cursor").join("mcp.json")
    }

    fn config_path(&self) -> std::path::PathBuf {
        self.home.join("config.toml")
    }
}

impl Drop for CursorTestEnv {
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

// ---------------------------------------------------------------------------
// Schema: hooks.json (B-003)
// ---------------------------------------------------------------------------

fn valid_hooks_doc() -> Value {
    json!({
        "version": 1,
        "hooks": {
            "postToolUse": [
                { "command": "/tools/other observe", "timeout": 30, "matcher": "Read" }
            ],
            "stop": [
                { "type": "command", "command": "/tools/other stop", "loop_limit": 2 },
                { "type": "prompt", "prompt": "wrap up", "failClosed": true }
            ],
            "sessionStart": [
                { "command": "/tools/remem-helper context" }
            ]
        }
    })
}

#[test]
fn hooks_schema_accepts_frozen_v1_shapes() {
    assert!(validate_hooks_document(&valid_hooks_doc()).is_ok());
}

#[test]
fn hooks_schema_rejects_whole_document_errors() {
    let cases: Vec<(Value, &str)> = vec![
        (json!([]), "hooks_root_not_object"),
        (json!({"hooks": {}}), "hooks_version_missing"),
        (
            json!({"version": 2, "hooks": {}}),
            "hooks_version_not_integer_1",
        ),
        (
            json!({"version": 1.0, "hooks": {}}),
            "hooks_version_not_integer_1",
        ),
        (
            json!({"version": "1", "hooks": {}}),
            "hooks_version_not_integer_1",
        ),
        (json!({"version": 1}), "hooks_container_missing"),
        (
            json!({"version": 1, "hooks": []}),
            "hooks_container_not_object",
        ),
        (
            json!({"version": 1, "hooks": {"notAnEvent": []}}),
            "hooks_unknown_event",
        ),
        (
            json!({"version": 1, "hooks": {"stop": {}}}),
            "hooks_event_not_array",
        ),
        (
            json!({"version": 1, "hooks": {"stop": ["x"]}}),
            "hooks_entry_not_object",
        ),
        (
            json!({"version": 1, "hooks": {"stop": [{"command": "a", "prompt": "b"}]}}),
            "hooks_entry_shape_mismatch",
        ),
        (
            json!({"version": 1, "hooks": {"stop": [{"type": "prompt", "command": "a", "prompt": "b"}]}}),
            "hooks_entry_shape_mismatch",
        ),
        (
            json!({"version": 1, "hooks": {"stop": [{"command": ""}]}}),
            "hooks_command_empty",
        ),
        (
            json!({"version": 1, "hooks": {"stop": [{"command": "a", "timeout": 0}]}}),
            "hooks_timeout_invalid",
        ),
        (
            json!({"version": 1, "hooks": {"stop": [{"command": "a", "extra": true}]}}),
            "hooks_entry_unknown_field",
        ),
        // matcher is event-gated: sessionStart is not in the closed list.
        (
            json!({"version": 1, "hooks": {"sessionStart": [{"command": "a", "matcher": "x"}]}}),
            "hooks_matcher_not_allowed_on_event",
        ),
        // loop_limit only on stop/subagentStop, positive integer or null.
        (
            json!({"version": 1, "hooks": {"postToolUse": [{"command": "a", "loop_limit": 1}]}}),
            "hooks_loop_limit_not_allowed_on_event",
        ),
        (
            json!({"version": 1, "hooks": {"stop": [{"command": "a", "loop_limit": 0}]}}),
            "hooks_loop_limit_invalid",
        ),
    ];
    for (doc, expected_code) in cases {
        let error = validate_hooks_document(&doc).expect_err(expected_code);
        assert_eq!(error.code, expected_code, "{doc}");
    }
}

#[test]
fn hooks_schema_accepts_null_loop_limit_on_stop() {
    let doc = json!({"version": 1, "hooks": {"stop": [{"command": "a", "loop_limit": null}]}});
    assert!(validate_hooks_document(&doc).is_ok());
}

// ---------------------------------------------------------------------------
// Schema: mcp.json (B-004)
// ---------------------------------------------------------------------------

#[test]
fn mcp_schema_accepts_frozen_foreign_variants() {
    let doc = json!({
        "mcpServers": {
            "explicit": {
                "type": "stdio",
                "command": "/bin/tool",
                "args": ["serve"],
                "env": {"KEY": "value"},
                "envFile": ".env"
            },
            "documented_example": { "command": "/bin/tool2", "args": [] },
            "remote": {
                "url": "https://example.invalid/mcp",
                "type": "http",
                "headers": {"Authorization": "Bearer x"},
                "auth": {
                    "CLIENT_ID": "id",
                    "CLIENT_SECRET": "secret",
                    "scopes": ["read"]
                }
            }
        }
    });
    assert!(validate_mcp_document(&doc).is_ok());
    // Missing container is a valid state.
    assert!(validate_mcp_document(&json!({})).is_ok());
}

#[test]
fn mcp_schema_rejects_malformed_servers() {
    let cases: Vec<(Value, &str)> = vec![
        (json!([]), "mcp_root_not_object"),
        (json!({"mcpServers": []}), "mcp_servers_not_object"),
        (json!({"mcpServers": {"a": null}}), "mcp_server_not_object"),
        (
            json!({"mcpServers": {"a": {"command": "x", "url": "y"}}}),
            "mcp_server_mixed_transport",
        ),
        (
            json!({"mcpServers": {"a": {}}}),
            "mcp_server_missing_transport",
        ),
        (
            json!({"mcpServers": {"a": {"command": ""}}}),
            "mcp_stdio_command_invalid",
        ),
        (
            json!({"mcpServers": {"a": {"type": "http", "command": "x"}}}),
            "mcp_stdio_type_invalid",
        ),
        (
            json!({"mcpServers": {"a": {"command": "x", "transport": "stdio"}}}),
            "mcp_stdio_unknown_field",
        ),
        (
            json!({"mcpServers": {"a": {"command": "x", "args": [1]}}}),
            "mcp_stdio_args_invalid",
        ),
        (
            json!({"mcpServers": {"a": {"url": "u", "type": "ws"}}}),
            "mcp_remote_type_invalid",
        ),
        (
            json!({"mcpServers": {"a": {"url": "u", "auth": {"CLIENT_ID": ""}}}}),
            "mcp_remote_auth_client_id_invalid",
        ),
        (
            json!({"mcpServers": {"a": {"url": "u", "auth": {"CLIENT_ID": "x", "extra": 1}}}}),
            "mcp_remote_auth_unknown_field",
        ),
    ];
    for (doc, expected_code) in cases {
        let error = validate_mcp_document(&doc).expect_err(expected_code);
        assert_eq!(error.code, expected_code, "{doc}");
    }
}

// ---------------------------------------------------------------------------
// Managed builder + ownership (B-004..B-007)
// ---------------------------------------------------------------------------

#[test]
fn managed_mcp_entry_is_exact_stdio_shape() {
    assert_eq!(
        managed_cursor_mcp_entry(BIN),
        json!({"type": "stdio", "command": BIN, "args": ["mcp"]})
    );
}

#[test]
fn current_capability_gates_install_no_hook_components() {
    // The merged GH-823 runtime has no total delivered-failure policy, MCP
    // ownership is generic, and GH-825's reader is unproven: the contract v1
    // hook component set must be empty (B-005).
    assert!(managed_hook_components(BIN, CURRENT_CAPABILITY_GATES).is_empty());
    let receipt = build_receipt(BIN, false);
    assert_eq!(receipt.mode, "full");
    assert_eq!(receipt.components.len(), 1);
    assert_eq!(receipt.components[0].component, "mcp_server_v1");
}

#[test]
fn hypothetical_open_gates_render_frozen_component_table() {
    let gates = CursorCapabilityGates {
        observe_total_failure_policy: true,
        mcp_specific_per_call_id: true,
        summarize_reader_proven: true,
    };
    let components = managed_hook_components("/path with space/remem", gates);
    let ids: Vec<&str> = components.iter().map(|c| c.component.as_str()).collect();
    assert_eq!(
        ids,
        [
            "observe_generic_success_v1",
            "observe_generic_failure_v1",
            "observe_mcp_specific_v1",
            "summarize_stop_v1"
        ]
    );
    // POSIX shell_quote: space path is single-quoted, fixed tail appended.
    assert_eq!(
        components[0].rendered_command.as_deref(),
        Some("'/path with space/remem' observe --host cursor")
    );
    assert_eq!(components[3].key, "stop");
    assert!(components.iter().all(|c| c.timeout == Some(120)));

    // Success capture never installs without the failure half of the bundle.
    let success_only = managed_hook_components(
        BIN,
        CursorCapabilityGates {
            observe_total_failure_policy: false,
            mcp_specific_per_call_id: true,
            summarize_reader_proven: false,
        },
    );
    assert!(success_only.is_empty());
}

#[test]
fn mcp_ownership_requires_exact_proof() {
    let receipt = build_receipt("/old/bin/remem", false);
    // (a) current builder equality.
    let current = managed_cursor_mcp_entry(BIN);
    assert_eq!(
        classify_remem_mcp(Some(&current), BIN, Some(&receipt)),
        McpOwnership::OwnedCurrent
    );
    // (b) receipt-bound old binary path with matching digest.
    let old = managed_cursor_mcp_entry("/old/bin/remem");
    assert_eq!(
        classify_remem_mcp(Some(&old), BIN, Some(&receipt)),
        McpOwnership::OwnedReceipt
    );
    // Key/basename/substring alone never proves ownership.
    let lookalike = json!({"type": "stdio", "command": "/other/remem", "args": ["mcp"]});
    assert_eq!(
        classify_remem_mcp(Some(&lookalike), BIN, Some(&receipt)),
        McpOwnership::Collision
    );
    let extra_field = json!({"type": "stdio", "command": BIN, "args": ["mcp"], "env": {}});
    assert_eq!(
        classify_remem_mcp(Some(&extra_field), BIN, None),
        McpOwnership::Collision
    );
    assert_eq!(classify_remem_mcp(None, BIN, None), McpOwnership::Absent);
}

#[test]
fn canonical_digest_is_key_order_independent() {
    let a: Value = serde_json::from_str(r#"{"b":1,"a":{"y":2,"x":3}}"#).unwrap();
    let b: Value = serde_json::from_str(r#"{"a":{"x":3,"y":2},"b":1}"#).unwrap();
    assert_eq!(canonical_json_digest(&a), canonical_json_digest(&b));
}

// ---------------------------------------------------------------------------
// Plan + apply integration (B-008, B-009, B-013..B-017)
// ---------------------------------------------------------------------------

fn parse_file(path: &std::path::Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).expect("read json file"))
        .expect("parse json file")
}

#[test]
fn install_apply_uninstall_roundtrip_preserves_foreign_data() -> Result<()> {
    let env = CursorTestEnv::new("roundtrip")?;
    // Foreign data designed to trip substring-based cleanup: a server key
    // and hook command that merely contain "remem".
    std::fs::write(
        env.hooks_path(),
        serde_json::to_string_pretty(&json!({
            "version": 1,
            "hooks": {
                "stop": [{ "command": "/tools/remem-fan summarize", "timeout": 5 }],
                "postToolUse": [{ "command": "/usr/local/bin/remem unknown-subcommand" }]
            }
        }))?,
    )?;
    std::fs::write(
        env.mcp_path(),
        serde_json::to_string_pretty(&json!({
            "mcpServers": {
                "remem-helper": {
                    "type": "stdio",
                    "command": "/bin/helper",
                    "env": { "SECRET_TOKEN": "sentinel-secret" }
                }
            }
        }))?,
    )?;
    let foreign_hooks_before = parse_file(&env.hooks_path());

    // Install.
    let plan = CursorConfigPlan::build(CursorOperation::Install { hooks_only: false }, BIN)?;
    assert_eq!(plan.hooks.action, FileAction::NoOp);
    assert_eq!(plan.mcp.action, FileAction::Add);
    apply_plan(&plan)?;

    let mcp = parse_file(&env.mcp_path());
    assert_eq!(mcp["mcpServers"]["remem"], managed_cursor_mcp_entry(BIN));
    assert_eq!(
        mcp["mcpServers"]["remem-helper"]["env"]["SECRET_TOKEN"],
        "sentinel-secret"
    );
    assert_eq!(parse_file(&env.hooks_path()), foreign_hooks_before);
    let config_text = std::fs::read_to_string(env.config_path())?;
    assert!(
        config_text.contains("[memory_ai.hosts.cursor]"),
        "{config_text}"
    );
    assert!(
        config_text.contains("context_gate = \"strict\""),
        "{config_text}"
    );
    assert!(
        config_text.contains("capture_adapter = \"cursor\""),
        "{config_text}"
    );
    assert!(config_text.contains("install_receipt"), "{config_text}");

    // Idempotency: a second install converges to a no-op plan.
    let second = CursorConfigPlan::build(CursorOperation::Install { hooks_only: false }, BIN)?;
    assert_eq!(second.mcp.action, FileAction::NoOp);
    assert_eq!(second.runtime_config.action, FileAction::NoOp);
    apply_plan(&second)?;

    // Uninstall removes exactly the managed entry and clears the receipt.
    let uninstall = CursorConfigPlan::build(CursorOperation::Uninstall, BIN)?;
    assert_eq!(uninstall.mcp.action, FileAction::Remove);
    assert_eq!(uninstall.hooks.action, FileAction::NoOp);
    apply_plan(&uninstall)?;
    let mcp_after = parse_file(&env.mcp_path());
    assert!(mcp_after["mcpServers"].get("remem").is_none());
    assert_eq!(
        mcp_after["mcpServers"]["remem-helper"]["env"]["SECRET_TOKEN"],
        "sentinel-secret"
    );
    assert_eq!(parse_file(&env.hooks_path()), foreign_hooks_before);
    assert!(!std::fs::read_to_string(env.config_path())?.contains("install_receipt"));
    assert!(env.mcp_path().exists(), "user file must not be deleted");

    // Uninstall twice is a successful no-op.
    let again = CursorConfigPlan::build(CursorOperation::Uninstall, BIN)?;
    assert_eq!(again.mcp.action, FileAction::NoOp);
    apply_plan(&again)?;
    Ok(())
}

#[test]
fn malformed_input_fails_closed_before_any_write() -> Result<()> {
    let env = CursorTestEnv::new("malformed")?;
    std::fs::write(env.hooks_path(), "{ not json")?;
    let mcp_before = serde_json::to_string_pretty(&json!({"mcpServers": {}}))?;
    std::fs::write(env.mcp_path(), &mcp_before)?;

    let error = CursorConfigPlan::build(CursorOperation::Install { hooks_only: false }, BIN)
        .expect_err("malformed hooks must fail closed");
    assert!(format!("{error:#}").contains("malformed_json"), "{error:#}");

    // Zero side effects: both raw files unchanged, no runtime config.
    assert_eq!(std::fs::read_to_string(env.hooks_path())?, "{ not json");
    assert_eq!(std::fs::read_to_string(env.mcp_path())?, mcp_before);
    assert!(!env.config_path().exists());

    // Uninstall uses the same validator and also fails closed (B-017).
    let error = CursorConfigPlan::build(CursorOperation::Uninstall, BIN)
        .expect_err("malformed hooks must fail uninstall too");
    assert!(format!("{error:#}").contains("malformed_json"), "{error:#}");
    Ok(())
}

#[test]
fn schema_drift_and_collision_fail_closed() -> Result<()> {
    let env = CursorTestEnv::new("collision")?;
    // Whole-document validation: a malformed foreign server (not remem)
    // fails the operation.
    std::fs::write(
        env.mcp_path(),
        serde_json::to_string_pretty(&json!({
            "mcpServers": { "foreign": { "command": "x", "unknownField": 1 } }
        }))?,
    )?;
    let error = CursorConfigPlan::build(CursorOperation::Install { hooks_only: false }, BIN)
        .expect_err("foreign malformed server must fail closed");
    assert!(
        format!("{error:#}").contains("mcp_stdio_unknown_field"),
        "{error:#}"
    );

    // A user entry occupying mcpServers.remem is a collision for install
    // and uninstall.
    std::fs::write(
        env.mcp_path(),
        serde_json::to_string_pretty(&json!({
            "mcpServers": { "remem": { "url": "https://example.invalid/other" } }
        }))?,
    )?;
    for operation in [
        CursorOperation::Install { hooks_only: false },
        CursorOperation::Uninstall,
    ] {
        let error =
            CursorConfigPlan::build(operation, BIN).expect_err("collision must fail closed");
        assert!(
            format!("{error:#}").contains("ownership_collision"),
            "{error:#}"
        );
    }
    Ok(())
}

#[test]
fn receipt_bound_old_binary_path_upgrades_exactly_once() -> Result<()> {
    let env = CursorTestEnv::new("upgrade")?;
    let old_bin = "/old/location/remem";
    std::fs::write(
        env.mcp_path(),
        serde_json::to_string_pretty(&json!({
            "mcpServers": { "remem": managed_cursor_mcp_entry(old_bin) }
        }))?,
    )?;
    let receipt = build_receipt(old_bin, false);
    let receipt_json = serde_json::to_string(&receipt)?;
    std::fs::write(
        env.config_path(),
        format!(
            "[memory_ai.hosts.cursor]\nmemory_profile = \"codex\"\ncontext_gate = \"strict\"\ncontext_color = true\ncapture_adapter = \"cursor\"\ninstall_receipt = {}\n",
            toml_edit::Value::from(receipt_json)
        ),
    )?;

    let plan = CursorConfigPlan::build(CursorOperation::Install { hooks_only: false }, BIN)?;
    assert_eq!(plan.mcp.action, FileAction::Replace);
    apply_plan(&plan)?;
    let mcp = parse_file(&env.mcp_path());
    assert_eq!(mcp["mcpServers"]["remem"], managed_cursor_mcp_entry(BIN));
    let config_text = std::fs::read_to_string(env.config_path())?;
    assert!(
        config_text.contains(BIN),
        "receipt must record the new path: {config_text}"
    );
    Ok(())
}

#[test]
fn capture_adapter_identity_boundary_rejects_other_hosts() -> Result<()> {
    let env = CursorTestEnv::new("adapter")?;
    std::fs::write(
        env.config_path(),
        "[memory_ai.hosts.cursor]\ncapture_adapter = \"claude-code\"\n",
    )?;
    let error = CursorConfigPlan::build(CursorOperation::Install { hooks_only: false }, BIN)
        .expect_err("wrong capture_adapter must fail closed");
    assert!(
        format!("{error:#}").contains("capture_adapter_invalid"),
        "{error:#}"
    );

    // Explicit valid values for other fields are preserved, missing fields
    // materialize the exact defaults.
    std::fs::write(
        env.config_path(),
        "[memory_ai.hosts.cursor]\ncontext_gate = \"auto\"\n",
    )?;
    let plan = CursorConfigPlan::build(CursorOperation::Install { hooks_only: false }, BIN)?;
    apply_plan(&plan)?;
    let config_text = std::fs::read_to_string(env.config_path())?;
    assert!(
        config_text.contains("context_gate = \"auto\""),
        "{config_text}"
    );
    assert!(
        config_text.contains("memory_profile = \"codex\""),
        "{config_text}"
    );
    assert!(
        config_text.contains("capture_adapter = \"cursor\""),
        "{config_text}"
    );

    // Illegal closed-set value fails.
    std::fs::write(
        env.config_path(),
        "[memory_ai.hosts.cursor]\ncontext_gate = \"maybe\"\n",
    )?;
    let error = CursorConfigPlan::build(CursorOperation::Install { hooks_only: false }, BIN)
        .expect_err("illegal context_gate must fail closed");
    assert!(
        format!("{error:#}").contains("host_config_invalid"),
        "{error:#}"
    );
    Ok(())
}

#[test]
fn hooks_only_mode_validates_but_never_writes_mcp() -> Result<()> {
    let env = CursorTestEnv::new("hooks-only")?;
    let plan = CursorConfigPlan::build(CursorOperation::Install { hooks_only: true }, BIN)?;
    assert_eq!(plan.mcp.action, FileAction::NoOp);
    assert_eq!(plan.mcp.reason, "validated/no-change (hooks-only)");
    apply_plan(&plan)?;
    assert!(
        !env.mcp_path().exists(),
        "hooks-only must not create mcp.json"
    );
    let config_text = std::fs::read_to_string(env.config_path())?;
    assert!(config_text.contains("hooks_only"), "{config_text}");

    // Intentional hooks-only converges and stays receipt-consistent.
    let plan = CursorConfigPlan::build(CursorOperation::Install { hooks_only: true }, BIN)?;
    assert_eq!(plan.runtime_config.action, FileAction::NoOp);
    Ok(())
}

#[test]
fn dry_run_lines_show_both_absolute_paths_and_actions() -> Result<()> {
    let env = CursorTestEnv::new("dry-run")?;
    let plan = CursorConfigPlan::build(CursorOperation::Install { hooks_only: false }, BIN)?;
    let lines = plan.dry_run_lines().join("\n");
    assert!(
        lines.contains(&env.hooks_path().display().to_string()),
        "{lines}"
    );
    assert!(
        lines.contains(&env.mcp_path().display().to_string()),
        "{lines}"
    );
    assert!(lines.contains("[add]"), "{lines}");
    assert!(lines.contains("[no-op]"), "{lines}");
    // Dry-run rendering itself has zero side effects.
    assert!(!env.mcp_path().exists());
    assert!(!env.config_path().exists());
    Ok(())
}

// ---------------------------------------------------------------------------
// Secure staged writer + rollback (B-010..B-013)
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn secure_writer_leaves_target_owner_only_even_when_previously_permissive() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let env = CursorTestEnv::new("perms")?;
    // Pre-existing world-readable target with a foreign secret.
    std::fs::write(
        env.mcp_path(),
        serde_json::to_string_pretty(&json!({
            "mcpServers": {
                "foreign": { "command": "/bin/x", "env": { "TOKEN": "sentinel" } }
            }
        }))?,
    )?;
    std::fs::set_permissions(env.mcp_path(), std::fs::Permissions::from_mode(0o644))?;

    let plan = CursorConfigPlan::build(CursorOperation::Install { hooks_only: false }, BIN)?;
    apply_plan(&plan)?;

    let mode = std::fs::metadata(env.mcp_path())?.permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "replaced target must stay owner-only");
    // No temp residue in the directory.
    let residue: Vec<_> = std::fs::read_dir(env.home.join(".cursor"))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp"))
        .collect();
    assert!(residue.is_empty(), "no temp files may remain");
    Ok(())
}

#[test]
fn concurrent_edit_before_final_compare_aborts_and_preserves_external_bytes() -> Result<()> {
    let env = CursorTestEnv::new("concurrent")?;
    let plan = CursorConfigPlan::build(CursorOperation::Install { hooks_only: false }, BIN)?;
    // Non-cooperating writer lands after plan snapshot, before apply.
    let external = serde_json::to_string_pretty(&json!({
        "mcpServers": { "user-added": { "command": "/bin/new" } }
    }))?;
    std::fs::write(env.mcp_path(), &external)?;

    let error = apply_plan(&plan).expect_err("observable concurrent edit must abort");
    assert!(
        format!("{error:#}").contains("concurrent_edit"),
        "{error:#}"
    );
    assert_eq!(std::fs::read_to_string(env.mcp_path())?, external);
    Ok(())
}

#[test]
fn later_target_failure_rolls_back_committed_targets() -> Result<()> {
    let env = CursorTestEnv::new("rollback")?;
    let mcp_before = serde_json::to_string_pretty(&json!({
        "mcpServers": { "foreign": { "command": "/bin/x" } }
    }))?;
    std::fs::write(env.mcp_path(), &mcp_before)?;

    let plan = CursorConfigPlan::build(CursorOperation::Install { hooks_only: false }, BIN)?;
    super::writer::fail_next_rename_for_path_for_test(&env.config_path());
    let error = apply_plan(&plan).expect_err("injected runtime config failure must abort");
    super::writer::clear_failpoints_for_test();
    let rendered = format!("{error:#}");
    assert!(
        rendered.contains("injected cursor staged rename failure"),
        "{rendered}"
    );
    assert!(rendered.contains("compensating rollback"), "{rendered}");

    // The already-committed MCP write was restored to the snapshot.
    assert_eq!(std::fs::read_to_string(env.mcp_path())?, mcp_before);
    assert!(!env.config_path().exists(), "config must not be committed");
    Ok(())
}
