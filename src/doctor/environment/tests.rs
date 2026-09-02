use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn temp_path(label: &str) -> PathBuf {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("remem-{label}-{id}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn probe_hooks_requires_remem_on_each_event() -> anyhow::Result<()> {
    let dir = temp_path("doctor-hooks");
    let hooks_path = dir.join("hooks.json");
    std::fs::write(
        &hooks_path,
        r#"{"hooks":{"SessionStart":[{"matcher":"startup|resume|clear|compact","hooks":[{"command":"/tmp/remem context --host claude-code","timeout":15}]}],"Stop":[{"hooks":[{"command":"other-tool summarize"}]}],"PostToolUse":[{"hooks":[{"command":"other-tool observe"}]}],"UserPromptSubmit":[{"hooks":[{"command":"other-tool init"}]}]}}"#,
    )?;
    let mcp_path = dir.join("claude.json");
    std::fs::write(
        &mcp_path,
        r#"{ "mcpServers": { "remem": { "command": "/tmp/remem" } } }"#,
    )?;

    let check = probe_hooks(HostProbe {
        name: "claude",
        hooks_path,
        mcp_paths: vec![mcp_path],
    });

    assert!(matches!(check.status, Status::Warn));
    assert!(check.detail.contains("1/6 registered"), "{}", check.detail);
    Ok(())
}

#[test]
fn probe_hooks_accepts_codex_strategy() -> anyhow::Result<()> {
    let dir = temp_path("doctor-codex-hooks");
    let hooks_path = dir.join("hooks.json");
    std::fs::write(
        &hooks_path,
        r#"{
  "hooks": {
    "SessionStart": [{ "hooks": [{ "command": "/tmp/remem context --host codex-cli" }] }],
    "UserPromptSubmit": [{ "hooks": [{ "command": "/tmp/remem session-init --host codex-cli" }] }],
    "Stop": [{ "hooks": [{ "command": "/tmp/remem summarize --host codex-cli" }] }]
  }
}"#,
    )?;
    let missing_mcp_check = probe_hooks(HostProbe {
        name: "codex",
        hooks_path: hooks_path.clone(),
        mcp_paths: vec![dir.join("missing.toml")],
    });
    assert!(matches!(missing_mcp_check.status, Status::Ok));

    let mismatched_mcp_path = dir.join("mismatch.toml");
    std::fs::write(
        &mismatched_mcp_path,
        "[mcp_servers.remem]\ncommand = \"/configured/remem\"\n",
    )?;
    let mismatched_mcp_check = probe_hooks(HostProbe {
        name: "codex",
        hooks_path: hooks_path.clone(),
        mcp_paths: vec![mismatched_mcp_path],
    });
    assert!(matches!(mismatched_mcp_check.status, Status::Fail));

    let mcp_path = dir.join("config.toml");
    std::fs::write(&mcp_path, "[mcp_servers.remem]\ncommand = \"/tmp/remem\"\n")?;

    let check = probe_hooks(HostProbe {
        name: "codex",
        hooks_path,
        mcp_paths: vec![mcp_path],
    });

    assert!(matches!(check.status, Status::Ok));
    assert!(check.detail.contains("3/3 registered"), "{}", check.detail);
    Ok(())
}

#[test]
fn probe_hooks_rejects_wrong_remem_subcommands_and_hosts() -> anyhow::Result<()> {
    let cases = [
        (
            "doctor-codex-wrong-subcommands",
            "codex",
            r#"{"hooks":{"SessionStart":[{"hooks":[{"command":"/tmp/remem status --host codex-cli"}]}],"Stop":[{"hooks":[{"command":"/tmp/remem context --host codex-cli"}]}]}}"#,
            "[mcp_servers.remem]\ncommand = \"/tmp/remem\"\n",
            Status::Fail,
            "no remem hooks",
        ),
        (
            "doctor-claude-stale-only-wrong-hosts",
            "claude",
            r#"{"hooks":{"SessionStart":[{"matcher":"startup|resume|clear|compact","hooks":[{"command":"/tmp/remem context --host codex-cli","timeout":15}]}],"UserPromptSubmit":[{"hooks":[{"command":"/tmp/remem session-init --host codex-cli","timeout":15}]}],"PostToolUse":[{"matcher":"Write|Edit|NotebookEdit|Bash|Grep|Glob|Agent|Task","hooks":[{"command":"/tmp/remem observe --host codex-cli","timeout":120}]}],"PreCompact":[{"hooks":[{"command":"/tmp/remem summarize --host codex-cli","timeout":120}]}],"Stop":[{"hooks":[{"command":"/tmp/remem summarize --host claude-code","timeout":120}]}]}}"#,
            r#"{"mcpServers":{"remem":{"command":"/tmp/remem"}}}"#,
            Status::Warn,
            "remem install --target claude --repair",
        ),
    ];

    for (label, host, content, mcp_content, expected_status, expected_detail) in cases {
        let dir = temp_path(label);
        let hooks_path = dir.join("hooks.json");
        std::fs::write(&hooks_path, content)?;
        let mcp_path = dir.join(if host == "codex" {
            "config.toml"
        } else {
            "claude.json"
        });
        std::fs::write(&mcp_path, mcp_content)?;

        let check = probe_hooks(HostProbe {
            name: host,
            hooks_path,
            mcp_paths: vec![mcp_path],
        });

        assert_eq!(check.status, expected_status, "{label}: {}", check.detail);
        assert!(
            check.detail.contains(expected_detail),
            "{label}: {}",
            check.detail
        );
    }
    Ok(())
}

