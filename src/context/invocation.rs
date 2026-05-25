use anyhow::Result;
use serde::Deserialize;

use crate::db;

use super::host::{resolve_host_kind, HostKind};

const CONTEXT_STDIN_TIMEOUT_MS: u64 = 1000;

#[derive(Debug, Clone)]
pub(super) struct ContextInvocation {
    pub cwd: String,
    pub project: String,
    pub session_id: Option<String>,
    pub transcript_path: Option<String>,
    pub host: HostKind,
    pub use_colors: bool,
    pub debug: bool,
    pub force: bool,
    pub gate_mode: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ContextHookInput {
    session_id: Option<String>,
    cwd: Option<String>,
    transcript_path: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ContextCliOptions {
    pub cwd: Option<String>,
    pub session_id: Option<String>,
    pub host: Option<String>,
    pub use_colors: bool,
    pub debug: bool,
    pub force: bool,
    pub gate_mode: Option<String>,
}

pub(super) fn resolve_context_invocation(options: ContextCliOptions) -> Result<ContextInvocation> {
    Ok(resolve_context_invocation_from_stdin_result(
        options,
        crate::hook_stdin::read_stdin_with_timeout(CONTEXT_STDIN_TIMEOUT_MS),
    ))
}

fn resolve_context_invocation_from_stdin_result(
    options: ContextCliOptions,
    stdin_result: Result<Option<String>>,
) -> ContextInvocation {
    let stdin = match stdin_result {
        Ok(stdin) => stdin,
        Err(error) => {
            crate::log::warn(
                "context",
                &format!("failed to read context hook stdin, ignoring: {}", error),
            );
            None
        }
    };
    resolve_context_invocation_from_parts(options, stdin.as_deref())
}

pub(super) fn direct_context_invocation(
    cwd: &str,
    session_id: Option<&str>,
    use_colors: bool,
    host_arg: Option<&str>,
    debug: bool,
) -> ContextInvocation {
    let host = resolve_host_kind(host_arg);
    let project = db::project_from_cwd(cwd);
    ContextInvocation {
        cwd: cwd.to_string(),
        project,
        session_id: clean_optional(session_id.map(str::to_string)),
        transcript_path: None,
        host,
        use_colors,
        debug,
        force: true,
        gate_mode: Some("off".to_string()),
    }
}

pub(super) fn resolve_context_invocation_from_parts(
    options: ContextCliOptions,
    stdin: Option<&str>,
) -> ContextInvocation {
    let hook = parse_hook_input(stdin);
    let cwd = options
        .cwd
        .or_else(|| {
            hook.as_ref()
                .and_then(|hook| clean_optional(hook.cwd.clone()))
        })
        .unwrap_or_else(current_cwd);
    let project = db::project_from_cwd(&cwd);
    let session_id = clean_optional(options.session_id).or_else(|| {
        hook.as_ref()
            .and_then(|hook| clean_optional(hook.session_id.clone()))
    });
    let transcript_path = hook
        .as_ref()
        .and_then(|hook| clean_optional(hook.transcript_path.clone()));
    let host = resolve_host_kind(options.host.as_deref());
    ContextInvocation {
        cwd,
        project,
        session_id,
        transcript_path,
        host,
        use_colors: options.use_colors,
        debug: options.debug,
        force: options.force,
        gate_mode: clean_optional(options.gate_mode),
    }
}

fn parse_hook_input(stdin: Option<&str>) -> Option<ContextHookInput> {
    let raw = stdin?.trim();
    if raw.is_empty() {
        return None;
    }
    match serde_json::from_str(raw) {
        Ok(hook) => Some(hook),
        Err(error) => {
            crate::log::warn(
                "context",
                &format!("invalid context hook payload, ignoring: {}", error),
            );
            None
        }
    }
}

fn current_cwd() -> String {
    std::env::current_dir()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string())
}

fn clean_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_codex_hook_stdin() {
        let invocation = resolve_context_invocation_from_parts(
            ContextCliOptions {
                host: Some("codex-cli".to_string()),
                ..ContextCliOptions::default()
            },
            Some(r#"{"session_id":"sess-1","cwd":"/tmp/remem","transcript_path":"/tmp/t.jsonl"}"#),
        );

        assert_eq!(invocation.session_id.as_deref(), Some("sess-1"));
        assert_eq!(invocation.cwd, "/tmp/remem");
        assert_eq!(invocation.project, db::project_from_cwd("/tmp/remem"));
        assert_eq!(invocation.transcript_path.as_deref(), Some("/tmp/t.jsonl"));
        assert_eq!(invocation.host, HostKind::CodexCli);
    }

    #[test]
    fn cli_values_override_hook_stdin() {
        let invocation = resolve_context_invocation_from_parts(
            ContextCliOptions {
                cwd: Some("/tmp/cli".to_string()),
                session_id: Some("cli-session".to_string()),
                host: Some("codex-cli".to_string()),
                ..ContextCliOptions::default()
            },
            Some(r#"{"session_id":"hook-session","cwd":"/tmp/hook"}"#),
        );

        assert_eq!(invocation.session_id.as_deref(), Some("cli-session"));
        assert_eq!(invocation.cwd, "/tmp/cli");
    }

    #[test]
    fn malformed_hook_stdin_falls_back_to_cli_values() {
        let invocation = resolve_context_invocation_from_parts(
            ContextCliOptions {
                cwd: Some("/tmp/cli".to_string()),
                host: Some("codex-cli".to_string()),
                ..ContextCliOptions::default()
            },
            Some("not-json"),
        );

        assert_eq!(invocation.cwd, "/tmp/cli");
        assert_eq!(invocation.session_id, None);
    }

    #[test]
    fn codex_hook_stdin_timeout_allows_normal_startup_latency() {
        assert!(CONTEXT_STDIN_TIMEOUT_MS >= 1000);
    }

    #[test]
    fn stdin_read_failure_falls_back_to_cli_values() {
        let invocation = resolve_context_invocation_from_stdin_result(
            ContextCliOptions {
                cwd: Some("/tmp/cli".to_string()),
                host: Some("codex-cli".to_string()),
                ..ContextCliOptions::default()
            },
            Err(anyhow::anyhow!("stdin read failed")),
        );

        assert_eq!(invocation.cwd, "/tmp/cli");
        assert_eq!(invocation.session_id, None);
    }
}
