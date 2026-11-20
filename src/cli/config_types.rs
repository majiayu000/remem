use clap::Subcommand;

#[derive(Subcommand)]
pub(in crate::cli) enum ConfigAction {
    /// Print the active config file path.
    Path,
    /// Print the resolved config text, including built-in defaults.
    Show,
    /// Create or update the config file with default sections.
    Init,
    /// Set one scalar config key, for example memory_ai.profiles.codex.model.
    Set { key: String, value: String },
    /// Explicitly migrate legacy Claude context_gate = "off" to "auto".
    MigrateClaudeGate {
        /// Print the planned migration without writing the config file.
        #[arg(long)]
        dry_run: bool,
        /// Emit a single JSON object with stable fields for scripts.
        #[arg(long)]
        json: bool,
    },
}