#[test]
fn probe_hooks_warns_on_codex_posttool_observe() -> anyhow::Result<()> {
    let dir = temp_path("doctor-codex-observe-warning");
    let hooks_path = dir.join("hooks.json");
    std::fs::write(
        &hooks_path,
        r#"{
  "hooks": {
    "SessionStart": [{ "hooks": [{ "command": "/tmp/remem context --host codex-cli" }] }],
    "UserPromptSubmit": [{ "hooks": [{ "command": "/tmp/remem session-init --host codex-cli" }] }],
    "PostToolUse": [{ "hooks": [{ "command": "/stale/remem observe --host codex-cli" }] }],
    "Stop": [{ "hooks": [{ "command": "/tmp/remem summarize --host codex-cli" }] }]
  }
}"#,
    )?;
    let mcp_path = dir.join("config.toml");
    std::fs::write(&mcp_path, "[mcp_servers.remem]\ncommand = \"/tmp/remem\"\n")?;

    let check = probe_hooks(HostProbe {
        name: "codex",
        hooks_path,
        mcp_paths: vec![mcp_path],
    });

    assert!(matches!(check.status, Status::Warn));
    assert!(
        check.detail.contains("PostToolUse observe"),
        "{}",
        check.detail
    );
    Ok(())
}

#[test]
fn probe_mcp_requires_exact_codex_remem_entry() {
    let dir = temp_path("doctor-mcp");
    let mcp_path = dir.join("config.toml");
    std::fs::write(
        &mcp_path,
        r#"# remem should not be detected from comments
[mcp_servers.other]
command = "echo"
note = "remem"
"#,
    )
    .unwrap();

    let check = probe_mcp(HostProbe {
        name: "codex",
        hooks_path: dir.join("hooks.json"),
        mcp_paths: vec![mcp_path],
    });

    assert!(matches!(check.status, Status::Fail));
    assert!(check.detail.contains("not registered"), "{}", check.detail);
}

#[test]
fn configured_paths_read_codex_hooks_and_mcp_command() {
    let dir = temp_path("doctor-configured-codex-paths");
    let hooks_path = dir.join("hooks.json");
    let mcp_path = dir.join("config.toml");
    std::fs::write(
        &hooks_path,
        r#"{
  "hooks": {
    "SessionStart": [{ "hooks": [{ "command": "REMEM_CONTEXT_HOST=codex-cli /hooks/bin/remem context --color" }] }],
    "Stop": [{ "hooks": [{ "command": "REMEM_SUMMARY_EXECUTOR=codex-cli /hooks/bin/remem summarize" }] }]
  }
}"#,
    )
    .unwrap();
    std::fs::write(
        &mcp_path,
        r#"[mcp_servers.remem]
command = "/mcp/bin/remem"
"#,
    )
    .unwrap();

    let paths = configured_remem_paths_for(vec![HostProbe {
        name: "codex",
        hooks_path,
        mcp_paths: vec![mcp_path],
    }]);

    assert!(paths.contains(&PathBuf::from("/hooks/bin/remem")));
    assert!(paths.contains(&PathBuf::from("/mcp/bin/remem")));
}

#[test]
fn configured_paths_read_claude_mcp_command() {
    let dir = temp_path("doctor-configured-claude-paths");
    let mcp_path = dir.join("claude.json");
    std::fs::write(
        &mcp_path,
        r#"{ "mcpServers": { "remem": { "command": "/claude/bin/remem" } } }"#,
    )
    .unwrap();

    let paths = configured_remem_paths_for(vec![HostProbe {
        name: "claude",
        hooks_path: dir.join("settings.json"),
        mcp_paths: vec![mcp_path],
    }]);

    assert_eq!(paths, vec![PathBuf::from("/claude/bin/remem")]);
}

