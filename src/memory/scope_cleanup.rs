mod audit;
mod merge;
mod mutate;
mod refs;

pub use audit::{audit_scope, AuditItem, DuplicateCluster, ScopeAuditReport, ScopeAuditRequest};
pub use merge::{merge_preferences, MergePreferencesRequest, MergePreferencesResult};
pub use mutate::{
    archive_objects, reroute_objects, ArchiveRequest, ObjectMutation, OwnerSnapshot,
    RerouteRequest, ScopeMutationResult, TargetProjectUpdate,
};
pub use refs::{memory_refs_from_ids, parse_object_refs, ObjectRef, ScopeObjectKind};

#[cfg(test)]
mod tests;
