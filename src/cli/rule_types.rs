use clap::{Subcommand, ValueEnum};

#[derive(Subcommand)]
pub(in crate::cli) enum RulesAction {
    /// List compiled rules and their provenance for a project.
    List {
        /// Project working directory. Defaults to the current directory.
        #[arg(long)]
        project: Option<String>,
    },
    /// Disable a compiled rule after the next worker rebuild.
    Disable { rule_id: String },
    /// Re-enable a compiled rule after the next worker rebuild.
    Enable { rule_id: String },
    /// Override a compiled rule action after the next worker rebuild.
    SetAction {
        rule_id: String,
        #[arg(value_enum)]
        action: RuleActionArg,
        /// Host expected to enforce block actions. Required for block mode.
        #[arg(long, value_enum)]
        host: Option<RuleHostArg>,
    },
    /// Internal read-only Claude PreToolUse Bash evaluator.
    Eval {
        /// Hook host. Only claude-code supports pre-execution enforcement.
        #[arg(long, value_enum)]
        host: Option<RuleHostArg>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub(in crate::cli) enum RuleActionArg {
    Warn,
    Block,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(in crate::cli) enum RuleHostArg {
    ClaudeCode,
    CodexCli,
}
