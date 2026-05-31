use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

pub(super) use crate::install::InstallTarget;

#[derive(Parser)]
#[command(
    name = "remem",
    about = "Persistent memory for Claude Code and Codex",
    version
)]
pub(super) struct Cli {
    #[command(subcommand)]
    pub(super) command: Commands,
}

#[derive(Subcommand)]
pub(super) enum Commands {
    /// Render SessionStart memory context for the current project.
    Context {
        /// Project working directory to render context for.
        #[arg(long)]
        cwd: Option<String>,
        /// Host session ID used by duplicate-injection gating.
        #[arg(long)]
        session_id: Option<String>,
        /// Host profile: claude-code, codex-cli, or unknown.
        #[arg(long)]
        host: Option<String>,
        /// Preserve ANSI colors in rendered context.
        #[arg(long)]
        color: bool,
        /// Include context rendering and gate diagnostics.
        #[arg(long)]
        debug: bool,
        /// Force full context emission and update gate state.
        #[arg(long)]
        force: bool,
        /// Duplicate-injection gate mode: off, auto, strict, or delta.
        #[arg(long, value_name = "off|auto|strict|delta")]
        gate: Option<String>,
    },
    /// Hook entrypoint for starting a memory capture session.
    SessionInit,
    /// Hook entrypoint for recording a tool or prompt observation.
    Observe,
    /// Hook entrypoint for summarizing captured session activity.
    Summarize,
    /// Run the background worker loop or one drain pass.
    Worker {
        /// Process ready work once and exit.
        #[arg(long)]
        once: bool,
    },
    /// Run the MCP server over stdio.
    Mcp,
    /// Install remem MCP and hooks into supported hosts.
    Install {
        /// Which host(s) to install into.
        #[arg(long, value_enum, default_value = "auto")]
        target: InstallTarget,
        /// Print what would be written without touching disk.
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove remem MCP and hooks without deleting memory data.
    Uninstall {
        /// Which host(s) to uninstall from. Defaults to all known hosts.
        #[arg(long, value_enum, default_value = "auto")]
        target: InstallTarget,
        /// Print what would be removed without touching disk.
        #[arg(long)]
        dry_run: bool,
    },
    /// Clean old events and archive stale memories.
    Cleanup,
    /// Sync the project memory index into CLAUDE.md.
    SyncMemory {
        /// Project working directory to sync.
        #[arg(long)]
        cwd: Option<String>,
    },
    /// List, add, or remove remembered user preferences.
    Preferences {
        #[command(subcommand)]
        action: PreferenceAction,
    },
    /// Inspect or repair failed pending observation rows.
    Pending {
        #[command(subcommand)]
        action: PendingAction,
    },
    /// Review, approve, edit, or discard memory candidates.
    Review {
        #[command(subcommand)]
        action: ReviewAction,
    },
    /// Auditably delete, reject, or stale curated memories by ID.
    Govern {
        /// Restrict governance to one project path.
        #[arg(long, short)]
        project: Option<String>,
        /// Governance action to apply.
        #[arg(long, value_enum)]
        action: MemoryGovernanceCliAction,
        /// Required human-readable reason for non-dry-run mutations.
        #[arg(long)]
        reason: Option<String>,
        /// Actor recorded in governance audit events.
        #[arg(long)]
        actor: Option<String>,
        /// Select memories whose title, content, or search context contains this text.
        #[arg(long)]
        query: Option<String>,
        /// Select memories by type. Defaults to the existing positional IDs only.
        #[arg(long, alias = "type")]
        memory_type: Option<String>,
        /// Select memories by status. Omit for active-only selector batches; use "all" for any status.
        #[arg(long)]
        status: Option<String>,
        /// Maximum selector matches to include before governance is applied.
        #[arg(long, default_value = "50")]
        limit: i64,
        /// Number of selector matches to skip.
        #[arg(long, default_value = "0")]
        offset: i64,
        /// Read additional memory IDs from a text file. Whitespace and commas are accepted.
        #[arg(long)]
        from_file: Option<PathBuf>,
        /// Read additional memory IDs from stdin. Whitespace and commas are accepted.
        #[arg(long = "stdin")]
        read_stdin: bool,
        /// Required for destructive non-dry-run governance mutations.
        #[arg(long)]
        confirm_destructive: bool,
        /// Preview affected memories without mutating them.
        #[arg(long)]
        dry_run: bool,
        /// Emit a single JSON object with stable fields for scripts.
        #[arg(long)]
        json: bool,
        /// Memory IDs to govern.
        #[arg()]
        ids: Vec<i64>,
    },
    /// Audit likely mis-scoped memories and workstreams for one project.
    AuditScope {
        /// Project path to audit. Defaults to the current project.
        #[arg(long, short)]
        project: Option<String>,
        /// Maximum rows per audit bucket.
        #[arg(long, short = 'n', default_value = "50")]
        limit: i64,
        /// Emit a single JSON object with stable fields for scripts.
        #[arg(long)]
        json: bool,
    },
    /// Reroute object-qualified memory refs to a new owner without deleting history.
    Reroute {
        /// Object refs such as memory:123 or workstream:18. Commas are accepted.
        #[arg(long, value_delimiter = ',', num_args = 1..)]
        refs: Vec<String>,
        /// Compatibility shorthand for memory:<id> refs. Commas are accepted.
        #[arg(long, value_delimiter = ',')]
        ids: Vec<i64>,
        /// New owner scope: user, workspace, repo, tool, domain, workstream, or session.
        #[arg(long)]
        owner_scope: String,
        /// New owner key inside the owner scope.
        #[arg(long)]
        owner_key: String,
        /// Set target_project to this project path.
        #[arg(long)]
        target_project: Option<String>,
        /// Store SQL NULL in target_project.
        #[arg(long)]
        clear_target_project: bool,
        /// Optional topic domain to store with the route.
        #[arg(long)]
        topic_domain: Option<String>,
        /// Optional context class to store with the route.
        #[arg(long)]
        context_class: Option<String>,
        /// Optional routing confidence.
        #[arg(long)]
        confidence: Option<f64>,
        /// Human-readable reason written to the audit event.
        #[arg(long)]
        reason: Option<String>,
        /// Required to write changes. Omit for dry-run preview.
        #[arg(long)]
        confirm: bool,
        /// Preview without writing changes.
        #[arg(long)]
        dry_run: bool,
        /// Emit a single JSON object with stable fields for scripts.
        #[arg(long)]
        json: bool,
    },
    /// Archive memory refs or pause workstream refs without hard-deleting rows.
    Archive {
        /// Object refs such as memory:123 or workstream:18. Commas are accepted.
        #[arg(long, value_delimiter = ',', num_args = 1..)]
        refs: Vec<String>,
        /// Compatibility shorthand for memory:<id> refs. Commas are accepted.
        #[arg(long, value_delimiter = ',')]
        ids: Vec<i64>,
        /// Human-readable reason written to the audit event.
        #[arg(long)]
        reason: Option<String>,
        /// Required to write changes. Omit for dry-run preview.
        #[arg(long)]
        confirm: bool,
        /// Preview without writing changes.
        #[arg(long)]
        dry_run: bool,
        /// Emit a single JSON object with stable fields for scripts.
        #[arg(long)]
        json: bool,
    },
    /// Merge duplicate active project preferences into one canonical row.
    MergePreferences {
        /// Project path to clean. Defaults to the current project.
        #[arg(long, short)]
        project: Option<String>,
        /// Preview without writing changes.
        #[arg(long)]
        dry_run: bool,
        /// Required to write changes.
        #[arg(long)]
        confirm: bool,
        /// Emit a single JSON object with stable fields for scripts.
        #[arg(long)]
        json: bool,
    },
    /// Show token and cost accounting.
    Usage {
        /// Restrict usage totals to one project path.
        #[arg(long, short)]
        project: Option<String>,
        /// Number of daily buckets to show.
        #[arg(long, default_value = "14")]
        days: i64,
        /// Number of weekly buckets to show.
        #[arg(long, default_value = "8")]
        weeks: i64,
    },
    /// Show memory store health, queue counts, and schema status.
    Status {
        /// Emit a single JSON object with stable fields for scripts.
        #[arg(long)]
        json: bool,
    },
    /// Check install, hook, MCP, database, and queue health.
    Doctor {
        /// Emit a single JSON object with per-check status. Stable shape;
        /// fields: `version`, `status`, `fails`, `warns`, `checks[]`.
        #[arg(long)]
        json: bool,
        /// Suppress human-readable output. Useful when only the exit code
        /// matters (CI/scripts). Has no effect when `--json` is set.
        #[arg(long, short)]
        quiet: bool,
    },
    /// Search curated memories from the terminal.
    Search {
        /// Search query.
        query: String,
        /// Restrict search to one project path.
        #[arg(long, short)]
        project: Option<String>,
        /// Restrict search to one memory type.
        #[arg(long, alias = "type", short = 't')]
        memory_type: Option<String>,
        /// Maximum curated results to show.
        #[arg(long, short = 'n', default_value = "10")]
        limit: i64,
        /// Result offset for pagination.
        #[arg(long, default_value = "0")]
        offset: i64,
        /// Include memories from this branch plus branchless older rows.
        #[arg(long)]
        branch: Option<String>,
        /// Include stale or archived memories when searching.
        #[arg(long)]
        include_stale: bool,
        /// Expand through related entities after the first search pass.
        #[arg(long)]
        multi_hop: bool,
        /// Show retrieval channels, ranks, and score contributions.
        #[arg(long)]
        explain: bool,
        /// Emit a single JSON object with stable fields for scripts.
        #[arg(long)]
        json: bool,
    },
    /// Look up git commits and linked memory sessions.
    Commit {
        #[command(subcommand)]
        action: CommitAction,
    },
    /// Show one memory by ID.
    Show {
        /// Memory ID to show.
        id: i64,
        #[arg(long)]
        json: bool,
    },
    /// Explain retrieval visibility and scoring for one memory.
    Why {
        /// Memory ID to explain.
        id: i64,
        /// Restrict explanation to one project path.
        #[arg(long, short)]
        project: Option<String>,
        /// Restrict explanation to one branch.
        #[arg(long)]
        branch: Option<String>,
    },
    /// Run the golden retrieval evaluation dataset.
    Eval {
        /// Golden dataset path.
        #[arg(long, default_value = "eval/golden.json")]
        dataset: String,
        /// Number of results to evaluate per query.
        #[arg(long, short = 'k', default_value = "5")]
        k: usize,
    },
    /// Run end-to-end local API evaluation.
    EvalE2e {
        /// Number of results to evaluate per query.
        #[arg(long, short = 'k', default_value = "5")]
        k: usize,
        /// Emit the evaluation report as JSON.
        #[arg(long)]
        json: bool,
        /// Keep the temporary REMEM_DATA_DIR for inspection.
        #[arg(long)]
        keep_data_dir: bool,
    },
    /// Run local retrieval diagnostics.
    EvalLocal,
    /// Backfill entity records for existing memories.
    BackfillEntities,
    /// Encrypt the local database if encryption is configured.
    Encrypt,
    /// Run the local HTTP API server.
    Api {
        /// Loopback port for the HTTP API.
        #[arg(long, short, default_value = "5567")]
        port: u16,
    },
    /// Merge duplicate or overlapping memories.
    Dream {
        /// Restrict dream processing to one project path.
        #[arg(long, short)]
        project: Option<String>,
        /// Print what would be merged without writing to DB
        #[arg(long)]
        dry_run: bool,
    },
    /// Admin commands for database backup.
    Admin {
        #[command(subcommand)]
        action: AdminAction,
    },
    /// Import commands for moving older backup rows into the runtime database.
    Import {
        #[command(subcommand)]
        action: ImportAction,
    },
}

#[derive(Subcommand)]
pub(in crate::cli) enum PreferenceAction {
    List,
    Add {
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        global: bool,
        text: String,
    },
    Remove {
        id: i64,
    },
}

#[derive(Subcommand)]
pub(in crate::cli) enum AdminAction {
    /// Back up the existing remem database to a timestamped file.
    Backup {
        /// Output path. Defaults to <data_dir>/backups/remem-backup-<ts>.sqlite.
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub(in crate::cli) enum ImportAction {
    /// Import memories from an older backup sqlite file. Transcripts are not
    /// replayed; only the old `memories` table is imported, with synthesized
    /// provenance defaults.
    Backup {
        /// Backup sqlite path produced by `remem admin backup`.
        #[arg(long)]
        source: PathBuf,
        /// Acknowledge that import is best-effort and skips constraint
        /// violations rather than failing.
        #[arg(long)]
        best_effort: bool,
    },
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
#[allow(clippy::enum_variant_names)]
pub(in crate::cli) enum PendingAction {
    /// List failed pending observation rows.
    #[command(alias = "list")]
    ListFailed {
        /// Restrict rows to one project path.
        #[arg(long, short)]
        project: Option<String>,
        /// Maximum failed rows to show.
        #[arg(long, short = 'n', default_value = "20")]
        limit: i64,
        #[arg(long)]
        json: bool,
    },
    /// Move failed pending observation rows back to pending.
    #[command(alias = "retry")]
    RetryFailed {
        /// Restrict rows to one project path.
        #[arg(long, short)]
        project: Option<String>,
        /// Maximum failed rows to retry.
        #[arg(long, short = 'n', default_value = "100")]
        limit: i64,
        /// Preview retry count without mutating rows.
        #[arg(long)]
        dry_run: bool,
    },
    /// Purge old failed pending observation rows.
    #[command(alias = "purge")]
    PurgeFailed {
        /// Restrict rows to one project path.
        #[arg(long, short)]
        project: Option<String>,
        /// Only purge failed rows older than this many days.
        #[arg(long, default_value = "7")]
        older_than_days: i64,
        /// Preview purge count without deleting rows.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
pub(in crate::cli) enum ReviewAction {
    List {
        #[arg(long, short)]
        project: Option<String>,
        #[arg(long, short = 'n', default_value = "20")]
        limit: i64,
    },
    Approve {
        id: i64,
    },
    Discard {
        id: i64,
    },
    Edit {
        id: i64,
        #[arg(long)]
        text: Option<String>,
        #[arg(long = "topic-key")]
        topic_key: Option<String>,
        #[arg(long = "type")]
        memory_type: Option<String>,
        #[arg(long)]
        scope: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(in crate::cli) enum MemoryGovernanceCliAction {
    Delete,
    Reject,
    Stale,
}
