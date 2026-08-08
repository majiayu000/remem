use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub(in crate::cli) enum DoctorAction {
    /// Inspect the read-only CurrentTruth projection for one project scope.
    Truth(DoctorTruthArgs),
}

#[derive(Args)]
pub(in crate::cli) struct DoctorTruthArgs {
    /// Exact project key to inspect. Defaults to the project derived from --cwd.
    #[arg(long, conflicts_with = "cwd")]
    pub(in crate::cli) project: Option<String>,
    /// Working directory used to derive the canonical project key.
    #[arg(long)]
    pub(in crate::cli) cwd: Option<String>,
    /// Restrict the projection to branch-neutral claims plus this exact branch.
    #[arg(long)]
    pub(in crate::cli) branch: Option<String>,
    /// Evaluate truth at this Unix epoch instead of the current time.
    #[arg(long)]
    pub(in crate::cli) as_of_epoch: Option<i64>,
    /// Restrict the projection to one exact topic/claim key.
    #[arg(long)]
    pub(in crate::cli) subject: Option<String>,
}
