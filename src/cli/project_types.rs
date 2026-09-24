use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub(in crate::cli) enum ProjectAction {
    /// Manage audited project identity aliases.
    Alias {
        #[command(subcommand)]
        action: ProjectAliasAction,
    },
}

#[derive(Subcommand)]
pub(in crate::cli) enum ProjectAliasAction {
    /// Preview or apply a worktree-to-project alias.
    Add {
        alias_path: PathBuf,
        #[arg(long)]
        canonical: PathBuf,
        #[arg(long)]
        actor: String,
        #[arg(long)]
        reason: String,
        /// Record the alias after reviewing the preview.
        #[arg(long)]
        apply: bool,
    },
    /// List active aliases with their actor and reason.
    List,
    /// Preview or apply an audited alias revocation.
    Revoke {
        alias_path: PathBuf,
        #[arg(long)]
        actor: String,
        #[arg(long)]
        reason: String,
        #[arg(long)]
        apply: bool,
    },
}
