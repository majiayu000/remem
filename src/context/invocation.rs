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
    pub source: Option<String>,
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
    source: Option<String>,
    target: Option<ContextHookTarget>,
}

#[derive(Debug, Deserialize)]
struct ContextHookTarget {
    source: Option<String>,
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
        source: None,
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
    let source = hook.as_ref().and_then(hook_source);
    let host = resolve_host_kind(options.host.as_deref());
    let host_config =
        crate::runtime_config::resolve_host_runtime_config(Some(host.as_env_value())).ok();
    let gate_mode = effective_gate_mode(
        options.gate_mode,
        std::env::var("REMEM_CONTEXT_GATE").ok(),
        host_config
            .as_ref()
            .and_then(|config| config.context_gate.clone()),
    );
    let use_colors = options.use_colors
        || host_config
            .as_ref()
            .map(|config| config.context_color)
            .unwrap_or(false);
    ContextInvocation {
        cwd,
        project,
        session_id,
        transcript_path,
        source,
        host,
        use_colors,
        debug: options.debug,
        force: options.force,
        gate_mode,
    }
}

fn hook_source(hook: &ContextHookInput) -> Option<String> {
    clean_optional(hook.source.clone()).or_else(|| {
        hook.target
            .as_ref()
            .and_then(|target| clean_optional(target.source.clone()))
    })
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

fn effective_gate_mode(
    cli_mode: Option<String>,
    env_mode: Option<String>,
    config_mode: Option<String>,
) -> Option<String> {
    clean_optional(cli_mode)
        .or_else(|| clean_optional(env_mode))
        .or_else(|| clean_optional(config_mode))
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
            Some(
                r#"{"session_id":"sess-1","cwd":"/tmp/remem","transcript_path":"/tmp/t.jsonl","target":{"type":"SessionStart","source":"Startup"}}"#,
            ),
        );

        assert_eq!(invocation.session_id.as_deref(), Some("sess-1"));
        assert_eq!(invocation.cwd, "/tmp/remem");
        assert_eq!(invocation.project, db::project_from_cwd("/tmp/remem"));
        assert_eq!(invocation.transcript_path.as_deref(), Some("/tmp/t.jsonl"));
        assert_eq!(invocation.source.as_deref(), Some("Startup"));
        assert_eq!(invocation.host, HostKind::CodexCli);
    }

    #[test]
    fn parses_root_hook_source() {
        let invocation = resolve_context_invocation_from_parts(
            ContextCliOptions {
                host: Some("codex-cli".to_string()),
                ..ContextCliOptions::default()
            },
            Some(r#"{"session_id":"sess-1","cwd":"/tmp/remem","source":"compact"}"#),
        );

        assert_eq!(invocation.source.as_deref(), Some("compact"));
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

    #[test]
    fn env_gate_overrides_host_config_gate() {
        assert_eq!(
            effective_gate_mode(None, Some("off".to_string()), Some("strict".to_string()))
                .as_deref(),
            Some("off")
        );
    }

    #[test]
    fn cli_gate_overrides_env_and_host_config_gate() {
        assert_eq!(
            effective_gate_mode(
                Some("delta".to_string()),
                Some("off".to_string()),
                Some("strict".to_string())
            )
            .as_deref(),
            Some("delta")
        );
    }
}
