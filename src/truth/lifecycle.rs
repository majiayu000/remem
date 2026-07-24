//! Deterministic mapping from existing stored status vocabularies to the
//! three orthogonal lifecycle dimensions (GH933 Phase A).
//!
//! Every mapping is total: unknown legacy strings fail closed to
//! `(Candidate, Unknown, Live, Visible)` so they can never become current
//! truth, and never panic on legacy data.

use super::types::{Lifecycle, PublicationState, RetentionState, ValidityState, Visibility};

const fn lifecycle(
    publication: PublicationState,
    validity: ValidityState,
    retention: RetentionState,
    visibility: Visibility,
) -> Lifecycle {
    Lifecycle {
        publication,
        validity,
        retention,
        visibility,
    }
}

/// Fail-closed default for unrecognized stored statuses.
const UNKNOWN_STATUS: Lifecycle = lifecycle(
    PublicationState::Candidate,
    ValidityState::Unknown,
    RetentionState::Live,
    Visibility::Visible,
);

/// `memories.status` -> lifecycle.
///
/// Writers today produce: `active`, `stale`, `superseded`, `archived`,
/// `deleted`, `rejected` (governance).
pub fn memory_lifecycle(status: &str) -> Lifecycle {
    match status {
        "active" => lifecycle(
            PublicationState::Active,
            ValidityState::Current,
            RetentionState::Live,
            Visibility::Visible,
        ),
        "stale" => lifecycle(
            PublicationState::Active,
            ValidityState::Stale,
            RetentionState::Live,
            Visibility::Visible,
        ),
        "superseded" => lifecycle(
            PublicationState::Active,
            ValidityState::Superseded,
            RetentionState::Live,
            Visibility::Visible,
        ),
        // Archived is a retention decision, not knowledge invalidity.
        "archived" => lifecycle(
            PublicationState::Active,
            ValidityState::Current,
            RetentionState::Archived,
            Visibility::Visible,
        ),
        "deleted" => lifecycle(
            PublicationState::Active,
            ValidityState::Unknown,
            RetentionState::Deleted,
            Visibility::Visible,
        ),
        // Review-rejected content never counts as published knowledge.
        "rejected" => lifecycle(
            PublicationState::Candidate,
            ValidityState::Unknown,
            RetentionState::Deleted,
            Visibility::Visible,
        ),
        _ => UNKNOWN_STATUS,
    }
}

/// `observations.status` -> lifecycle.
///
/// Writers today produce: `active`, `stale`, `compressed`.
pub fn observation_lifecycle(status: &str) -> Lifecycle {
    match status {
        "active" => lifecycle(
            PublicationState::Active,
            ValidityState::Current,
            RetentionState::Live,
            Visibility::Visible,
        ),
        "stale" => lifecycle(
            PublicationState::Active,
            ValidityState::Stale,
            RetentionState::Live,
            Visibility::Visible,
        ),
        // Compressed is a storage/projection state, not knowledge validity.
        "compressed" => lifecycle(
            PublicationState::Active,
            ValidityState::Current,
            RetentionState::Archived,
            Visibility::Visible,
        ),
        _ => UNKNOWN_STATUS,
    }
}

/// `user_context_claims.status` -> lifecycle.
///
/// Schema CHECK allows: `active`, `pending_review`, `stale`, `superseded`,
/// `suppressed`, `rejected`, `deleted`.
pub fn user_claim_lifecycle(status: &str) -> Lifecycle {
    match status {
        "active" => lifecycle(
            PublicationState::Active,
            ValidityState::Current,
            RetentionState::Live,
            Visibility::Visible,
        ),
        "pending_review" => lifecycle(
            PublicationState::Candidate,
            ValidityState::Unknown,
            RetentionState::Live,
            Visibility::Visible,
        ),
        "stale" => lifecycle(
            PublicationState::Active,
            ValidityState::Stale,
            RetentionState::Live,
            Visibility::Visible,
        ),
        "superseded" => lifecycle(
            PublicationState::Active,
            ValidityState::Superseded,
            RetentionState::Live,
            Visibility::Visible,
        ),
        // Suppression is a visibility/policy decision, not falsity.
        "suppressed" => lifecycle(
            PublicationState::Active,
            ValidityState::Current,
            RetentionState::Live,
            Visibility::Suppressed,
        ),
        "rejected" => lifecycle(
            PublicationState::Candidate,
            ValidityState::Unknown,
            RetentionState::Deleted,
            Visibility::Visible,
        ),
        "deleted" => lifecycle(
            PublicationState::Active,
            ValidityState::Unknown,
            RetentionState::Deleted,
            Visibility::Visible,
        ),
        _ => UNKNOWN_STATUS,
    }
}

/// `memory_candidates.review_status` -> lifecycle.
///
/// Writers today produce: `pending_review`, `quarantined`, `deferred`,
/// `failed`, `discarded`, `auto_promoted`, `approved`, `accepted`, `edited`.
/// Candidates are never claim sources in Phase A; this mapping exists so the
/// review queue shares the same lifecycle language.
pub fn candidate_lifecycle(review_status: &str) -> Lifecycle {
    match review_status {
        "pending_review" | "deferred" | "failed" => UNKNOWN_STATUS,
        "quarantined" => lifecycle(
            PublicationState::Candidate,
            ValidityState::Unknown,
            RetentionState::Live,
            Visibility::Suppressed,
        ),
        "discarded" => lifecycle(
            PublicationState::Candidate,
            ValidityState::Unknown,
            RetentionState::Deleted,
            Visibility::Visible,
        ),
        "auto_promoted" => lifecycle(
            PublicationState::Active,
            ValidityState::Current,
            RetentionState::Live,
            Visibility::Visible,
        ),
        "approved" | "accepted" | "edited" => lifecycle(
            PublicationState::Reviewed,
            ValidityState::Current,
            RetentionState::Live,
            Visibility::Visible,
        ),
        _ => UNKNOWN_STATUS,
    }
}

