use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ExpectedHookSpec {
    pub(crate) event: &'static str,
    pub(crate) subcommand: &'static str,
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
        host: "claude-code",
        matcher: Some("Write|Edit|NotebookEdit|Bash|Grep|Glob|Agent|Task"),
        timeout_seconds: Some(120),
    },
    ExpectedHookSpec {
        event: "PreCompact",
        subcommand: "summarize",
        host: "claude-code",
        matcher: None,
        timeout_seconds: Some(120),
    },
    ExpectedHookSpec {
        event: "Stop",
        subcommand: "summarize",
        host: "claude-code",
        matcher: None,
        timeout_seconds: Some(120),
    },
    ExpectedHookSpec {
        event: "SessionStart",
        subcommand: "context",
        host: "claude-code",
        matcher: Some("startup|resume|clear|compact"),
        timeout_seconds: Some(15),
    },
    ExpectedHookSpec {
        event: "UserPromptSubmit",
        subcommand: "session-init",
        host: "claude-code",
        matcher: None,
        timeout_seconds: Some(15),
    },
];

const CODEX_EXPECTED: &[ExpectedHookSpec] = &[
    ExpectedHookSpec {
        event: "SessionStart",
        subcommand: "context",
        host: "codex-cli",
        matcher: None,
        timeout_seconds: None,
    },
    ExpectedHookSpec {
        event: "Stop",
        subcommand: "summarize",
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
            if invocation.subcommand.as_deref() == Some(spec.subcommand)
                && invocation
                    .resolved_host()
                    .is_none_or(|resolved| resolved == runtime_host(host))
                && !paths.contains(&invocation.executable)
            {
                paths.push(invocation.executable);
            }
        }
    }
    match paths.as_slice() {
        [path] => Some(path.clone()),
        _ => None,
    }
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
        if invocation.subcommand.as_deref() != Some(spec.subcommand) {
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
    if Path::new(&invocation.executable) != executable {
        reasons.push(format!("executable {}", invocation.executable));
    }
    if invocation.resolved_host() != Some(spec.host) {
        reasons.push(format!(
            "host {}",
            invocation.resolved_host().unwrap_or("missing")
        ));
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
            Path::new(&invocation.executable) == executable
                && invocation.subcommand.as_deref() == Some(spec.subcommand)
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
        invocation.subcommand.as_deref() == Some(spec.subcommand)
            && invocation
                .resolved_host()
                .is_none_or(|resolved| resolved == runtime_host(host))
    })
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

pub(crate) fn parse_remem_hook_value(hook: &Value) -> Option<RememInvocation> {
    let command = hook.get("command").and_then(|command| command.as_str())?;
    if let Some(args) = hook.get("args").and_then(|args| args.as_array()) {
        let mut tokens = vec![command.to_string()];
        for arg in args {
            tokens.push(arg.as_str()?.to_string());
        }
        return parse_remem_tokens(tokens);
    }
    parse_remem_invocation(command)
}

pub(crate) fn parse_remem_invocation(command: &str) -> Option<RememInvocation> {
    parse_remem_tokens(shell_words(command)?)
}

fn parse_remem_tokens(tokens: Vec<String>) -> Option<RememInvocation> {
    let command_index = tokens.iter().position(|token| !is_env_assignment(token))?;
    if !is_remem_command_token(&tokens[command_index]) {
        return None;
    }
    let host = find_host_arg(&tokens[command_index + 1..]);

    Some(RememInvocation {
        executable: tokens[command_index].clone(),
        subcommand: tokens.get(command_index + 1).cloned(),
        host,
        env_host: find_legacy_host_env(&tokens[..command_index]),
    })
}

fn find_host_arg(tokens: &[String]) -> Option<String> {
    let mut iter = tokens.iter();
    while let Some(token) = iter.next() {
        if token == "--host" {
            return iter
                .next()
                .map(|host| crate::runtime_config::normalize_host(host));
        }
        if let Some(host) = token.strip_prefix("--host=") {
            return Some(crate::runtime_config::normalize_host(host));
        }
    }
    None
}

fn find_legacy_host_env(tokens: &[String]) -> Option<String> {
    for token in tokens {
        let Some((name, value)) = env_assignment(token) else {
            continue;
        };
        if matches!(name, "REMEM_HOOK_HOST" | "REMEM_CONTEXT_HOST") {
            return Some(crate::runtime_config::normalize_host(value));
        }
    }
    for token in tokens {
        let Some((name, value)) = env_assignment(token) else {
            continue;
        };
        if matches!(name, "REMEM_SUMMARY_EXECUTOR" | "REMEM_EXECUTOR") {
            match value.trim().to_ascii_lowercase().as_str() {
                "codex" | "codex-cli" => {
                    return Some(crate::runtime_config::CODEX_HOST.to_string())
                }
                "claude" | "claude-cli" | "cli" => {
                    return Some(crate::runtime_config::CLAUDE_HOST.to_string());
                }
                _ => {}
            }
        }
    }
    None
}

fn is_remem_command_token(token: &str) -> bool {
    Path::new(token)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem == "remem")
}

fn is_env_assignment(token: &str) -> bool {
    env_assignment(token).is_some()
}

fn env_assignment(token: &str) -> Option<(&str, &str)> {
    let (name, value) = token.split_once('=')?;
    let mut chars = name.chars();
    let first = chars.next()?;
    if (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        Some((name, value))
    } else {
        None
    }
}

fn shell_words(command: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = command.chars().peekable();
    let mut quote = Quote::None;
    let mut in_token = false;

    while let Some(ch) = chars.next() {
        match quote {
            Quote::None => match ch {
                '\'' => {
                    quote = Quote::Single;
                    in_token = true;
                }
                '"' => {
                    quote = Quote::Double;
                    in_token = true;
                }
                '\\' => {
                    in_token = true;
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                }
                ch if ch.is_whitespace() => {
                    if in_token {
                        tokens.push(std::mem::take(&mut current));
                        in_token = false;
                    }
                }
                _ => {
                    current.push(ch);
                    in_token = true;
                }
            },
            Quote::Single => {
                if ch == '\'' {
                    quote = Quote::None;
                } else {
                    current.push(ch);
                }
            }
            Quote::Double => match ch {
                '"' => quote = Quote::None,
                '\\' => {
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                }
                _ => current.push(ch),
            },
        }
    }

    if quote != Quote::None {
        return None;
    }
    if in_token {
        tokens.push(current);
    }
    Some(tokens)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Quote {
    None,
    Single,
    Double,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detects_missing_claude_hooks_as_three_of_five() {
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
        assert_eq!(report.expected, 5);
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
}
