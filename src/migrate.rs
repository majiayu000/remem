mod dry_run;
mod run;
mod schema_drift;
mod state;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_compression_provenance;
#[cfg(test)]
mod tests_convergence;
#[cfg(test)]
mod tests_schema;
#[cfg(test)]
mod tests_schema_drift;
mod transition;
mod types;

pub(crate) use dry_run::dry_run_pending;
pub(crate) use run::ensure_schema_current;
pub use run::run_migrations;
pub(crate) use schema_drift::validate_schema_invariants;
#[cfg(test)]
pub(crate) use types::MIGRATIONS;

pub(crate) fn latest_schema_version() -> i64 {
    types::MIGRATIONS
        .last()
        .map(|migration| migration.version)
        .unwrap_or(0)
}
