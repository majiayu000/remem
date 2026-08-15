# Legacy-Unverified Context Quarantine — Product Contract

Status: Current contract

Refs #1017.

Implementation status: Accepted and merged in PR #1019 on 2026-08-15. The
copied-production-database validation and the focused #1017 acceptance audit
are complete.

## Outcome

Historical curated rows remain recoverable, but only evidence-backed current rows may appear as CurrentTruth or default SessionStart context. G2 is a read-only projection: it creates no second store, rewrites no historical row, and adds no migration.

## Classification

The ordered classes are quarantined, expired, superseded, not_yet_valid, inactive, legacy_unverified, and current. Lifecycle exclusions win before proof checks. An active row is legacy_unverified when required provenance is absent or malformed, confidence is absent or below 0.80, validity start is absent, or a topic-keyed mutable decision/architecture/preference lacks a state-key identity. Generated provenance must be either an accepted, approved, or auto-promoted candidate whose evidence resolves completely to captured events, or direct memory evidence that resolves completely to captured events; syntactically valid dangling identifiers fail closed. Mutable state-key identifiers must resolve to an existing state key. Explicit direct user saves are accepted as user-authored proof; proven lesson writes are accepted through their lesson metadata. Those writer-proof arms do not require generated provenance, and direct-user writes do not require generated state identity. Unknown proof fails closed.

## Compatibility and recovery

Search is an inspection/recovery surface and may return excluded rows with visible classification and reason labels without changing pagination. Detail-by-id remains a recovery surface. Exclusion never changes stored status or content. Default SessionStart excludes the same rows before rendering and audits every considered exclusion. Current high-confidence rows retain deterministic ordering.

## Rollout boundary

Classification performs no network, LLM, or writes. A classification read failure is a context-load error, not an eligible fallback. G2 does not route production context through G3 CurrentTruth or mutate lifecycle state.

Validation used one consistent encrypted copy only; the live installed remem
runtime and database were not mutated. Search/detail recovery and default
SessionStart/CurrentTruth exclusion remain the quality-first behavior.
