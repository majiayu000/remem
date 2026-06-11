use clap::{Subcommand, ValueEnum};

#[derive(Subcommand)]
pub(in crate::cli) enum TimelineAction {
    /// Get chronological observations around an anchor observation or search query.
    Around {
        /// Anchor observation ID.
        #[arg(long)]
        anchor: Option<i64>,
        /// Search query used to resolve the anchor.
        #[arg(long)]
        query: Option<String>,
        /// Restrict timeline rows to one project path.
        #[arg(long, short)]
        project: Option<String>,
        /// Observations before the anchor.
        #[arg(long, default_value = "5")]
        depth_before: i64,
        /// Observations after the anchor.
        #[arg(long, default_value = "5")]
        depth_after: i64,
        /// Emit a single JSON object with stable fields for scripts.
        #[arg(long)]
        json: bool,
    },
    /// Generate a project timeline report.
    Report {
        /// Project path to report.
        project: String,
        /// Include recent timeline and monthly breakdown.
        #[arg(long)]
        full: bool,
        /// Emit a single JSON object with stable fields for scripts.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(in crate::cli) enum WorkstreamAction {
    /// List project workstreams.
    List {
        /// Project path to list workstreams for.
        #[arg(long, short)]
        project: String,
        /// Status filter: active, paused, completed, abandoned.
        #[arg(long, value_enum)]
        status: Option<WorkstreamStatusArg>,
        /// Emit a single JSON object with stable fields for scripts.
        #[arg(long)]
        json: bool,
    },
    /// Update a workstream's manual status, next action, or blockers.
    Update {
        /// Workstream ID to update.
        id: i64,
        /// Project path that owns the workstream.
        #[arg(long, short)]
        project: String,
        /// New status: active, paused, completed, abandoned.
        #[arg(long, value_enum)]
        status: Option<WorkstreamStatusArg>,
        /// Next action to take.
        #[arg(long)]
        next_action: Option<String>,
        /// Current blockers.
        #[arg(long)]
        blockers: Option<String>,
        /// Confirm the mutation after reviewing id, project, and fields.
        #[arg(long)]
        confirm: bool,
        /// Emit a single JSON object with stable fields for scripts.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(in crate::cli) enum WorkstreamStatusArg {
    Active,
    Paused,
    Completed,
    Abandoned,
}

impl WorkstreamStatusArg {
    pub(in crate::cli) fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Abandoned => "abandoned",
        }
    }
}

#[derive(Subcommand)]
pub(in crate::cli) enum CommitAction {
    /// Look up git metadata and linked memory sessions for a full or short SHA.
    Show {
        sha: String,
        #[arg(long, short)]
        project: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// List commits linked to a content session ID or memory session ID.
    Session {
        session_id: String,
        #[arg(long, short)]
        project: Option<String>,
        #[arg(long, short = 'n', default_value = "20")]
        limit: i64,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(in crate::cli) enum RawAction {
    /// Search raw captured user/assistant chat turns, not curated memories.
    Search {
        /// Literal phrase or terms to search in raw archive rows.
        query: String,
        /// Restrict search to one project path.
        #[arg(long, short)]
        project: Option<String>,
        /// Include rows from this branch plus branchless older rows.
        #[arg(long)]
        branch: Option<String>,
        /// Restrict search to one message role.
        #[arg(long, value_enum)]
        role: Option<RawRole>,
        /// Maximum raw rows to show.
        #[arg(long, short = 'n', default_value = "20")]
        limit: i64,
        /// Result offset for pagination.
        #[arg(long, default_value = "0")]
        offset: i64,
        /// Emit a single JSON object with stable fields for scripts.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(in crate::cli) enum RawRole {
    User,
    Assistant,
}

impl RawRole {
    pub(in crate::cli) fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}
