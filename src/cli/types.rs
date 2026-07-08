use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

pub(in crate::cli) use super::archive_types::{ExportArgs, ImportAction};
pub(in crate::cli) use super::config_types::ConfigAction;
pub(in crate::cli) use super::embedding_types::EmbeddingAction;
pub(in crate::cli) use super::memory_types::{
    MemoryAction, MemoryCleanupType, MemorySuppressionsAction,
};
pub(in crate::cli) use super::procedure_types::ProcedureAction;
pub(in crate::cli) use super::query_types::{
    CommitAction, RawAction, RawRole, TimelineAction, UserAction, WorkstreamAction,
    WorkstreamStatusArg,
};
pub(in crate::cli) use super::review_types::{
    GraphReviewAction, ReviewAction, ReviewBatchFilterArgs,
};
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
    #[command(
        after_help = "Environment variables:\n  REMEM_CONTEXT_HOST: legacy host fallback when --host is omitted.\n  REMEM_CONTEXT_GATE: legacy gate fallback when neither --gate nor host config sets context_gate.\n  REMEM_CONTEXT_GATE_HOSTS: comma-separated hosts that use duplicate-injection gating.\n  REMEM_CONTEXT_SUPPRESS_SOURCES: hook sources suppressed when the context hash is unchanged.\n  REMEM_CONTEXT_DEBUG=1: include context rendering and gate diagnostics."
    )]
    Context {
        /// Project working directory to render context for.
        #[arg(long)]
        cwd: Option<String>,
        /// Host session ID used by duplicate-injection gating.
        #[arg(long)]
        session_id: Option<String>,
        /// Host profile: claude-code, codex-cli, or unknown. Overrides REMEM_CONTEXT_HOST.
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
        /// Duplicate-injection gate mode: off, auto, strict, or delta. Overrides host config and REMEM_CONTEXT_GATE.
        #[arg(long, value_name = "off|auto|strict|delta")]
        gate: Option<String>,
    },
    /// Inspect recent context duplicate-injection gate decisions.
    ContextGate {
        #[command(subcommand)]
        action: ContextGateAction,
    },
    /// Inspect or edit remem runtime configuration.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Inspect or switch the memory AI model profile.
    #[command(
        after_help = "Examples:\n  remem model current\n  remem model list\n  remem model use cheap\n  remem model use balanced --dry-run\n  remem model use gpt-5.2 --reasoning medium\n  remem model use haiku --host claude-code\n  remem model test\n  remem model test --live\n  remem model rollback\n\nNotes:\n  Presets currently target Codex profiles. For Claude Code, pass an explicit model name.\n  `test` is a config check by default and only calls AI when --live is set.\n  `use` writes ~/.remem/config.toml and saves a rollback backup first."
    )]
    Model {
        #[command(subcommand)]
        action: ModelAction,
    },
    /// Manage local semantic embedding models and backfill.
    Embedding {
        #[command(subcommand)]
        action: EmbeddingAction,
    },
    /// Hook entrypoint for starting a memory capture session.
    SessionInit {
        /// Host profile for this hook: claude-code, codex-cli, or unknown.
        #[arg(long)]
        host: Option<String>,
    },
    /// Hook entrypoint for recording a tool or prompt observation.
    Observe {
        /// Host profile for this hook: claude-code, codex-cli, or unknown.
        #[arg(long)]
        host: Option<String>,
    },
    /// Hook entrypoint for summarizing captured session activity.
    Summarize {
        /// Host profile for this hook: claude-code, codex-cli, or unknown.
        #[arg(long)]
        host: Option<String>,
        /// Memory AI profile name from [memory_ai.profiles].
        #[arg(long)]
        profile: Option<String>,
    },
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
        /// Install automatic capture hooks without registering MCP servers.
        #[arg(long)]
        hooks_only: bool,
        /// Repair host hooks without touching MCP, runtime store, or API token.
        #[arg(long)]
        repair: bool,
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
    /// Clean old events, compressed source observations, and stale lifecycle state.
    Cleanup {
        /// Preview retention counts without mutating data.
        #[arg(long)]
        dry_run: bool,
        /// Emit a single JSON object with stable fields for scripts.
        #[arg(long)]
        json: bool,
        /// Purge archived failed queue rows older than DAYS. Defaults to 90 days when the flag is present.
        #[arg(long, value_name = "DAYS", num_args = 0..=1, default_missing_value = "90")]
        archived_failures: Option<i64>,
    },
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
    /// Store and inspect explicit user-context claims.
    User {
        #[command(subcommand)]
        action: UserAction,
    },
    /// Inspect or apply data-only memory governance plans.
    Memory {
        #[command(subcommand)]
        action: MemoryAction,
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
    /// Review, approve, reject, defer, or inspect graph candidates.
    GraphReview {
        #[command(subcommand)]
        action: GraphReviewAction,
    },
    /// Inspect promoted procedure memories.
    Procedures {
        #[command(subcommand)]
        action: ProcedureAction,
    },
    /// Auditably delete, reject, stale, or acknowledge curated memories by ID.
    Govern {
        /// Restrict governance to one project path.
        #[arg(long, short)]
        project: Option<String>,
        /// Governance action to apply.
        #[arg(long, value_enum)]
        action: MemoryGovernanceCliAction,
        /// Pattern id required when action is acknowledge-pattern.
        #[arg(long)]
        acknowledge_pattern: Option<String>,
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
        #[arg(long, value_delimiter = ',', num_args = 1..)]
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
        #[arg(long, value_delimiter = ',', num_args = 1..)]
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
        /// Include policy-suppressed memories when searching.
        #[arg(long)]
        include_suppressed: bool,
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
    /// Resolve the current memory for a stable state key.
    Current {
        /// Stable state key, such as a durable topic key.
        state_key: String,
        /// Restrict lookup to one project path. Defaults to repo-owned state plus global user state.
        #[arg(long, short)]
        project: Option<String>,
        /// Restrict lookup to one memory type.
        #[arg(long, alias = "type", short = 't')]
        memory_type: Option<String>,
        /// Explicit state-key owner scope, such as `repo` or `user`.
        #[arg(long)]
        owner_scope: Option<String>,
        /// Explicit state-key owner key, such as a repo path or `user:default`.
        #[arg(long)]
        owner_key: Option<String>,
        /// Resolve the state that applied at this Unix epoch.
        #[arg(long)]
        as_of_epoch: Option<i64>,
        /// Emit a single JSON object with stable fields for scripts and MCP parity.
        #[arg(long)]
        json: bool,
    },
    /// Search raw archive rows from the terminal.
    Raw {
        #[command(subcommand)]
        action: RawAction,
    },
    /// Inspect chronological observations and project timeline reports.
    Timeline {
        #[command(subcommand)]
        action: TimelineAction,
    },
    /// List or manually update tracked workstreams.
    Workstreams {
        #[command(subcommand)]
        action: WorkstreamAction,
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
    /// Public benchmark artifact commands.
    Bench {
        #[command(subcommand)]
        action: super::eval_types::BenchAction,
    },
    /// Run the golden retrieval evaluation dataset.
    Eval {
        /// Golden dataset path.
        #[arg(long, default_value = "eval/golden.json")]
        dataset: String,
        /// Number of results to evaluate per query.
        #[arg(long, short = 'k', default_value = "5")]
        k: usize,
        /// Emit the deterministic retrieval report as JSON.
        #[arg(long)]
        json: bool,
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
    /// Run sandboxed memory governance quality evaluation.
    EvalGovernance {
        /// Number of results to evaluate per query.
        #[arg(long, short = 'k', default_value = "5")]
        k: usize,
        /// Emit the evaluation report as JSON.
        #[arg(long)]
        json: bool,
    },
    #[command(name = "eval-extraction")]
    EvalExtraction(super::eval_types::EvalExtractionArgs),
    #[command(name = "eval-provider-comparison")]
    EvalProviderComparison(super::eval_types::EvalProviderComparisonArgs),
    #[command(name = "eval-graph-decision")]
    EvalGraphDecision(super::eval_types::EvalGraphDecisionArgs),
    #[command(name = "eval-associative-baseline")]
    EvalAssociativeBaseline(super::eval_types::EvalAssociativeBaselineArgs),
    #[command(name = "eval-capacity")]
    EvalCapacity(super::eval_types::EvalCapacityArgs),
    #[command(name = "eval-weight-grid")]
    EvalWeightGrid(super::eval_types::EvalWeightGridArgs),
    #[command(name = "eval-gates")]
    EvalGates(super::eval_types::EvalGatesArgs),
    #[command(name = "eval-coding-bench")]
    EvalCodingBench(super::eval_types::EvalCodingBenchArgs),
    /// Run local retrieval diagnostics.
    EvalLocal,
    /// Backfill entity records for existing memories.
    BackfillEntities,
    #[command(visible_alias = "reindex-embeddings")]
    BackfillEmbeddings {
        #[arg(long, default_value_t = 1000)]
        limit: i64,
        #[arg(long, default_value_t = 1000, help = "Rows per measured write batch")]
        batch_size: i64,
    },
    /// Encrypt the local database or migrate its key format.
    Encrypt {
        /// Migrate an existing legacy passphrase key file to raw-key format.
        #[arg(long)]
        rekey_raw: bool,
    },
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
        /// Memory AI profile name from [memory_ai.profiles].
        #[arg(long)]
        profile: Option<String>,
        /// Print what would be merged without writing to DB
        #[arg(long)]
        dry_run: bool,
    },
    /// Admin commands for database backup.
    Admin {
        #[command(subcommand)]
        action: AdminAction,
    },
    /// Import older backup rows, markdown mirrors, or project memory packs.
    Import {
        /// Project memory pack directory to validate and plan.
        #[arg(long)]
        pack: Option<PathBuf>,
        /// Plan a pack import without mutating the runtime store.
        #[arg(long)]
        dry_run: bool,
        #[command(subcommand)]
        action: Option<ImportAction>,
    },
    /// Batch-ingest Claude Code / Codex session transcripts into the raw archive.
    IngestSessions {
        /// Extra scan root as label=path (repeatable). Defaults always include
        /// ~/.claude/projects and ~/.codex/sessions.
        #[arg(long = "root")]
        roots: Vec<String>,
        /// Skip files last modified before this bound (Unix epoch or ISO8601 date/datetime).
        #[arg(long)]
        since: Option<String>,
        /// Emit a single JSON summary object for scripts.
        #[arg(long)]
        json: bool,
    },
    /// Export curated memories to a human-editable mirror.
    Export(ExportArgs),
}
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
    /// Replay legacy pending rows into captured_events/extraction_tasks.
    MigrateLegacy {
        /// Restrict rows to one project path.
        #[arg(long, short)]
        project: Option<String>,
        /// Host to use for rows stored with legacy host=unknown.
        #[arg(long)]
        host: Option<String>,
        /// Maximum pending rows to migrate.
        #[arg(long, short = 'n', default_value = "100")]
        limit: i64,
        /// Preview migration count without mutating rows.
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },
    /// List exhausted extraction event ranges.
    ListExtractionRanges {
        #[arg(long, short)]
        project: Option<String>,
        #[arg(long, short = 'n', default_value = "20")]
        limit: i64,
        #[arg(long)]
        json: bool,
    },
    /// Requeue exhausted extraction event ranges.
    RetryExtractionRanges {
        #[arg(long, short)]
        project: Option<String>,
        #[arg(long, short = 'n', default_value = "100")]
        limit: i64,
        #[arg(long)]
        dry_run: bool,
    },
    /// Quarantine exhausted extraction event ranges.
    QuarantineExtractionRanges {
        #[arg(long, short)]
        project: Option<String>,
        #[arg(long, short = 'n', default_value = "100")]
        limit: i64,
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(in crate::cli) enum MemoryGovernanceCliAction {
    Delete,
    Reject,
    Stale,
    AcknowledgePattern,
}
