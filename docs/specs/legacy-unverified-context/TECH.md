# Legacy-Unverified Context Quarantine — Technical Contract

Status: Current contract

Refs #1017, #1029.

The module separates fact from policy. `truth::classify_memory` and
`truth::classify_memories` report the classification and never consult rollout
state, so search, detail, inventory, and `doctor truth` always see the true
class. `truth::admit_for_current_context` and
`truth::admit_many_for_current_context` are the admission decisions for live
current-context readers and are the only functions that honor the gate mode
below. A live reader must call an `admit_*` function; calling `classify_*` to
gate injection bypasses the rollout control.

`admit_many_for_current_context` classifies in chunked statements of at most 900
ids so a context load costs a few statements rather than one per candidate. Every
requested id appears in the result; ids with no `memories` row stay at the
fail-closed `legacy_unverified_row_missing` default so an absent key can never be
read as an admission. Callers keep a per-id fallback for the case where the batch
statement itself fails.

## Enforcement rollout and rollback

Enforcement changes which memories reach live context. On the validated copied
production database below, `current=41` of `80828` rows, so enabling enforcement
there reduces the current-context candidate set by more than three orders of
magnitude. That is the intended classification result, not a defect, but it is
large enough that it must be measurable and reversible without a downgrade.

`REMEM_CURRENT_CONTEXT_GATE` controls the gate:

- unset, or any unrecognized value, enforces; this is the default and the
  fail-closed direction;
- `shadow` keeps classifying and reporting, but still admits rows that are
  excluded *only* for `legacy_unverified` reasons.

Shadow mode never relaxes `status_quarantined`, `validity_expired`,
`status_superseded`, `validity_not_yet_started`, or `status_inactive`. Those are
pre-existing security and correctness boundaries and are outside this rollout.
`MemoryVisibility::admitted_by_shadow_mode` identifies a row that enforcement
would have dropped, so shadow runs can report the injection delta rather than
silently behaving like the old build.

truth::classify_memory is the sole deterministic classifier. It reads existing memories columns and resolves referenced memory_candidates, captured_events, memory_lessons, and memory_state_keys without mutation. It returns a class, exact reason enum, and current_context_eligible. Generated provenance is valid only through an accepted, approved, or auto-promoted candidate with a non-empty captured-event set, or a non-empty direct memory evidence set, with every referenced event present. If both references are present, both must resolve. Direct-user and proven-lesson writer arms remain explicit compatibility proof. Precedence is quarantine, expiry, supersession, not-yet-valid, inactive status, provenance, confidence, validity start, mutable identity, current.

Stable reasons are status_quarantined, validity_expired, status_superseded, validity_not_yet_started, status_inactive, legacy_unverified_provenance_missing, legacy_unverified_provenance_malformed, legacy_unverified_confidence_missing, legacy_unverified_confidence_below_floor, legacy_unverified_validity_start_missing, legacy_unverified_mutable_state_identity_missing, and legacy_unverified_row_missing.

CurrentTruth excludes a non-eligible memory before resolution. SessionStart partitions loaded memories, lessons, and preferences before relevance/rendering and records classifier reasons through existing preselection audit drops. Search never filters by current-context eligibility; CLI, REST, and MCP annotate returned rows from the shared projection without changing retrieval limits or pagination. REST and MCP detail annotate recovered rows too.

Inventory may contain only schema/runtime/snapshot binding, class/reason counts, and a digest over canonical aggregate JSON. No inventory apply path exists. No schema, migration, row rewrite, G3 routing, LLM call, or lifecycle mutation is permitted.

Copied-production-database validation used source/runtime `0.6.67`, migrated
the encrypted copy from logical schema 81 to 82, and used fixed
`as_of_epoch=1786372000`. Two complete inventory runs were byte-identical:
`snapshot_memory_count=80828`, with `current=41`, `expired=152`,
`inactive=50`, and `legacy_unverified=80585`. Legacy reasons were
`provenance_malformed=66480` and `provenance_missing=14105`; the canonical
inventory digest was
`e8207ae4287bf08e2cac7138c41499afd163773152ae16f6bcf2d75c6f0bcfd6`.
Validation used the consistent encrypted copy only; the live installed runtime
and database were not mutated.
