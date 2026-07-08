use std::path::PathBuf;

use clap::{Subcommand, ValueEnum};

use crate::memory::procedure::ProcedureExportFormat;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(in crate::cli) enum ProcedureExportFormatArg {
    ClaudeSkill,
    CodexPrompt,
    RunbookMd,
}

impl From<ProcedureExportFormatArg> for ProcedureExportFormat {
    fn from(value: ProcedureExportFormatArg) -> Self {
        match value {
            ProcedureExportFormatArg::ClaudeSkill => ProcedureExportFormat::ClaudeSkill,
            ProcedureExportFormatArg::CodexPrompt => ProcedureExportFormat::CodexPrompt,
            ProcedureExportFormatArg::RunbookMd => ProcedureExportFormat::RunbookMd,
        }
    }
}

#[derive(Subcommand)]
pub(in crate::cli) enum ProcedureAction {
    /// List promoted, active procedure memories and maturity signals.
    List {
        /// Restrict results to one project path.
        #[arg(long, short)]
        project: Option<String>,
        /// Maximum procedures to show.
        #[arg(long, short = 'n', default_value = "50")]
        limit: i64,
        /// Result offset for pagination.
        #[arg(long, default_value = "0")]
        offset: i64,
        /// Emit a single JSON object with stable fields for scripts.
        #[arg(long)]
        json: bool,
    },
    /// Export one promoted procedure into a reviewable draft artifact.
    Export {
        /// Procedure memory id to export.
        memory_id: i64,
        /// Draft output format.
        #[arg(long, value_enum)]
        format: ProcedureExportFormatArg,
        /// Output directory for generated drafts.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Overwrite only when the existing target is the unchanged generated draft.
        #[arg(long)]
        overwrite_generated: bool,
    },
}