#[test]
fn extract_remem_command_path_ignores_env_assignments() {
    assert_eq!(
        extract_remem_command_path(
            "REMEM_CONTEXT_HOST=codex-cli '/opt/remem/bin/remem' context --color"
        ),
        Some("/opt/remem/bin/remem".to_string())
    );
    assert_eq!(extract_remem_command_path("NOTE=remem echo ok"), None);
}

#[test]
fn active_hosts_keeps_all_present_hosts() {
    let claude_dir = temp_path("doctor-claude");
    let claude = HostProbe {
        name: "claude",
        hooks_path: claude_dir.join("settings.json"),
        mcp_paths: vec![claude_dir.join("claude.json")],
    };
    std::fs::write(&claude.mcp_paths[0], r#"{ "mcpServers": { "other": {} } }"#).unwrap();

    let codex_dir = temp_path("doctor-codex");
    let codex = HostProbe {
        name: "codex",
        hooks_path: codex_dir.join("hooks.json"),
        mcp_paths: vec![codex_dir.join("config.toml")],
    };
    std::fs::write(
        &codex.mcp_paths[0],
        r#"[mcp_servers.remem]
command = "/tmp/remem"
"#,
    )
    .unwrap();

    let expected = vec![claude.clone(), codex.clone()];
    let hosts: Vec<_> = expected.clone().into_iter().filter(host_present).collect();

    assert_eq!(hosts, expected);
}

#[test]
fn doctor_reports_each_present_host_even_if_only_one_targets_remem() {
    let claude_dir = temp_path("doctor-home-claude");
    let codex_dir = temp_path("doctor-home-codex");

    std::fs::write(
        codex_dir.join("hooks.json"),
        r#"{
  "hooks": {
    "SessionStart": [{ "hooks": [{ "command": "/tmp/remem context --host codex-cli" }] }],
    "UserPromptSubmit": [{ "hooks": [{ "command": "/tmp/remem session-init --host codex-cli" }] }],
    "Stop": [{ "hooks": [{ "command": "/tmp/remem summarize --host codex-cli" }] }]
  }
}"#,
    )
    .unwrap();
    std::fs::write(
        codex_dir.join("config.toml"),
        r#"[mcp_servers.remem]
command = "/tmp/remem"
"#,
    )
    .unwrap();
    std::fs::write(
        claude_dir.join("claude.json"),
        r#"{ "mcpServers": { "other": {} } }"#,
    )
    .unwrap();

    let hosts = vec![
        HostProbe {
            name: "claude",
            hooks_path: claude_dir.join("settings.json"),
            mcp_paths: vec![claude_dir.join("claude.json")],
        },
        HostProbe {
            name: "codex",
            hooks_path: codex_dir.join("hooks.json"),
            mcp_paths: vec![codex_dir.join("config.toml")],
        },
    ];

    let hook_checks = check_hooks_for(hosts.clone());
    assert_eq!(hook_checks.len(), 2);
    assert_eq!(hook_checks[0].name, "Hooks (claude)");
    assert!(matches!(hook_checks[0].status, Status::Fail));
    assert_eq!(hook_checks[1].name, "Hooks (codex)");
    assert!(matches!(hook_checks[1].status, Status::Ok));

    let mcp_checks = check_mcp_for(hosts);
    assert_eq!(mcp_checks.len(), 2);
    assert_eq!(mcp_checks[0].name, "MCP (claude)");
    assert!(matches!(mcp_checks[0].status, Status::Fail));
    assert_eq!(mcp_checks[1].name, "MCP (codex)");
    assert!(matches!(mcp_checks[1].status, Status::Ok));
}

#[test]
fn probe_mcp_accepts_claude_desktop_config_path() {
    let dir = temp_path("doctor-claude-desktop");
    std::fs::write(
        dir.join("claude_desktop_config.json"),
        r#"{ "mcpServers": { "remem": { "command": "/tmp/remem" } } }"#,
    )
    .unwrap();

    let check = probe_mcp(HostProbe {
        name: "claude",
        hooks_path: dir.join("settings.json"),
        mcp_paths: vec![
            dir.join("claude.json"),
            dir.join("claude_desktop_config.json"),
        ],
    });

    assert!(matches!(check.status, Status::Ok));
    assert!(
        check.detail.contains("claude_desktop_config.json"),
        "{}",
        check.detail
    );
}

