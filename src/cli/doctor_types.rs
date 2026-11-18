use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub(in crate::cli) enum DoctorAction {
    /// Inspect the read-only CurrentTruth projection for one project scope.
    Truth(DoctorTruthArgs),
    /// Trace why a phrase or source session did not become usable context.
    Memory(DoctorMemoryArgs),
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
    /// Restrict to one subject: a memory topic key, a full CurrentTruth subject key
    /// (`state:...`), or a user-claim key (`type:key` also works).
    #[arg(long)]
    pub(in crate::cli) subject: Option<String>,
}

#[derive(Args)]
pub(in crate::cli) struct DoctorMemoryArgs {
    /// Original phrase to discover (no match means unknown, not absent capture).
    #[arg(required_unless_present = "session_id", conflicts_with = "session_id")]
    pub(in crate::cli) query: Option<String>,
    /// Exact stored project key from raw sessions. Defaults to --cwd's project.
    #[arg(long, conflicts_with = "cwd")]
    pub(in crate::cli) project: Option<String>,
    #[arg(long)]
    pub(in crate::cli) cwd: Option<String>,
    /// Original source session ID; copy its host and source root unchanged.
    #[arg(long, requires_all = ["host", "source_root"])]
    pub(in crate::cli) session_id: Option<String>,
    #[arg(long)]
    pub(in crate::cli) host: Option<String>,
    #[arg(long, requires = "session_id")]
    pub(in crate::cli) source_root: Option<String>,
    /// Inspect this exact destination context run instead of the latest recorded run.
    #[arg(long)]
    pub(in crate::cli) injection_run_id: Option<String>,
}