/// Derived expiry: a validity window that ended before the reference time
/// overrides the stored validity. Expiry is computed, never stored.
pub fn apply_expiry(
    base: Lifecycle,
    valid_to_epoch: Option<i64>,
    reference_epoch: i64,
) -> Lifecycle {
    match valid_to_epoch {
        Some(end) if end <= reference_epoch => Lifecycle {
            validity: ValidityState::Expired,
            ..base
        },
        _ => base,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dims(l: Lifecycle) -> (PublicationState, ValidityState, RetentionState, Visibility) {
        (l.publication, l.validity, l.retention, l.visibility)
    }

    #[test]
    fn memory_statuses_map_deterministically() {
        use PublicationState as P;
        use RetentionState as R;
        use ValidityState as V;
        use Visibility as Vis;
        let cases = [
            ("active", (P::Active, V::Current, R::Live, Vis::Visible)),
            ("stale", (P::Active, V::Stale, R::Live, Vis::Visible)),
            (
                "superseded",
                (P::Active, V::Superseded, R::Live, Vis::Visible),
            ),
            (
                "archived",
                (P::Active, V::Current, R::Archived, Vis::Visible),
            ),
            ("deleted", (P::Active, V::Unknown, R::Deleted, Vis::Visible)),
            (
                "rejected",
                (P::Candidate, V::Unknown, R::Deleted, Vis::Visible),
            ),
        ];
        for (status, expected) in cases {
            assert_eq!(dims(memory_lifecycle(status)), expected, "status {status}");
        }
    }

    #[test]
    fn observation_statuses_map_deterministically() {
        use PublicationState as P;
        use RetentionState as R;
        use ValidityState as V;
        use Visibility as Vis;
        let cases = [
            ("active", (P::Active, V::Current, R::Live, Vis::Visible)),
            ("stale", (P::Active, V::Stale, R::Live, Vis::Visible)),
            (
                "compressed",
                (P::Active, V::Current, R::Archived, Vis::Visible),
            ),
        ];
        for (status, expected) in cases {
            assert_eq!(
                dims(observation_lifecycle(status)),
                expected,
                "status {status}"
            );
        }
    }

    #[test]
    fn user_claim_statuses_map_deterministically() {
        use PublicationState as P;
        use RetentionState as R;
        use ValidityState as V;
        use Visibility as Vis;
        let cases = [
            ("active", (P::Active, V::Current, R::Live, Vis::Visible)),
            (
                "pending_review",
                (P::Candidate, V::Unknown, R::Live, Vis::Visible),
            ),
            ("stale", (P::Active, V::Stale, R::Live, Vis::Visible)),
            (
                "superseded",
                (P::Active, V::Superseded, R::Live, Vis::Visible),
            ),
            (
                "suppressed",
                (P::Active, V::Current, R::Live, Vis::Suppressed),
            ),
            (
                "rejected",
                (P::Candidate, V::Unknown, R::Deleted, Vis::Visible),
            ),
            ("deleted", (P::Active, V::Unknown, R::Deleted, Vis::Visible)),
        ];
        for (status, expected) in cases {
            assert_eq!(
                dims(user_claim_lifecycle(status)),
                expected,
                "status {status}"
            );
        }
    }

    #[test]
    fn candidate_review_statuses_map_deterministically() {
        use PublicationState as P;
        use RetentionState as R;
        use ValidityState as V;
        use Visibility as Vis;
        let cases = [
            (
                "pending_review",
                (P::Candidate, V::Unknown, R::Live, Vis::Visible),
            ),
            (
                "deferred",
                (P::Candidate, V::Unknown, R::Live, Vis::Visible),
            ),
            ("failed", (P::Candidate, V::Unknown, R::Live, Vis::Visible)),
            (
                "quarantined",
                (P::Candidate, V::Unknown, R::Live, Vis::Suppressed),
            ),
            (
                "discarded",
                (P::Candidate, V::Unknown, R::Deleted, Vis::Visible),
            ),
            (
                "auto_promoted",
                (P::Active, V::Current, R::Live, Vis::Visible),
            ),
            ("approved", (P::Reviewed, V::Current, R::Live, Vis::Visible)),
            ("accepted", (P::Reviewed, V::Current, R::Live, Vis::Visible)),
            ("edited", (P::Reviewed, V::Current, R::Live, Vis::Visible)),
        ];
        for (status, expected) in cases {
            assert_eq!(
                dims(candidate_lifecycle(status)),
                expected,
                "review_status {status}"
            );
        }
    }

    #[test]
    fn unknown_statuses_fail_closed() {
        for mapped in [
            memory_lifecycle("weird_legacy"),
            observation_lifecycle(""),
            user_claim_lifecycle("invalidated"),
            candidate_lifecycle("deprecated"),
        ] {
            assert_eq!(mapped.publication, PublicationState::Candidate);
            assert_eq!(mapped.validity, ValidityState::Unknown);
        }
    }

    #[test]
    fn expiry_is_derived_from_validity_window() {
        let base = memory_lifecycle("active");
        assert_eq!(
            apply_expiry(base, Some(100), 100).validity,
            ValidityState::Expired
        );
        assert_eq!(
            apply_expiry(base, Some(101), 100).validity,
            ValidityState::Current
        );
        assert_eq!(
            apply_expiry(base, None, 100).validity,
            ValidityState::Current
        );
    }
}
