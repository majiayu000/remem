use clap::Subcommand;

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
}
