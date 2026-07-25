# Tech Spec

## Linked Issue

GH-678

## Product Spec

Link to `product.md`.

## Accepted Contract

The authoritative technical contract is
`docs/specs/project-memory-pack/TECH.md`.

This SpecRail packet reflects the existing #678 contract and keeps
implementation behind the normal SpecRail readiness and spec-approval gates.

## Codebase Context

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| Memory store | `src/memory/store/`, `src/memory/lifecycle/` | Memories carry type, content, status, owner, state key, confidence, and validity metadata. | Export/import must preserve project memory identity without row ids. |
| Suppression/governance | `src/memory/suppression.rs`, `src/memory/governance.rs` | Local policy hides or invalidates rows without hard deletion. | Import must never resurrect suppressed or invalidated local decisions. |
| Markdown archive | `src/cli/actions/markdown_archive/` | Human-readable export/import-like workflows exist but are not pack contracts. | Reuse parsing/serialization lessons while keeping pack format distinct. |
| Redaction | `src/adapter/redaction.rs` | Capture-time redaction detects sensitive values. | Export must re-run redaction and fail loudly on hits. |
| Trust classes | `docs/specs/memory-poisoning-defense/` | #672 defines the trust vocabulary. | Active import depends on `pack` trust class and scan support. |
| Doctor/why | `src/doctor/`, `src/memory/current_state.rs` | Surfaces provenance and runtime health. | Pack origin must stay auditable. |

## Design Rules

- `memories.jsonl` is canonical; `INDEX.md` is derived and never parsed.
- Serialization uses stable field order, LF line endings, and no export-run
  timestamp in canonical bytes.
- Local row ids, raw session ids, and machine-local evidence ids are not
  exported.
- Import operates in a transaction; dry-run uses the same planner and rolls
  back.
- Pack rows never overwrite or resurrect local inactive decisions.

## Pack Format

```
<pack-dir>/
  pack.json
  memories.jsonl
  INDEX.md
```

`pack.json` stores format version, project identity, exporter remem version,
memory count, and digest. `memories.jsonl` stores one active project memory per
line in canonical order. `INDEX.md` groups the same rows for human review.

## Export Design

1. Resolve project and repo owner identity.
2. Query active repo-owned startup memories only; exclude stale, expired,
   suppressed, tool/domain/user/workstream/session-owned, and non-startup rows.
3. Re-run redaction against content and abort with offending row ids on any
   match.
4. Serialize canonical JSONL sorted by `(memory_type, state_key | content_hash)`.
5. Emit manifest digest and derived index.

## Import Design

Inside one transaction:

1. Validate manifest version and digest.
2. Remap project identity to the importing checkout.
3. Match by owner, memory type, and state key where present; otherwise content
   hash.
4. Dedup identical active rows.
5. Skip suppressed or inactive local identities.
6. Route conflicting state-key rows to review; local wins by default.
7. After #672 lands, insert safe rows with source trust class `pack` after
   instruction-pattern scanning.

`--dry-run` uses the same planner against a savepoint and reports the same
categories without mutating the store.

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| Deterministic export | pack serializer | unchanged export byte equality |
| Round trip | export/import fixture | export -> import -> export byte equality |
| Merge safety | import planner + active import | suppressed/inactive resurrection tests |
| Trust class | active import | `pack` trust-class and quarantine tests |
| Redaction gate | export | seeded secret blocks export |
| Auditability | doctor/why | pack origin visible in reports |

## Risks

- Security: packs are third-party input; active import depends on #672 scan and
  `pack` trust class.
- Compatibility: format versioning must be explicit from v1.
- Data loss: import must be transactionally all-or-reportable and never
  silently skip without a report row.

## Test Plan

- [x] Export determinism fixture.
- [x] Round-trip export/import fixture.
- [x] Import planner dedup, conflict, suppression, and invalidation tests.
- [x] Redaction failure test.
- [x] Active import trust-class tests after #672 dependency lands.
- [x] README walkthrough smoke.
- [x] `cargo fmt --check`, `cargo check`, focused tests, and `cargo test`
      before merge readiness.

## Rollback Plan

Because pack import is explicit, disabling the command stops new imports.
Additive provenance fields may remain. Imported rows remain ordinary memories
with pack origin visible for governance or cleanup.
