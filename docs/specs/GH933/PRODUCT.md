# GH933 Product Spec — CurrentTruth Read-Side Projection (Phase A)

Refs #933.

## Problem

remem stores knowledge in several mature but disjoint objects (memories,
observations, user-context claims, memory/graph edges, captured events), each
with its own status vocabulary (`stale`, `suppressed`, `superseded`,
`archived`, `compressed`, `expired`, ...). Callers cannot answer one unified
question: *as of a given time, within a given scope, which claims does the
system currently consider true, which conflict, and what evidence supports
each conclusion?*

## Goal (Phase A only)

A read-only, deterministic projection over the existing tables:

- Versioned read DTOs: `EvidenceView`, `ClaimView`, `RelationView`,
  `CurrentTruthView`.
- Lifecycle split into three orthogonal dimensions
  (`PublicationState`, `ValidityState`, `RetentionState`) plus a separate
  visibility flag for policy suppression, with a deterministic mapping from
  every existing stored status value.
- A deterministic projection policy that resolves competing claims and always
  reports a selection reason. Unresolvable conflicts return `Contradicted`;
  insufficient evidence returns abstention (`Unknown`), never a silent pick.

## Non-Goals (explicitly out of Phase A)

- No Context Bundle integration (Phase B).
- No writer changes, no schema migrations, no new tables (Phase C decides).
- No LLM involvement in truth resolution; stored confidence numbers are never
  used as a truth score.

## User-Visible Behavior

Library-level API only in Phase A (`remem_ai::truth`). No CLI/HTTP surface
changes. Behavior contract:

1. Scope match first: a projection for project P / branch B never returns
   claims from another project; branch-tagged claims only match their branch.
2. `as_of` returns the truth as it stood at that time (based on stored
   temporal fields and relation timestamps).
3. Explicit supersedes relations beat pure recency.
4. Verified evidence (user-authored / tool output) beats model-generated.
5. Conflicts that cannot be safely resolved are surfaced as `Contradicted`
   with both sides attached.
6. No matching claims, or no claim with sufficient standing, yields an
   abstention result, not an invented answer.
7. Every result carries its canonical ref, evidence refs, and a
   `TruthSelectionReason`.

## Acceptance (Phase A slice of #933)

- [x] Versioned Evidence/Claim/Relation/CurrentTruth read DTOs.
- [x] Adapter mapping from memories, user-context claims, memory/graph edges,
      captured events; observations covered by the lifecycle mapping.
- [x] `as_of`, project, branch selectors.
- [x] Deterministic supersedes/refutes/supports resolution with reasons.
- [x] Unresolved conflict never silently folded.
- [x] Unit tests cover every existing stored status value per source table.
- [x] Golden fixtures: supersedes, dual-evidence support, contradiction,
      branch isolation, as-of history, scope isolation, abstention.

Deferred to Phase B/C: Context Bundle consumption, worktree/task selector,
writer convergence, generated-enrichment claim firewall at write time
(read side already never treats enrichment as a claim source).
