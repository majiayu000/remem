mod audit;
mod merge;
mod mutate;
mod plan;
mod preference_cluster;
mod refs;

pub use audit::{audit_scope, AuditItem, DuplicateCluster, ScopeAuditReport, ScopeAuditRequest};
pub use merge::{merge_preferences, MergePreferencesRequest, MergePreferencesResult};
pub use mutate::{
    archive_objects, reroute_objects, ArchiveRequest, ObjectMutation, OwnerSnapshot,
    RerouteRequest, ScopeMutationResult, TargetProjectUpdate,
};
pub use plan::{
    apply_memory_cleanup_plan, build_preference_cleanup_plan, MemoryCleanupApplyResult,
    MemoryCleanupGroup, MemoryCleanupPlan, MemoryCleanupRowSnapshot, CLEANUP_PLANNER_VERSION,
};
pub use refs::{memory_refs_from_ids, parse_object_refs, ObjectRef, ScopeObjectKind};

#[cfg(test)]
mod tests;
