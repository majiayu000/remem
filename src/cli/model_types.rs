use clap::Subcommand;

#[derive(Subcommand)]
pub(in crate::cli) enum ModelAction {
    /// Show the currently effective memory AI model configuration.
    Current {
        /// Host to inspect, such as codex-cli or claude-code. Omit to show installed hosts.
        #[arg(long)]
        host: Option<String>,
        /// Inspect a named memory AI profile directly.
        #[arg(long)]
        profile: Option<String>,
    },
    /// List built-in Codex model presets and examples.
    List,
    /// Switch a host/profile to a preset or explicit model name.
    Use {
        /// Preset or model name: cheap, balanced, quality, auto, or an explicit model.
        target: String,
        /// Host to update. Defaults to [memory_ai].default_host.
        #[arg(long)]
        host: Option<String>,
        /// Update a named memory AI profile directly instead of resolving a host.
        #[arg(long)]
        profile: Option<String>,
        /// Codex reasoning effort: low, medium, or high.
        #[arg(long, value_name = "low|medium|high")]
        reasoning: Option<String>,
        /// Print the config diff without writing files.
        #[arg(long)]
        dry_run: bool,
    },
    /// Check the selected model profile; pass --live to make a tiny AI call.
    Test {
        /// Host to test. Defaults to [memory_ai].default_host.
        #[arg(long)]
        host: Option<String>,
        /// Test a named memory AI profile directly.
        #[arg(long)]
        profile: Option<String>,
        /// Actually call the configured AI model. Without this, only config is checked.
        #[arg(long)]
        live: bool,
    },
    /// Restore the config backup saved by the last `remem model use`.
    Rollback,
}
