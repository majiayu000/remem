mod dry_run;
mod run;
mod state;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_convergence;
mod transition;
mod types;

pub(crate) use dry_run::dry_run_pending;
pub(crate) use run::run_migrations;
#[cfg(test)]
pub(crate) use types::MIGRATIONS;

pub(crate) fn latest_schema_version() -> i64 {
    types::MIGRATIONS
        .last()
        .map(|migration| migration.version)
        .unwrap_or(0)
}
