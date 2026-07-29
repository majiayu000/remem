#[cfg(test)]
mod auto_migration_tests;
mod migration;
mod mutate;
mod query;
mod recovery;
#[cfg(test)]
mod tests;
mod types;

pub use migration::{
    auto_migrate_actionable_legacy_pending, count_admin_required_archived_legacy_pending,
    count_legacy_migration_candidates, count_recoverable_archived_legacy_pending,
    migrate_legacy_pending, AutoLegacyMigrationOutcome, LegacyPendingMigration,
};
pub use mutate::{purge_failed, retry_failed};
pub(crate) use query::list_admin_required_archived_legacy_pending;
pub use query::{count_failed_purge_candidates, count_failed_retry_candidates, list_failed};
pub use recovery::{
    preview_archived_legacy_pending_recovery, recover_archived_legacy_pending,
    ArchivedLegacyPendingRecovery, ArchivedLegacyPendingRecoveryPreview,
};
pub(crate) use types::AdminRequiredArchivedLegacyPendingRow;
pub use types::FailedPendingRow;
