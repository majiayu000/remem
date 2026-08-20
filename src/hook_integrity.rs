use serde_json::Value;
use std::path::{Path, PathBuf};

mod parse;
use parse::{parse_remem_hook_value, parse_remem_invocation};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ExpectedHookSpec {
    pub(crate) event: &'static str,
    pub(crate) subcommand: &'static str,
    pub(crate) nested_subcommand: Option<&'static str>,
    pub(crate) host: &'static str,
    pub(crate) matcher: Option<&'static str>,
    pub(crate) timeout_seconds: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HookIntegrityReport {
    pub(crate) host: &'static str,
    pub(crate) expected: usize,
    pub(crate) registered: usize,
    pub(crate) path: PathBuf,
    pub(crate) missing_events: Vec<&'static str>,
    pub(crate) stale_details: Vec<String>,
}

impl HookIntegrityReport {
    pub(crate) fn is_healthy(&self) -> bool {
        self.registered == self.expected && self.stale_details.is_empty()
    }

    pub(crate) fn warning_block(&self) -> String {
        let mut output = String::new();
        output.push_str("## Hook Integrity Warning\n");
        output.push_str(&format!(
            "- Hooks ({}) stale or incomplete: {}/{} registered in {}.\n",
            self.host,
            self.registered,
            self.expected,
            self.path.display()
        ));
        if !self.missing_events.is_empty() {
            output.push_str(&format!(
                "- Missing or stale events: {}.\n",
                self.missing_events.join(", ")
            ));
        }
        if let Some(detail) = self.stale_details.first() {
            output.push_str(&format!("- Detail: {detail}.\n"));
        }
        output.push_str(&format!(
            "- Repair: remem install --target {} --repair\n",
            self.host
        ));
        output
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RememInvocation {
    pub(crate) executable: String,
    pub(crate) subcommand: Option<String>,
    pub(crate) nested_subcommand: Option<String>,
    pub(crate) host: Option<String>,
    pub(crate) env_host: Option<String>,
}

impl RememInvocation {
    pub(crate) fn resolved_host(&self) -> Option<&str> {
        self.host.as_deref().or(self.env_host.as_deref())
    }
}

const CLAUDE_EXPECTED: &[ExpectedHookSpec] = &[
    ExpectedHookSpec {
        event: "PostToolUse",
        subcommand: "observe",
        nested_subcommand: None,
        host: "claude-code",
        matcher: Some("Write|Edit|NotebookEdit|Bash|Grep|Glob|Agent|Task"),
        timeout_seconds: Some(120),
    },
    ExpectedHookSpec {
        event: "PreCompact",
        subcommand: "summarize",
        nested_subcommand: None,
        host: "claude-code",
        matcher: None,
        timeout_seconds: Some(120),
    },
    ExpectedHookSpec {
        event: "Stop",
        subcommand: "summarize",
        nested_subcommand: None,
        host: "claude-code",
        matcher: None,
        timeout_seconds: Some(120),
    },
    ExpectedHookSpec {
        event: "SessionStart",
        subcommand: "context",
        nested_subcommand: None,
        host: "claude-code",
        matcher: Some("startup|resume|clear|compact"),
        timeout_seconds: Some(15),
    },
    ExpectedHookSpec {
        event: "UserPromptSubmit",
        subcommand: "session-init",
        nested_subcommand: None,
        host: "claude-code",
        matcher: None,
        timeout_seconds: Some(15),
    },
    ExpectedHookSpec {
        event: "PreToolUse",
        subcommand: "rules",
        nested_subcommand: Some("eval"),
        host: "claude-code",
        matcher: Some("Bash"),
        timeout_seconds: Some(5),
    },
];

const CODEX_EXPECTED: &[ExpectedHookSpec] = &[
    ExpectedHookSpec {
        event: "SessionStart",
        subcommand: "context",
        nested_subcommand: None,
        host: "codex-cli",
        matcher: None,
        timeout_seconds: None,
    },
    ExpectedHookSpec {
        event: "Stop",
        subcommand: "summarize",
        nested_subcommand: None,
        host: "codex-cli",
        matcher: None,
        timeout_seconds: None,
    },
];

pub(crate) fn expected_specs(host: &str) -> &'static [ExpectedHookSpec] {
    match host {
        "codex" => CODEX_EXPECTED,
        _ => CLAUDE_EXPECTED,
    }
}

pub(crate) fn expected_hook_events(host: &str) -> Vec<&'static str> {
    expected_specs(host).iter().map(|spec| spec.event).collect()
}

pub(crate) fn runtime_host(host: &str) -> &'static str {
    match host {
        "codex" => "codex-cli",
        _ => "claude-code",
    }
}

