use super::policy::ContextPolicy;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKind {
    ClaudeCode,
    CodexCli,
    Unknown,
}

impl HostKind {
    pub fn as_env_value(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::CodexCli => "codex-cli",
            Self::Unknown => "unknown",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "claude" | "claude-code" | "claudecode" => Some(Self::ClaudeCode),
            "codex" | "codex-cli" | "codexcli" => Some(Self::CodexCli),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

pub(super) struct HostCapabilities {
    pub has_mcp_tools: bool,
    pub has_session_start_hook: bool,
    pub has_user_prompt_submit_hook: bool,
    pub observes_native_file_edits: bool,
    pub observes_bash: bool,
}

pub(super) struct RetrievalHints {
    pub line: &'static str,
}

pub(super) trait ContextHostProfile {
    fn capabilities(&self) -> HostCapabilities;
    fn default_policy(&self) -> ContextPolicy;
    fn retrieval_hints(&self) -> RetrievalHints;
}

pub(super) struct ClaudeCodeContextProfile;
pub(super) struct CodexCliContextProfile;
pub(super) struct UnknownContextProfile;

impl ContextHostProfile for ClaudeCodeContextProfile {
    fn capabilities(&self) -> HostCapabilities {
        HostCapabilities {
            has_mcp_tools: true,
            has_session_start_hook: true,
            has_user_prompt_submit_hook: true,
            observes_native_file_edits: true,
            observes_bash: true,
        }
    }

    fn default_policy(&self) -> ContextPolicy {
        ContextPolicy::from_env()
    }

    fn retrieval_hints(&self) -> RetrievalHints {
        RetrievalHints {
            line: "Use `search`/`get_observations` for details. `save_memory` after decisions/bugfixes.",
        }
    }
}

impl ContextHostProfile for CodexCliContextProfile {
    fn capabilities(&self) -> HostCapabilities {
        HostCapabilities {
            has_mcp_tools: true,
            has_session_start_hook: true,
            has_user_prompt_submit_hook: false,
            observes_native_file_edits: false,
            observes_bash: false,
        }
    }

    fn default_policy(&self) -> ContextPolicy {
        ContextPolicy::from_env()
    }

    fn retrieval_hints(&self) -> RetrievalHints {
        RetrievalHints {
            line: "Use `search`/`get_observations` for details. Codex automatic capture is Stop/context-focused, so save explicit decisions/bugfixes when they matter.",
        }
    }
}

impl ContextHostProfile for UnknownContextProfile {
    fn capabilities(&self) -> HostCapabilities {
        HostCapabilities {
            has_mcp_tools: true,
            has_session_start_hook: true,
            has_user_prompt_submit_hook: false,
            observes_native_file_edits: false,
            observes_bash: true,
        }
    }

    fn default_policy(&self) -> ContextPolicy {
        ContextPolicy::from_env()
    }

    fn retrieval_hints(&self) -> RetrievalHints {
        RetrievalHints {
            line: "Use `search`/`get_observations` for details. `save_memory` after decisions/bugfixes.",
        }
    }
}

pub(super) fn resolve_host_kind(host_arg: Option<&str>) -> HostKind {
    let env_host = std::env::var("REMEM_CONTEXT_HOST").ok();
    let default_host = crate::runtime_config::default_host().ok();
    resolve_host_kind_from_sources(host_arg, env_host.as_deref(), default_host.as_deref())
}

fn resolve_host_kind_from_sources(
    host_arg: Option<&str>,
    env_host: Option<&str>,
    default_host: Option<&str>,
) -> HostKind {
    host_arg
        .and_then(HostKind::parse)
        .or_else(|| env_host.and_then(HostKind::parse))
        .or_else(|| default_host.and_then(HostKind::parse))
        .unwrap_or(HostKind::Unknown)
}

pub(super) fn resolve_profile(host: HostKind) -> Box<dyn ContextHostProfile> {
    match host {
        HostKind::ClaudeCode => Box::new(ClaudeCodeContextProfile),
        HostKind::CodexCli => Box::new(CodexCliContextProfile),
        HostKind::Unknown => Box::new(UnknownContextProfile),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_host_names() {
        assert_eq!(HostKind::parse("claude-code"), Some(HostKind::ClaudeCode));
        assert_eq!(HostKind::parse("codex-cli"), Some(HostKind::CodexCli));
        assert_eq!(HostKind::parse("unknown"), Some(HostKind::Unknown));
        assert_eq!(HostKind::parse("missing"), None);
    }

    #[test]
    fn codex_profile_reports_stop_focused_capture() {
        let profile = CodexCliContextProfile;
        let capabilities = profile.capabilities();
        assert!(capabilities.has_mcp_tools);
        assert!(!capabilities.observes_bash);
        assert!(!capabilities.observes_native_file_edits);
        assert!(profile
            .retrieval_hints()
            .line
            .contains("Stop/context-focused"));
    }

    #[test]
    fn missing_host_uses_configured_default_source() {
        assert_eq!(
            resolve_host_kind_from_sources(None, None, Some("codex-cli")),
            HostKind::CodexCli
        );
    }

    #[test]
    fn legacy_env_host_overrides_configured_default_source() {
        assert_eq!(
            resolve_host_kind_from_sources(None, Some("claude-code"), Some("codex-cli")),
            HostKind::ClaudeCode
        );
    }

    #[test]
    fn explicit_unknown_host_does_not_fall_back_to_default_source() {
        assert_eq!(
            resolve_host_kind_from_sources(Some("unknown"), None, Some("codex-cli")),
            HostKind::Unknown
        );
    }
}
