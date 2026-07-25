use clap::Args;

#[derive(Args, Debug)]
pub(super) struct WorkerArgs {
    /// Process ready work once and exit.
    #[arg(long)]
    pub(super) once: bool,

    /// Recover and process exactly one extraction replay range.
    #[arg(
        long,
        value_parser = clap::value_parser!(i64).range(1..),
        requires_all = ["once", "acknowledge_quarantine", "include_archived", "profile"]
    )]
    pub(super) replay_range_id: Option<i64>,

    /// Explicitly acknowledge that the exact range is quarantined.
    #[arg(long, requires = "replay_range_id")]
    pub(super) acknowledge_quarantine: bool,

    /// Explicitly acknowledge that the exact range is archived.
    #[arg(long, requires = "replay_range_id")]
    pub(super) include_archived: bool,

    /// Memory AI profile used only for the exact replay task.
    #[arg(long, requires = "replay_range_id")]
    pub(super) profile: Option<String>,
}
