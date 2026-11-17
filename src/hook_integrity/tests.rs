use super::*;
use serde_json::json;

fn make_executable(_paths: &[&Path]) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in _paths {
            let mut permissions = std::fs::metadata(path)
                .expect("binary metadata")
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(path, permissions).expect("make binary executable");
        }
    }
}

#[test]
fn detects_missing_claude_hooks_as_three_of_six() {
    let doc = json!({
        "hooks": {
            "SessionStart": [{
                "matcher": "startup|resume|clear|compact",
                "hooks": [{ "command": "/tmp/remem context --host claude-code", "timeout": 15 }]
            }],
            "UserPromptSubmit": [{
                "hooks": [{ "command": "/tmp/remem session-init --host claude-code", "timeout": 15 }]
            }],
            "PreCompact": [{
                "hooks": [{ "command": "/tmp/remem summarize --host claude-code", "timeout": 120 }]
            }]
        }
    });

    let report = evaluate_hooks(
        &doc,
        "claude",
        PathBuf::from("/tmp/settings.json"),
        Path::new("/tmp/remem"),
        true,
    );

    assert_eq!(report.registered, 3);
    assert_eq!(report.expected, 6);
    assert!(report.missing_events.contains(&"PostToolUse"));
    assert!(report.missing_events.contains(&"Stop"));
    assert!(!report.is_healthy());
}

#[test]
fn detects_matcher_and_timeout_drift() {
    let doc = json!({
        "hooks": {
            "SessionStart": [{
                "matcher": "startup|clear|compact",
                "hooks": [{ "command": "/tmp/remem context --host claude-code", "timeout": 15000 }]
            }],
            "UserPromptSubmit": [{
                "hooks": [{ "command": "/tmp/remem session-init --host claude-code", "timeout": 15 }]
            }],
            "PostToolUse": [{
                "matcher": "Write|Edit|NotebookEdit|Bash|Grep|Glob|Task",
                "hooks": [{ "command": "/tmp/remem observe --host claude-code", "timeout": 120000 }]
            }],
            "PreCompact": [{
                "hooks": [{ "command": "/tmp/remem summarize --host claude-code", "timeout": 120 }]
            }],
            "Stop": [{
                "hooks": [{ "command": "/tmp/remem summarize --host claude-code", "timeout": 120 }]
            }]
        }
    });

    let report = evaluate_hooks(
        &doc,
        "claude",
        PathBuf::from("/tmp/settings.json"),
        Path::new("/tmp/remem"),
        true,
    );

    assert_eq!(report.registered, 3);
    assert!(report
        .stale_details
        .iter()
        .any(|detail| detail.contains("matcher drift")));
    assert!(report
        .stale_details
        .iter()
        .any(|detail| detail.contains("timeout drift")));
}

#[test]
fn codex_prompt_submit_requires_installer_timeout() {
    for (timeout, healthy) in [(Some(15), true), (None, false), (Some(1), false)] {
        let mut doc = json!({
            "hooks": {
                "SessionStart": [{
                    "hooks": [{
                        "command": "/tmp/remem context --host codex-cli",
                        "timeout": 15
                    }]
                }],
                "UserPromptSubmit": [{
                    "hooks": [{
                        "command": "/tmp/remem session-init --host codex-cli",
                        "timeout": timeout
                    }]
                }],
                "Stop": [{
                    "hooks": [{
                        "command": "/tmp/remem summarize --host codex-cli",
                        "timeout": 120
                    }]
                }]
            }
        });
        let prompt_hook = &mut doc["hooks"]["UserPromptSubmit"][0]["hooks"][0];
        if timeout.is_none() {
            prompt_hook
                .as_object_mut()
                .and_then(|hook| hook.remove("timeout"));
        }

        let report = evaluate_hooks(
            &doc,
            "codex",
            PathBuf::from("/tmp/settings.json"),
            Path::new("/tmp/remem"),
            true,
        );

        assert_eq!(report.is_healthy(), healthy, "timeout={timeout:?}");
        if !healthy {
            assert!(report
                .stale_details
                .iter()
                .any(|detail| detail.contains("UserPromptSubmit")
                    && detail.contains("timeout drift")));
            assert!(report
                .warning_block()
                .contains("Repair: remem install --target codex --repair"));
        }
    }
}