#[test]
fn probe_hooks_accepts_sibling_remem_hook_for_slim_commands() -> anyhow::Result<()> {
    let dir = temp_path("doctor-sibling-hook");
    let remem = dir.join("remem");
    let hook = dir.join("remem-hook");
    std::fs::write(&remem, [])?;
    std::fs::write(&hook, [])?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&hook)?.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&hook, permissions)?;
    }
    let remem_s = remem.to_str().expect("utf8");
    let hook_s = hook.to_str().expect("utf8");
    let hooks_path = dir.join("settings.json");
    std::fs::write(
        &hooks_path,
        format!(
            r#"{{
  "hooks": {{
    "SessionStart": [{{
      "matcher": "startup|resume|clear|compact",
      "hooks": [{{ "command": "{hook_s} context --host claude-code", "timeout": 15 }}]
    }}],
    "UserPromptSubmit": [{{
      "hooks": [{{ "command": "{hook_s} session-init --host claude-code", "timeout": 15 }}]
    }}],
    "PreToolUse": [{{
      "matcher": "Bash",
      "hooks": [{{ "command": "{remem_s} rules eval --host claude-code", "timeout": 5 }}]
    }}],
    "PostToolUse": [{{
      "matcher": "Write|Edit|NotebookEdit|Bash|Grep|Glob|Agent|Task",
      "hooks": [{{ "command": "{hook_s} observe --host claude-code", "timeout": 120 }}]
    }}],
    "PreCompact": [{{
      "hooks": [{{ "command": "{hook_s} summarize --host claude-code", "timeout": 120 }}]
    }}],
    "Stop": [{{
      "hooks": [{{ "command": "{hook_s} summarize --host claude-code", "timeout": 120 }}]
    }}]
  }}
}}"#
        ),
    )?;
    let mcp_path = dir.join("claude.json");
    std::fs::write(
        &mcp_path,
        format!(r#"{{ "mcpServers": {{ "remem": {{ "command": "{remem_s}" }} }} }}"#),
    )?;

    let check = probe_hooks(HostProbe {
        name: "claude",
        hooks_path,
        mcp_paths: vec![mcp_path],
    });
    assert!(
        matches!(check.status, Status::Ok),
        "sibling remem-hook should be healthy: {}",
        check.detail
    );
    assert!(check.detail.contains("6/6 registered"), "{}", check.detail);
    Ok(())
}

#[test]
fn configured_paths_map_sibling_remem_hook_to_full_binary() {
    let dir = temp_path("doctor-configured-sibling-hook");
    let remem = dir.join("remem");
    let hook = dir.join("remem-hook");
    std::fs::write(&remem, []).unwrap();
    std::fs::write(&hook, []).unwrap();
    let remem_s = remem.to_str().expect("utf8");
    let hook_s = hook.to_str().expect("utf8");
    let hooks_path = dir.join("settings.json");
    std::fs::write(
        &hooks_path,
        format!(
            r#"{{
  "hooks": {{
    "SessionStart": [{{ "hooks": [{{ "command": "{hook_s} context --host claude-code" }}] }}],
    "PreToolUse": [{{ "hooks": [{{ "command": "{remem_s} rules eval --host claude-code" }}] }}]
  }}
}}"#
        ),
    )
    .unwrap();
    let mcp_path = dir.join("claude.json");
    std::fs::write(
        &mcp_path,
        format!(r#"{{ "mcpServers": {{ "remem": {{ "command": "{remem_s}" }} }} }}"#),
    )
    .unwrap();

    let paths = configured_remem_paths_for(vec![HostProbe {
        name: "claude",
        hooks_path,
        mcp_paths: vec![mcp_path],
    }]);

    assert_eq!(paths, vec![remem]);
    assert!(!paths.iter().any(|path| path.ends_with("remem-hook")));
}

#[test]
fn configured_paths_map_slim_only_hooks_to_sibling_full_binary() {
    let dir = temp_path("doctor-configured-slim-only-hook");
    let remem = dir.join("remem");
    let hook = dir.join("remem-hook");
    let hook_s = hook.to_str().expect("utf8");
    let hooks_path = dir.join("hooks.json");
    std::fs::write(
        &hooks_path,
        format!(
            r#"{{
  "hooks": {{
    "SessionStart": [{{ "hooks": [{{ "command": "{hook_s} context --host codex-cli" }}] }}],
    "Stop": [{{ "hooks": [{{ "command": "{hook_s} summarize --host codex-cli" }}] }}]
  }}
}}"#
        ),
    )
    .unwrap();

    let paths = configured_remem_paths_for(vec![HostProbe {
        name: "codex",
        hooks_path,
        mcp_paths: Vec::new(),
    }]);

    assert_eq!(paths, vec![remem]);
}
