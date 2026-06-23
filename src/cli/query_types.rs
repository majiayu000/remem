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
    /// Merge duplicate workstreams into a canonical workstream.
    Merge {
        /// Project path that owns the workstreams.
        #[arg(long, short)]
        project: String,
        /// Canonical workstream ID to keep visible.
        #[arg(long, value_name = "CANONICAL_ID")]
        into: i64,
        /// Duplicate workstream ID(s) to hide and link to the canonical row.
        #[arg(value_name = "DUPLICATE_ID", required = true)]
        duplicates: Vec<i64>,
        /// Confirm the mutation after reviewing id, project, and duplicates.
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

#[derive(Subcommand)]
pub(in crate::cli) enum UserAction {
    /// Explicitly remember a user-context claim.
    Remember {
        /// User-context owner scope. Non-user scopes require --owner-key.
        #[arg(long, value_enum, default_value = "user")]
        scope: UserClaimScopeArg,
        /// Owner key for the selected scope. Defaults to user:default for user scope.
        #[arg(long)]
        owner_key: Option<String>,
        /// Claim type vocabulary.
        #[arg(long = "type", value_enum, default_value = "preference")]
        claim_type: UserClaimTypeArg,
        /// Stable claim key. Defaults to a deterministic hash of type and text.
        #[arg(long = "key")]
        claim_key: Option<String>,
        /// Claim sensitivity.
        #[arg(long, value_enum, default_value = "normal")]
        sensitivity: UserClaimSensitivityArg,
        /// Confidence from 0.0 to 1.0.
        #[arg(long, default_value = "1.0")]
        confidence: f64,
        /// Optional validity start epoch.
        #[arg(long)]
        valid_from_epoch: Option<i64>,
        /// Optional validity end epoch.
        #[arg(long)]
        valid_to_epoch: Option<i64>,
        /// Emit a single JSON object with stable fields for scripts.
        #[arg(long)]
        json: bool,
        /// Claim text to remember.
        text: String,
    },
    /// Inspect or govern explicit user-context claims.
    Claims {
        #[command(subcommand)]
        action: UserClaimsAction,
    },
    /// Show, refresh, edit, or inspect profile summaries.
    Summary {
        #[command(subcommand)]
        action: UserSummaryAction,
    },
    /// Review automatic user-context extraction candidates.
    Review {
        #[command(subcommand)]
        action: UserReviewAction,
    },
    /// Recall task-aware user context on demand without widening SessionStart.
    Recall {
        /// Query or task topic to recall user context for.
        query: String,
        /// Project path to combine with user context. Defaults to current cwd project.
        #[arg(long, short)]
        project: Option<String>,
        /// Optional task intent, e.g. review, implement, debug, plan.
        #[arg(long)]
        task_intent: Option<String>,
        /// Current file path relevant to the task. Can be repeated.
        #[arg(long = "current-file")]
        current_files: Vec<String>,
        /// Host profile for context, e.g. codex-cli or claude-code.
        #[arg(long)]
        host: Option<String>,
        /// User-context owner scope.
        #[arg(long, value_enum, default_value = "user")]
        scope: UserClaimScopeArg,
        /// Owner key for the selected scope. Defaults to user:default for user scope.
        #[arg(long)]
        owner_key: Option<String>,
        /// Stable current-state key to resolve. Can be repeated.
        #[arg(long = "state-key")]
        state_keys: Vec<String>,
        /// Include personal, sensitive, and restricted claims for explicit audit.
        #[arg(long)]
        include_sensitive: bool,
        /// Include suppressed claims and policy-suppressed memories for explicit audit.
        #[arg(long)]
        include_suppressed: bool,
        /// Maximum included recall items.
        #[arg(long, default_value = "12")]
        limit: i64,
        /// Maximum compact context characters.
        #[arg(long, default_value = "4000")]
        budget_chars: usize,
        /// Emit a single JSON object with stable fields for scripts.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(in crate::cli) enum UserReviewAction {
    /// Show pending user-context candidates.
    Inbox {
        /// Include resolved candidates.
        #[arg(long)]
        include_resolved: bool,
        /// Filter to one review status.
        #[arg(long)]
        status: Option<String>,
        #[arg(long, default_value = "50")]
        limit: i64,
        #[arg(long)]
        json: bool,
    },
    /// Approve a candidate and apply it to active user-context claims.
    Approve {
        id: i64,
        #[arg(long)]
        json: bool,
    },
    /// Edit a candidate before applying it to active user-context claims.
    Edit {
        id: i64,
        #[arg(long)]
        text: String,
        #[arg(long = "type", value_enum)]
        claim_type: Option<UserClaimTypeArg>,
        #[arg(long = "key")]
        claim_key: Option<String>,
        #[arg(long, value_enum)]
        sensitivity: Option<UserClaimSensitivityArg>,
        #[arg(long)]
        note: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Reject a candidate without creating a claim.
    Reject {
        id: i64,
        #[arg(long)]
        note: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Suppress a candidate from future default review use.
    Suppress {
        id: i64,
        #[arg(long)]
        note: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(in crate::cli) enum UserClaimsAction {
    /// List active claims by default.
    List {
        #[arg(long, value_enum)]
        scope: Option<UserClaimScopeArg>,
        #[arg(long)]
        owner_key: Option<String>,
        /// Include inactive, expired, not-yet-valid, and restricted claims.
        #[arg(long)]
        include_inactive: bool,
        #[arg(long, default_value = "50")]
        limit: i64,
        #[arg(long)]
        json: bool,
    },
    /// Show one claim and its source metadata.
    Show {
        id: i64,
        #[arg(long)]
        json: bool,
    },
    /// Explain one claim and its source metadata.
    Why {
        id: i64,
        #[arg(long)]
        json: bool,
    },
    /// Edit a claim by superseding it with a new active row.
    Edit {
        id: i64,
        #[arg(long)]
        text: String,
        #[arg(long = "type", value_enum)]
        claim_type: Option<UserClaimTypeArg>,
        #[arg(long = "key")]
        claim_key: Option<String>,
        #[arg(long, value_enum)]
        sensitivity: Option<UserClaimSensitivityArg>,
        #[arg(long)]
        valid_from_epoch: Option<i64>,
        #[arg(long)]
        valid_to_epoch: Option<i64>,
        #[arg(long)]
        json: bool,
    },
    /// Suppress a claim from default reads without deleting it.
    Suppress {
        id: i64,
        #[arg(long)]
        json: bool,
    },
    /// Return a suppressed claim to active status.
    Unsuppress {
        id: i64,
        #[arg(long)]
        json: bool,
    },
    /// Soft-delete a claim while keeping the audit row.
    Delete {
        id: i64,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(in crate::cli) enum UserSummaryAction {
    /// Show the latest active profile summary.
    Show {
        #[arg(long, short)]
        project: Option<String>,
        #[arg(long, value_enum, default_value = "user")]
        scope: UserClaimScopeArg,
        #[arg(long)]
        owner_key: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Recompile the profile summary from current safe sources.
    Refresh {
        #[arg(long, short)]
        project: Option<String>,
        #[arg(long, value_enum, default_value = "user")]
        scope: UserClaimScopeArg,
        #[arg(long)]
        owner_key: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Manually edit the active profile summary while preserving source ids.
    Edit {
        #[arg(long, short)]
        project: Option<String>,
        #[arg(long, value_enum, default_value = "user")]
        scope: UserClaimScopeArg,
        #[arg(long)]
        owner_key: Option<String>,
        #[arg(long)]
        text: String,
        #[arg(long)]
        json: bool,
    },
    /// Inspect the sources used by the summary compiler.
    Sources {
        #[arg(long, short)]
        project: Option<String>,
        #[arg(long, value_enum, default_value = "user")]
        scope: UserClaimScopeArg,
        #[arg(long)]
        owner_key: Option<String>,
        #[arg(long)]
        include_excluded: bool,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(in crate::cli) enum UserClaimScopeArg {
    User,
    Workspace,
    Repo,
    Session,
}

impl UserClaimScopeArg {
    pub(in crate::cli) fn db_value(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Workspace => "workspace",
            Self::Repo => "repo",
            Self::Session => "session",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(in crate::cli) enum UserClaimTypeArg {
    Identity,
    Role,
    Preference,
    Skill,
    Goal,
    Project,
    Relationship,
    Constraint,
    Activity,
}

impl From<UserClaimTypeArg> for crate::user_context::claims::UserContextClaimType {
    fn from(value: UserClaimTypeArg) -> Self {
        match value {
            UserClaimTypeArg::Identity => Self::Identity,
            UserClaimTypeArg::Role => Self::Role,
            UserClaimTypeArg::Preference => Self::Preference,
            UserClaimTypeArg::Skill => Self::Skill,
            UserClaimTypeArg::Goal => Self::Goal,
            UserClaimTypeArg::Project => Self::Project,
            UserClaimTypeArg::Relationship => Self::Relationship,
            UserClaimTypeArg::Constraint => Self::Constraint,
            UserClaimTypeArg::Activity => Self::Activity,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(in crate::cli) enum UserClaimSensitivityArg {
    Normal,
    Personal,
    Sensitive,
    Restricted,
}

impl From<UserClaimSensitivityArg> for crate::user_context::claims::UserContextSensitivity {
    fn from(value: UserClaimSensitivityArg) -> Self {
        match value {
            UserClaimSensitivityArg::Normal => Self::Normal,
            UserClaimSensitivityArg::Personal => Self::Personal,
            UserClaimSensitivityArg::Sensitive => Self::Sensitive,
            UserClaimSensitivityArg::Restricted => Self::Restricted,
        }
    }
}
