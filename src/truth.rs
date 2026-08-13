//! CurrentTruth read-side projection (GH933, Phase A).
//!
//! A read-only, deterministic projection that answers: as of a given time,
//! within a given scope, which claims does the system currently consider
//! true, which conflict, and what evidence supports each conclusion?
//!
//! Phase A is an adapter over existing tables (`memories`, `memory_edges`,
//! `graph_edges`, `captured_events`, `user_context_claims`). It never
//! writes, never migrates, and never lets model-generated enrichment or
//! stored confidence numbers decide truth. Unresolvable conflicts surface as
//! `Contradicted`; insufficient evidence surfaces as an abstention.
//!
//! Contract: `docs/specs/GH933/TECH.md`.

mod adapter;
mod inventory;
mod lifecycle;
mod projection;
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
mod types;
mod visibility;

pub use adapter::{load_memory_claim_groups, load_user_claim_groups};
pub use inventory::{build_memory_visibility_inventory, MemoryVisibilityInventory};
pub use lifecycle::{
    candidate_lifecycle, memory_lifecycle, observation_lifecycle, user_claim_lifecycle,
};
pub use projection::{project_current_truth, project_user_claim_truth};
pub use types::{
    ClaimRelationKind, ClaimSource, ClaimView, CurrentTruthProjection, CurrentTruthView,
    EvidenceKind, EvidenceTrust, EvidenceView, Lifecycle, PublicationState, RelationView,
    RetentionState, TruthQuery, TruthSelectionReason, ValidityState, Visibility,
    TRUTH_PROJECTION_VERSION,
};
pub use visibility::{
    admit_for_current_context, classify_memory, MemoryVisibility, MemoryVisibilityClass,
    MemoryVisibilityReason, CURRENT_CONFIDENCE_FLOOR,
};
