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
    fn host(&self) -> HostKind;
    fn capabilities(&self) -> HostCapabilities;
    fn default_policy(&self) -> ContextPolicy;
    fn retrieval_hints(&self) -> RetrievalHints;
}

pub(super) struct ClaudeCodeContextProfile;
pub(super) struct CodexCliContextProfile;
pub(super) struct UnknownContextProfile;

impl ContextHostProfile for ClaudeCodeContextProfile {
    fn host(&self) -> HostKind {
        HostKind::ClaudeCode
    }

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
    fn host(&self) -> HostKind {
        HostKind::CodexCli
    }

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
            line: "Use `search`/`get_observations` for details. Codex hook capture is Bash-focused, so save explicit decisions/bugfixes when they matter.",
        }
    }
}

impl ContextHostProfile for UnknownContextProfile {
    fn host(&self) -> HostKind {
        HostKind::Unknown
    }

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
    host_arg
        .and_then(HostKind::parse)
        .or_else(|| {
            std::env::var("REMEM_CONTEXT_HOST")
                .ok()
                .and_then(|value| HostKind::parse(&value))
        })
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
    fn codex_profile_reports_bash_focused_capture() {
        let profile = CodexCliContextProfile;
        let capabilities = profile.capabilities();
        assert!(capabilities.has_mcp_tools);
        assert!(capabilities.observes_bash);
        assert!(!capabilities.observes_native_file_edits);
        assert!(profile.retrieval_hints().line.contains("Bash-focused"));
    }
}