#[test]
fn parses_exec_form_hooks() {
    let hook = json!({
        "type": "command",
        "command": "/old/remem",
        "args": ["observe", "--host", "claude-code"]
    });

    let invocation = parse_remem_hook_value(&hook).expect("exec form remem hook");

    assert_eq!(invocation.executable, "/old/remem");
    assert_eq!(invocation.subcommand.as_deref(), Some("observe"));
    assert_eq!(invocation.resolved_host(), Some("claude-code"));
}

#[test]
fn removal_preserves_mixed_third_party_hooks() {
    let mut doc = json!({
        "hooks": {
            "SessionStart": [{
                "matcher": "startup|resume|clear|compact",
                "hooks": [
                    { "command": "/tmp/remem context" },
                    { "command": "/opt/not-remem-helper context" }
                ]
            }],
            "Stop": [{
                "hooks": [{ "command": "/tmp/remem summarize --host claude-code" }]
            }]
        }
    });

    let removed = remove_remem_hooks_for_host(&mut doc, "claude");

    assert_eq!(removed, 2);
    assert_eq!(
        doc["hooks"]["SessionStart"][0]["hooks"][0]["command"],
        "/opt/not-remem-helper context"
    );
    assert!(doc["hooks"].get("Stop").is_none());
}

#[test]
fn removal_preserves_non_array_entries_and_other_host_remem_hooks() {
    let mut doc = json!({
        "hooks": {
            "SessionStart": [
                {"matcher": "custom", "plugin": "third-party"},
                {"matcher": "custom", "hooks": {"command": "third-party"}},
                {"hooks": [
                    { "command": "/tmp/remem context --host codex-cli" },
                    { "command": "/tmp/remem context --host claude-code" }
                ]}
            ]
        }
    });

    let removed = remove_remem_hooks_for_host(&mut doc, "claude");

    assert_eq!(removed, 1);
    let Some(entries) = doc["hooks"]["SessionStart"].as_array() else {
        panic!("SessionStart entries should remain");
    };
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0]["plugin"], "third-party");
    assert!(entries[1]["hooks"].is_object());
    assert_eq!(
        entries[2]["hooks"][0]["command"],
        "/tmp/remem context --host codex-cli"
    );
}