pub(crate) fn expected_hook_executable_from_hooks(doc: &Value, host: &str) -> Option<String> {
    let mut paths = Vec::new();
    for spec in expected_specs(host) {
        for (_entry, hook) in hook_values_for_event(doc, spec.event) {
            let Some(invocation) = parse_remem_hook_value(hook) else {
                continue;
            };
            if invocation_matches_spec_command(&invocation, spec)
                && invocation
                    .resolved_host()
                    .is_none_or(|resolved| resolved == runtime_host(host))
                && !paths.contains(&invocation.executable)
            {
                paths.push(invocation.executable);
            }
        }
    }
    crate::hook_cli::preferred_expected_hook_executable(&paths)
}

pub(crate) fn event_has_remem_subcommand_hook(doc: &Value, event: &str, subcommand: &str) -> bool {
    hook_values_for_event(doc, event).any(|(_entry, hook)| {
        parse_remem_hook_value(hook)
            .is_some_and(|invocation| invocation.subcommand.as_deref() == Some(subcommand))
    })
}

pub(crate) fn hook_command_strings(doc: &Value) -> impl Iterator<Item = &str> {
    doc.get("hooks")
        .and_then(|hooks| hooks.as_object())
        .into_iter()
        .flat_map(|hooks| hooks.values())
        .filter_map(|entries| entries.as_array())
        .flatten()
        .filter_map(|entry| entry.get("hooks").and_then(|hooks| hooks.as_array()))
        .flatten()
        .filter_map(|hook| hook.get("command").and_then(|command| command.as_str()))
}

pub(crate) fn extract_remem_command_path(command: &str) -> Option<String> {
    parse_remem_invocation(command).map(|invocation| invocation.executable)
}

pub(crate) fn evaluate_hooks(
    doc: &Value,
    host: &'static str,
    path: PathBuf,
    expected_executable: &Path,
) -> HookIntegrityReport {
    let specs = expected_specs(host);
    let mut registered = 0;
    let mut missing_events = Vec::new();
    let mut stale_details = Vec::new();

    for spec in specs {
        let matches = expected_match_count(doc, spec, expected_executable);
        if matches > 0 {
            registered += 1;
            if matches > 1 {
                stale_details.push(format!(
                    "{} has {matches} duplicate fresh remem {} hooks",
                    spec.event, spec.subcommand
                ));
            }
        } else {
            missing_events.push(spec.event);
        }
        collect_stale_details(doc, spec, expected_executable, &mut stale_details);
    }
    stale_details.sort();
    stale_details.dedup();

    HookIntegrityReport {
        host,
        expected: specs.len(),
        registered,
        path,
        missing_events,
        stale_details,
    }
}

pub(crate) fn failed_report(
    host: &'static str,
    path: PathBuf,
    detail: impl Into<String>,
) -> HookIntegrityReport {
    HookIntegrityReport {
        host,
        expected: expected_specs(host).len(),
        registered: 0,
        path,
        missing_events: expected_hook_events(host),
        stale_details: vec![detail.into()],
    }
}

pub(crate) fn remove_remem_hooks_for_host(settings: &mut Value, host: &str) -> usize {
    let mut removed = 0;
    let Some(hooks) = settings
        .get_mut("hooks")
        .and_then(|hooks| hooks.as_object_mut())
    else {
        return removed;
    };

    let expected_events = expected_specs(host)
        .iter()
        .map(|spec| spec.event)
        .collect::<Vec<_>>();
    for event in expected_events {
        let Some(entries) = hooks
            .get_mut(event)
            .and_then(|entries| entries.as_array_mut())
        else {
            continue;
        };
        let mut retained_entries = Vec::new();
        for mut entry in std::mem::take(entries) {
            let Some(inner_hooks) = entry
                .get_mut("hooks")
                .and_then(|hooks| hooks.as_array_mut())
            else {
                retained_entries.push(entry);
                continue;
            };
            let before = inner_hooks.len();
            inner_hooks.retain(|hook| !is_remem_owned_for_event(host, event, hook));
            let removed_from_entry = before.saturating_sub(inner_hooks.len());
            removed += removed_from_entry;
            if !inner_hooks.is_empty() || removed_from_entry == 0 {
                retained_entries.push(entry);
            }
        }
        *entries = retained_entries;
    }

    let empty_events = hooks
        .iter()
        .filter(|(_event, entries)| entries.as_array().is_some_and(|entries| entries.is_empty()))
        .map(|(event, _entries)| event.clone())
        .collect::<Vec<_>>();
    for event in empty_events {
        hooks.remove(&event);
    }
    if hooks.is_empty() {
        if let Some(obj) = settings.as_object_mut() {
            obj.remove("hooks");
        }
    }
    removed
}

pub(crate) fn read_claude_mcp_command(path: &Path) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)
        .map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    let doc: Value = serde_json::from_str(&content)
        .map_err(|err| format!("cannot parse {}: {err}", path.display()))?;
    Ok(doc
        .get("mcpServers")
        .and_then(|servers| servers.get("remem"))
        .and_then(|server| server.get("command"))
        .and_then(|command| command.as_str())
        .map(ToString::to_string))
}

