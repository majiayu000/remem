//! Versioned read DTOs for the CurrentTruth projection (GH933 Phase A).

use serde::Serialize;

/// Version stamp carried by every projection payload. Bump when the DTO
/// shape or the resolution policy changes observable output.
pub const TRUTH_PROJECTION_VERSION: u32 = 1;

/// How a claim entered the published knowledge base.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicationState {
    Candidate,
    Reviewed,
    Active,
}

/// Whether the claim is currently considered to hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidityState {
    Current,
    Superseded,
    Contradicted,
    Stale,
    Expired,
    Unknown,
}

/// Storage/retention posture. `Archived` does not mean false.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RetentionState {
    Live,
    Archived,
    Deleted,
}

/// Policy visibility. `Suppressed` is a visibility decision, not falsity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Visible,
    Suppressed,
}

/// Three orthogonal lifecycle dimensions plus policy visibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Lifecycle {
    pub publication: PublicationState,
    pub validity: ValidityState,
    pub retention: RetentionState,
    pub visibility: Visibility,
}

/// Which canonical table a claim was projected from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimSource {
    Memory,
    UserContextClaim,
}

/// What kind of record backs a piece of evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    /// Immutable row in the `captured_events` ledger.
    CapturedEvent,
    /// Opaque source reference recorded on a user-context claim.
    SourceRef,
    /// Claim-level trust floor derived from `memories.source_trust_class`.
    SourceTrustClass,
}

/// Deterministic trust tier used by the resolution policy.
/// Ordering matters: `Verified > ModelGenerated > Untrusted`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceTrust {
    Untrusted,
    ModelGenerated,
    Verified,
}

/// Immutable evidence reference attached to a claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvidenceView {
    /// Stable reference, e.g. `captured_event:42`.
    pub evidence_ref: String,
    pub kind: EvidenceKind,
    /// Human-readable pointer back to the source (event type/role, source
    /// ref string, or trust class name).
    pub source_ref: String,
    pub observed_at_epoch: Option<i64>,
    pub trust: EvidenceTrust,
}

/// A claim projected from an existing canonical row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClaimView {
    /// Stable reference to the canonical row, e.g. `memory:7`.
    pub canonical_ref: String,
    pub source: ClaimSource,
    /// Grouping key competing claims share (topic key or claim key).
    pub subject_key: String,
    pub statement: String,
    /// Scope owner: project path for memories, owner key for user claims.
    pub scope: String,
    pub branch: Option<String>,
    pub lifecycle: Lifecycle,
    pub valid_from_epoch: Option<i64>,
    pub valid_to_epoch: Option<i64>,
    pub created_at_epoch: i64,
    pub updated_at_epoch: i64,
    pub evidence: Vec<EvidenceView>,
}

/// Typed relation between two claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimRelationKind {
    Supports,
    Refutes,
    Supersedes,
    DerivedFrom,
    AppliesTo,
}

/// Relation projected from `memory_edges`, trusted `graph_edges`, or
/// `user_context_claims.supersedes_claim_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RelationView {
    /// Stable reference, e.g. `memory_edge:3`.
    pub relation_ref: String,
    pub kind: ClaimRelationKind,
    pub from_ref: String,
    pub to_ref: String,
    pub created_at_epoch: i64,
    pub valid_from_epoch: Option<i64>,
    pub valid_to_epoch: Option<i64>,
}

/// Why the projection selected (or refused to select) a truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TruthSelectionReason {
    /// Only one eligible claim survived filtering.
    OnlySurvivingClaim,
    /// An explicit supersedes relation removed the competitors.
    ExplicitSupersedes,
    /// A strictly better evidence-trust tier decided between survivors.
    VerifiedEvidencePreferred,
    /// Equal trust tiers; the newest update won.
    MostRecent,
    /// A refutes relation or an unbreakable tie: no silent fold.
    UnresolvedConflict,
    /// No eligible claim remained: abstain instead of guessing.
    InsufficientEvidence,
}

/// One resolved subject: either a current truth, a surfaced contradiction,
/// or an abstention.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CurrentTruthView {
    pub subject_key: String,
    /// Winning claim; absent for contradictions and abstentions.
    pub claim: Option<ClaimView>,
    pub validity: ValidityState,
    pub evidence: Vec<EvidenceView>,
    pub supporting_relations: Vec<RelationView>,
    pub contradicting_relations: Vec<RelationView>,
    /// All claims that competed but were not selected (canonical refs).
    pub rejected: Vec<String>,
    /// Contradicting claims surfaced verbatim when validity is Contradicted.
    pub conflicting_claims: Vec<ClaimView>,
    pub selected_reason: TruthSelectionReason,
}

/// Read-only query selector for a projection run.
#[derive(Debug, Clone, Default)]
pub struct TruthQuery {
    pub project: String,
    pub branch: Option<String>,
    /// Reference time; `None` means "now".
    pub as_of_epoch: Option<i64>,
    /// Optional subject filter (topic key / claim key).
    pub subject_key: Option<String>,
}

/// Full projection output for one scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CurrentTruthProjection {
    pub projection_version: u32,
    pub project: String,
    pub branch: Option<String>,
    pub as_of_epoch: Option<i64>,
    pub truths: Vec<CurrentTruthView>,
}
