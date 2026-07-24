use serde::Serialize;

use crate::perf::PhaseTiming;

/// Closed set of reasons why the rerank stage did not publish a reranked
/// order. Every reason maps to a stable machine-readable token that is shared
/// by search explain, SessionStart diagnostics, logs, and doctor output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RerankDisabledReason {
    /// Rerank is explicitly disabled by configuration.
    Off,
    /// The current path has no usable normalized query.
    EmptyQuery,
    /// Rerank is enabled but the model manifest is not installed.
    ModelMissing,
    /// Manifest, file bytes, or hashes failed verification.
    ModelCorrupt,
    /// Local runtime failed to load the verified model.
    ModelLoadFailed,
    /// Scoring failed (including non-finite scores).
    InferenceFailed,
    /// The approved per-request deadline elapsed before completion.
    DeadlineExceeded,
    /// The caller cancelled the request before scores were published.
    Cancelled,
}

impl RerankDisabledReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::EmptyQuery => "empty_query",
            Self::ModelMissing => "model_missing",
            Self::ModelCorrupt => "model_corrupt",
            Self::ModelLoadFailed => "model_load_failed",
            Self::InferenceFailed => "inference_failed",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::Cancelled => "cancelled",
        }
    }

    /// Reasons that represent a fail-visible error state (as opposed to an
    /// intentional off/empty-query state).
    pub fn is_error(self) -> bool {
        !matches!(self, Self::Off | Self::EmptyQuery)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RerankStatus {
    Applied,
    NotApplied {
        disabled_reason: RerankDisabledReason,
    },
}

/// One eligible candidate entering the shared rerank stage. Candidates must
/// already have passed every project/owner/suppression/staleness rule of the
/// calling path; the stage never recalls or re-admits memories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RerankCandidate {
    pub id: i64,
    /// Stable 0-based rank in the caller's pre-rerank baseline order.
    pub baseline_rank: usize,
    /// Whether the memory carries the `verify-before-trust` source anchor.
    /// After a successful rerank these candidates are hard-partitioned behind
    /// all normal candidates.
    pub verify_before_trust: bool,
    /// Canonical bounded query-document projection text.
    pub document: String,
}

/// Result of one rerank stage invocation. `ordered_ids` is only meaningful
/// when `status == Applied`; on any failure the caller must keep the complete
/// pre-rerank baseline order.
#[derive(Debug, Clone, PartialEq)]
pub struct RerankOutcome {
    pub status: RerankStatus,
    pub ordered_ids: Vec<i64>,
    pub preset: Option<String>,
    pub model_manifest_sha256: Option<String>,
    pub input_count: usize,
    pub output_count: usize,
    pub top_n: usize,
    pub top_k: usize,
    pub timings: Vec<PhaseTiming>,
}

impl RerankOutcome {
    pub fn not_applied(reason: RerankDisabledReason) -> Self {
        Self {
            status: RerankStatus::NotApplied {
                disabled_reason: reason,
            },
            ordered_ids: vec![],
            preset: None,
            model_manifest_sha256: None,
            input_count: 0,
            output_count: 0,
            top_n: 0,
            top_k: 0,
            timings: vec![],
        }
    }

    pub fn applied(&self) -> bool {
        matches!(self.status, RerankStatus::Applied)
    }

    pub fn disabled_reason(&self) -> Option<RerankDisabledReason> {
        match self.status {
            RerankStatus::Applied => None,
            RerankStatus::NotApplied { disabled_reason } => Some(disabled_reason),
        }
    }

    pub fn to_explain(&self, requested: bool) -> RerankExplain {
        RerankExplain {
            requested,
            applied: self.applied(),
            preset: self.preset.clone(),
            model_manifest_sha256: self.model_manifest_sha256.clone(),
            top_n: self.top_n,
            top_k: self.top_k,
            input_count: self.input_count,
            output_count: self.output_count,
            disabled_reason: self.disabled_reason().map(|reason| reason.as_str().into()),
            timings: self.timings.clone(),
        }
    }
}

/// Serializable rerank diagnostics shared by search explain, service
/// responses, and SessionStart evidence. Contains no query or memory content.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RerankExplain {
    pub requested: bool,
    pub applied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_manifest_sha256: Option<String>,
    pub top_n: usize,
    pub top_k: usize,
    pub input_count: usize,
    pub output_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
    pub timings: Vec<PhaseTiming>,
}
