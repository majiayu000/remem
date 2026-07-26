//! Clap types for the context-gate and context-plan (GH-934) commands.

use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub(in crate::cli) enum ContextGateAction {
    /// Show recent read-only context injection rows.
    Status {
        /// Restrict rows to one project path.
        #[arg(long, short)]
        project: Option<String>,
        /// Restrict rows to one host session ID.
        #[arg(long)]
        session: Option<String>,
        /// Maximum recent rows to show.
        #[arg(long, short = 'n', default_value = "20")]
        limit: i64,
        /// Emit a single JSON object with stable fields for scripts.
        #[arg(long)]
        json: bool,
    },
}

/// Arguments for `remem context-plan` (GH-934 debug surface): compile a
/// deterministic task-aware retrieval plan. Prints the plan only, never
/// memory contents; no database, LLM, or network access.
#[derive(Args)]
pub(in crate::cli) struct ContextPlanArgs {
    /// Task description used for intent resolution and plan compilation.
    #[arg(long)]
    pub(in crate::cli) task: String,
    /// Explicit intent (wins over keyword resolution): resume-work,
    /// explain-decision, debug-failure, apply-preference, review-change,
    /// or explore-history.
    #[arg(long)]
    pub(in crate::cli) intent: Option<String>,
    /// Project key override; defaults to the project derived from --cwd.
    #[arg(long)]
    pub(in crate::cli) project: Option<String>,
    /// Working directory used to derive the project key.
    #[arg(long)]
    pub(in crate::cli) cwd: Option<String>,
    /// Branch scope filter.
    #[arg(long)]
    pub(in crate::cli) branch: Option<String>,
    /// Agent role: coder, reviewer, planner, or researcher.
    #[arg(long, default_value = "coder")]
    pub(in crate::cli) role: String,
    /// Risk class: low, medium, or high.
    #[arg(long, default_value = "medium")]
    pub(in crate::cli) risk: String,
    /// Total token budget for the compiled plan.
    #[arg(long, default_value_t = 4000)]
    pub(in crate::cli) token_budget: u32,
    /// Allow superseded memories in scope.
    #[arg(long)]
    pub(in crate::cli) include_superseded: bool,
    /// As-of epoch-seconds scope pin; 0 means no pin.
    #[arg(long, default_value_t = 0)]
    pub(in crate::cli) as_of_epoch: i64,
    /// Emit the full plan as JSON.
    #[arg(long)]
    pub(in crate::cli) json: bool,
}