pub(crate) fn read_first_claude_mcp_command(paths: &[PathBuf]) -> Result<Option<String>, String> {
    for path in paths {
        match read_claude_mcp_command(path) {
            Ok(Some(command)) => return Ok(Some(command)),
            Ok(None) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

fn expected_match_count(doc: &Value, spec: &ExpectedHookSpec, executable: &Path) -> usize {
    hook_values_for_event(doc, spec.event)
        .filter(|(entry, hook)| hook_matches_expected(entry, hook, spec, executable))
        .count()
}

fn collect_stale_details(
    doc: &Value,
    spec: &ExpectedHookSpec,
    executable: &Path,
    stale_details: &mut Vec<String>,
) {
    for (entry, hook) in hook_values_for_event(doc, spec.event) {
        let Some(invocation) = parse_remem_hook_value(hook) else {
            continue;
        };
        if !invocation_matches_spec_command(&invocation, spec) {
            continue;
        }
        if hook_matches_expected(entry, hook, spec, executable) {
            continue;
        }
        stale_details.push(format!(
            "{} has stale remem {} hook ({})",
            spec.event,
            spec.subcommand,
            stale_reason(entry, hook, spec, executable, &invocation)
        ));
    }
}

fn stale_reason(
    entry: &Value,
    hook: &Value,
    spec: &ExpectedHookSpec,
    executable: &Path,
    invocation: &RememInvocation,
) -> String {
    let mut reasons = Vec::new();
    if !crate::hook_cli::hook_executable_is_allowed(
        Path::new(&invocation.executable),
        executable,
        spec.subcommand,
    ) {
        reasons.push(format!("executable {}", invocation.executable));
    }
    if invocation.resolved_host() != Some(spec.host) {
        reasons.push(format!(
            "host {}",
            invocation.resolved_host().unwrap_or("missing")
        ));
    }
    if invocation.nested_subcommand.as_deref() != spec.nested_subcommand {
        reasons.push("nested subcommand drift".to_string());
    }
    if !entry_matcher_matches(entry, spec.matcher) {
        reasons.push("matcher drift".to_string());
    }
    if !hook_timeout_matches(hook, spec.timeout_seconds) {
        reasons.push("timeout drift".to_string());
    }
    if reasons.is_empty() {
        "shape drift".to_string()
    } else {
        reasons.join(", ")
    }
}

fn hook_matches_expected(
    entry: &Value,
    hook: &Value,
    spec: &ExpectedHookSpec,
    executable: &Path,
) -> bool {
    entry_matcher_matches(entry, spec.matcher)
        && hook_timeout_matches(hook, spec.timeout_seconds)
        && parse_remem_hook_value(hook).is_some_and(|invocation| {
            crate::hook_cli::hook_executable_is_allowed(
                Path::new(&invocation.executable),
                executable,
                spec.subcommand,
            ) && invocation_matches_spec_command(&invocation, spec)
                && invocation.resolved_host() == Some(spec.host)
        })
}

fn entry_matcher_matches(entry: &Value, expected: Option<&str>) -> bool {
    match expected {
        Some(expected) => {
            entry.get("matcher").and_then(|matcher| matcher.as_str()) == Some(expected)
        }
        None => entry.get("matcher").is_none(),
    }
}

fn hook_timeout_matches(hook: &Value, expected: Option<i64>) -> bool {
    expected.is_none_or(|expected| {
        hook.get("timeout").and_then(|timeout| timeout.as_i64()) == Some(expected)
    })
}

fn is_remem_owned_for_event(host: &str, event: &str, hook: &Value) -> bool {
    let Some(spec) = expected_specs(host).iter().find(|spec| spec.event == event) else {
        return false;
    };
    parse_remem_hook_value(hook).is_some_and(|invocation| {
        invocation_matches_spec_command(&invocation, spec)
            && invocation
                .resolved_host()
                .is_none_or(|resolved| resolved == runtime_host(host))
    })
}

fn invocation_matches_spec_command(invocation: &RememInvocation, spec: &ExpectedHookSpec) -> bool {
    invocation.subcommand.as_deref() == Some(spec.subcommand)
        && invocation.nested_subcommand.as_deref() == spec.nested_subcommand
}

fn hook_values_for_event<'a>(
    doc: &'a Value,
    event: &str,
) -> impl Iterator<Item = (&'a Value, &'a Value)> {
    doc.get("hooks")
        .and_then(|hooks| hooks.get(event))
        .and_then(|entries| entries.as_array())
        .into_iter()
        .flatten()
        .flat_map(|entry| {
            entry
                .get("hooks")
                .and_then(|hooks| hooks.as_array())
                .into_iter()
                .flatten()
                .map(move |hook| (entry, hook))
        })
}

#[cfg(test)]
mod tests {
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
                        "timeout": 15000
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
        );
        assert!(
            report.is_healthy(),
            "mixed remem + remem-hook Claude hooks should be healthy: {report:?}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
