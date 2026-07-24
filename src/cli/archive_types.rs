use clap::{Args, Subcommand};
use std::path::PathBuf;

#[derive(Args)]
pub(in crate::cli) struct ExportArgs {
    /// Export as one markdown file per curated memory.
    #[arg(long)]
    pub(in crate::cli) markdown: bool,
    /// Output directory for the markdown mirror.
    #[arg(long)]
    pub(in crate::cli) output: Option<PathBuf>,
    /// Output directory for a deterministic project memory pack.
    #[arg(long)]
    pub(in crate::cli) pack: Option<PathBuf>,
    /// Project path to export. Defaults to the current working directory.
    #[arg(long, short)]
    pub(in crate::cli) project: Option<String>,
    /// Include stale and archived memories.
    #[arg(long)]
    pub(in crate::cli) include_inactive: bool,
    /// Maximum memories to export.
    #[arg(long, default_value = "10000")]
    pub(in crate::cli) limit: i64,
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
    /// One-way, read-only import of Codex CLI native rollout-summary memories
    /// into the candidate review queue. Records are untrusted external
    /// content: they only land in pending_review or quarantined, never
    /// directly in active memories, and the source tree is never modified.
    CodexMemories {
        /// Override the source directory (defaults to
        /// `~/.codex/memories/rollout_summaries`). Intended for tests/PoC.
        #[arg(long)]
        source: Option<PathBuf>,
        /// Plan the import without writing to the remem database.
        #[arg(long)]
        dry_run: bool,
        /// Plan digest from a prior --dry-run. Required for apply; the apply
        /// is rejected if current planning no longer matches this digest.
        #[arg(long)]
        expect_plan_digest: Option<String>,
    },
    /// Rebuild curated memories from a markdown mirror produced by `remem export --markdown`.
    #[command(visible_alias = "reindex-markdown")]
    Markdown {
        /// Markdown file or directory containing exported `.md` memory files.
        #[arg(long)]
        source: PathBuf,
        /// Skip malformed markdown files instead of failing the import.
        #[arg(long)]
        best_effort: bool,
    },
}
