use std::path::{Path, PathBuf};

use serde_json::Value;

use super::host::HostKind;
use super::invocation::ContextInvocation;

pub(super) fn claude_hook_integrity_warning(invocation: &ContextInvocation) -> Option<String> {
    if invocation.host != HostKind::ClaudeCode || !is_claude_session_start_source(invocation) {
        return None;
    }

    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let settings_path = home.join(".claude").join("settings.json");
    let claude_mcp_paths = crate::install::claude_mcp_paths();
    let fallback_executable = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("remem"));
    claude_hook_integrity_warning_from_paths(
        invocation,
        &settings_path,
        &claude_mcp_paths,
        fallback_executable,
    )
}

fn claude_hook_integrity_warning_from_paths(
    invocation: &ContextInvocation,
    settings_path: &Path,
    claude_mcp_paths: &[PathBuf],
    fallback_executable: PathBuf,
) -> Option<String> {
    if invocation.host != HostKind::ClaudeCode || !is_claude_session_start_source(invocation) {
        return None;
    }

    let doc = match std::fs::read_to_string(settings_path) {
        Ok(content) => match serde_json::from_str::<Value>(&content) {
            Ok(doc) => doc,
            Err(error) => {
                return Some(
                    crate::hook_integrity::failed_report(
                        "claude",
                        settings_path.to_path_buf(),
                        format!("cannot parse Claude hooks: {error}"),
                    )
                    .warning_block(),
                )
            }
        },
        Err(error) => {
            return Some(
                crate::hook_integrity::failed_report(
                    "claude",
                    settings_path.to_path_buf(),
                    format!("cannot read Claude hooks: {error}"),
                )
                .warning_block(),
            )
        }
    };

    let expected_executable =
        crate::hook_integrity::read_first_claude_mcp_command(claude_mcp_paths)
            .ok()
            .flatten()
            .map(PathBuf::from)
            .or_else(|| {
                crate::hook_integrity::expected_hook_executable_from_hooks(&doc, "claude")
                    .map(PathBuf::from)
            })
            .unwrap_or(fallback_executable);
    let report = crate::hook_integrity::evaluate_hooks(
        &doc,
        "claude",
        settings_path.to_path_buf(),
        &expected_executable,
    );
    (!report.is_healthy()).then(|| report.warning_block())
}

pub(super) fn append_hook_integrity_warning(output: &mut String, warning: Option<&str>) {
    let Some(warning) = warning else {
        return;
    };
    if !output.is_empty() && !output.ends_with('\n') {
        output.push('\n');
    }
    if !output.is_empty() {
        output.push('\n');
    }
    output.push_str(warning);
}

fn is_claude_session_start_source(invocation: &ContextInvocation) -> bool {
    invocation.source.as_deref().is_none_or(|source| {
        matches!(
            source.trim().to_ascii_lowercase().as_str(),
            "" | "startup" | "resume" | "clear" | "compact"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warning_appends_to_suppressed_output() {
        let mut output = String::new();

        append_hook_integrity_warning(&mut output, Some("## Hook Integrity Warning\n- Repair\n"));

        assert!(output.contains("Hook Integrity Warning"));
    }

    #[test]
    fn codex_invocation_is_not_claude_session_start_source() {
        let invocation = ContextInvocation {
            cwd: ".".to_string(),
            project: ".".to_string(),
            session_id: None,
            transcript_path: None,
            source: Some("startup".to_string()),
            host: HostKind::CodexCli,
            use_colors: false,
            debug: false,
            force: false,
            gate_mode: None,
        };

        assert!(claude_hook_integrity_warning(&invocation).is_none());
    }

    #[test]
    fn claude_three_of_six_warning_names_repair_command() -> anyhow::Result<()> {
        let dir = std::env::temp_dir().join(format!(
            "remem-hook-warning-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir)?;
        let settings_path = dir.join("settings.json");
        let mcp_paths = vec![dir.join("claude.json")];
        std::fs::write(
            &settings_path,
            r#"{"hooks":{"SessionStart":[{"matcher":"startup|resume|clear|compact","hooks":[{"command":"/tmp/remem context --host claude-code","timeout":15}]}],"UserPromptSubmit":[{"hooks":[{"command":"/tmp/remem session-init --host claude-code","timeout":15}]}],"PreCompact":[{"hooks":[{"command":"/tmp/remem summarize --host claude-code","timeout":120}]}]}}"#,
        )?;
        let invocation = ContextInvocation {
            cwd: ".".to_string(),
            project: ".".to_string(),
            session_id: None,
            transcript_path: None,
            source: Some("startup".to_string()),
            host: HostKind::ClaudeCode,
            use_colors: false,
            debug: false,
            force: false,
            gate_mode: None,
        };

        let warning = claude_hook_integrity_warning_from_paths(
            &invocation,
            &settings_path,
            &mcp_paths,
            PathBuf::from("/tmp/remem"),
        )
        .expect("incomplete Claude hooks should warn");

        assert!(warning.contains("3/6 registered"), "{warning}");
        assert!(
            warning.contains("remem install --target claude --repair"),
            "{warning}"
        );
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn warning_uses_desktop_mcp_path_when_primary_is_missing() -> anyhow::Result<()> {
        let dir = std::env::temp_dir().join(format!(
            "remem-hook-warning-desktop-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir)?;
        let settings_path = dir.join("settings.json");
        let desktop_mcp = dir.join("claude_desktop_config.json");
        std::fs::write(
            &settings_path,
            r#"{"hooks":{"SessionStart":[{"matcher":"startup|resume|clear|compact","hooks":[{"command":"/hook/remem context --host claude-code","timeout":15}]}],"UserPromptSubmit":[{"hooks":[{"command":"/hook/remem session-init --host claude-code","timeout":15}]}],"PostToolUse":[{"matcher":"Write|Edit|NotebookEdit|Bash|Grep|Glob|Agent|Task","hooks":[{"command":"/hook/remem observe --host claude-code","timeout":120}]}],"PreCompact":[{"hooks":[{"command":"/hook/remem summarize --host claude-code","timeout":120}]}],"Stop":[{"hooks":[{"command":"/hook/remem summarize --host claude-code","timeout":120}]}]}}"#,
        )?;
        std::fs::write(
            &desktop_mcp,
            r#"{"mcpServers":{"remem":{"command":"/mcp/remem"}}}"#,
        )?;
        let invocation = ContextInvocation {
            cwd: ".".to_string(),
            project: ".".to_string(),
            session_id: None,
            transcript_path: None,
            source: Some("startup".to_string()),
            host: HostKind::ClaudeCode,
            use_colors: false,
            debug: false,
            force: false,
            gate_mode: None,
        };

        let Some(warning) = claude_hook_integrity_warning_from_paths(
            &invocation,
            &settings_path,
            &[dir.join("missing.json"), desktop_mcp],
            PathBuf::from("/hook/remem"),
        ) else {
            panic!("desktop MCP path drift should warn");
        };

        assert!(warning.contains("executable /hook/remem"), "{warning}");
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
}
