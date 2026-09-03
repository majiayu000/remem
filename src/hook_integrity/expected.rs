use super::ExpectedHookSpec;

pub(super) const CLAUDE_EXPECTED: &[ExpectedHookSpec] = &[
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

pub(super) const CODEX_EXPECTED: &[ExpectedHookSpec] = &[
    ExpectedHookSpec {
        event: "SessionStart",
        subcommand: "context",
        nested_subcommand: None,
        host: "codex-cli",
        matcher: None,
        timeout_seconds: None,
    },
    ExpectedHookSpec {
        event: "UserPromptSubmit",
        subcommand: "session-init",
        nested_subcommand: None,
        host: "codex-cli",
        matcher: None,
        timeout_seconds: Some(15),
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