#[test]
fn first_claude_mcp_command_checks_desktop_config_after_primary() -> anyhow::Result<()> {
    let dir = std::env::temp_dir().join(format!(
        "remem-mcp-paths-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir)?;
    let desktop = dir.join("claude_desktop_config.json");
    std::fs::write(
        &desktop,
        r#"{"mcpServers":{"remem":{"command":"/desktop/remem"}}}"#,
    )?;

    let Some(command) = read_first_claude_mcp_command(&[dir.join("missing.json"), desktop])
        .map_err(anyhow::Error::msg)?
    else {
        panic!("desktop MCP command should be found");
    };

    assert_eq!(command, "/desktop/remem");
    std::fs::remove_dir_all(dir)?;
    Ok(())
}

#[test]
fn accepts_sibling_remem_hook_for_slim_commands() {
    let dir = std::env::temp_dir().join(format!(
        "remem-integrity-hook-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let remem = dir.join("remem");
    let hook = dir.join("remem-hook");
    std::fs::write(&remem, []).expect("touch remem");
    std::fs::write(&hook, []).expect("touch remem-hook");
    make_executable(&[&remem, &hook]);
    let remem_s = remem.to_str().expect("utf8");
    let hook_s = hook.to_str().expect("utf8");
    let doc = json!({
        "hooks": {
            "SessionStart": [{
                "hooks": [{
                    "command": format!("{hook_s} context --host codex-cli"),
                    "timeout": 15
                }]
            }],
            "UserPromptSubmit": [{
                "hooks": [{
                    "command": format!("{hook_s} session-init --host codex-cli"),
                    "timeout": 15
                }]
            }],
            "Stop": [{
                "hooks": [{
                    "command": format!("{hook_s} summarize --host codex-cli"),
                    "timeout": 120000
                }]
            }]
        }
    });

    let report = evaluate_hooks(
        &doc,
        "codex",
        PathBuf::from("/tmp/settings.json"),
        remem.as_path(),
        true,
    );
    assert!(
        report.is_healthy(),
        "sibling remem-hook should satisfy slim hook specs: {report:?}"
    );
    let _ = remem_s;
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn expected_executable_prefers_full_remem_when_hooks_mix_binaries() {
    let dir = std::env::temp_dir().join(format!(
        "remem-integrity-mixed-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let remem = dir.join("remem");
    let hook = dir.join("remem-hook");
    std::fs::write(&remem, []).expect("touch remem");
    std::fs::write(&hook, []).expect("touch remem-hook");
    make_executable(&[&remem, &hook]);
    let remem_s = remem.to_str().expect("utf8");
    let hook_s = hook.to_str().expect("utf8");
    let doc = json!({
        "hooks": {
            "SessionStart": [{
                "matcher": "startup|resume|clear|compact",
                "hooks": [{
                    "command": format!("{hook_s} context --host claude-code"),
                    "timeout": 15
                }]
            }],
            "UserPromptSubmit": [{
                "hooks": [{
                    "command": format!("{hook_s} session-init --host claude-code"),
                    "timeout": 15
                }]
            }],
            "PreToolUse": [{
                "matcher": "Bash",
                "hooks": [{
                    "command": format!("{remem_s} rules eval --host claude-code"),
                    "timeout": 5
                }]
            }],
            "PostToolUse": [{
                "matcher": "Write|Edit|NotebookEdit|Bash|Grep|Glob|Agent|Task",
                "hooks": [{
                    "command": format!("{hook_s} observe --host claude-code"),
                    "timeout": 120
                }]
            }],
            "PreCompact": [{
                "hooks": [{
                    "command": format!("{hook_s} summarize --host claude-code"),
                    "timeout": 120
                }]
            }],
            "Stop": [{
                "hooks": [{
                    "command": format!("{hook_s} summarize --host claude-code"),
                    "timeout": 120
                }]
            }]
        }
    });

    assert_eq!(
        expected_hook_executable_from_hooks(&doc, "claude").as_deref(),
        Some(remem_s)
    );
    let report = evaluate_hooks(
        &doc,
        "claude",
        PathBuf::from("/tmp/settings.json"),
        remem.as_path(),
        true,
    );
    assert!(
        report.is_healthy(),
        "mixed remem + remem-hook Claude hooks should be healthy: {report:?}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rules_eval_hook_is_unwanted_when_rule_compilation_is_disabled() {
    let mut doc = json!({
        "hooks": {
            "SessionStart": [{
                "matcher": "startup|resume|clear|compact",
                "hooks": [{ "command": "/tmp/remem context --host claude-code", "timeout": 15 }]
            }],
            "UserPromptSubmit": [{
                "hooks": [{ "command": "/tmp/remem session-init --host claude-code", "timeout": 15 }]
            }],
            "PostToolUse": [{
                "matcher": "Write|Edit|NotebookEdit|Bash|Grep|Glob|Agent|Task",
                "hooks": [{ "command": "/tmp/remem observe --host claude-code", "timeout": 120 }]
            }],
            "PreCompact": [{
                "hooks": [{ "command": "/tmp/remem summarize --host claude-code", "timeout": 120 }]
            }],
            "Stop": [{
                "hooks": [{ "command": "/tmp/remem summarize --host claude-code", "timeout": 120 }]
            }]
        }
    });
    let evaluate = |doc: &Value, rules_eval| {
        evaluate_hooks(
            doc,
            "claude",
            PathBuf::from("/tmp/settings.json"),
            Path::new("/tmp/remem"),
            rules_eval,
        )
    };

    let disabled = evaluate(&doc, false);
    assert!(disabled.is_healthy(), "{disabled:?}");
    assert_eq!((disabled.registered, disabled.expected), (5, 5));
    let enabled = evaluate(&doc, true);
    assert_eq!(enabled.missing_events, vec!["PreToolUse"]);
    assert!(!enabled.is_healthy());

    doc["hooks"]["PreToolUse"] = json!([{
        "matcher": "Bash",
        "hooks": [{ "command": "/tmp/remem rules eval --host claude-code", "timeout": 5 }]
    }]);
    let leftover = evaluate(&doc, false);
    assert!(!leftover.is_healthy());
    assert!(
        leftover
            .warning_block()
            .contains("PreToolUse has remem rules eval hook while rule_compilation.enabled=false"),
        "{}",
        leftover.warning_block()
    );
    assert!(evaluate(&doc, true).is_healthy());
}
