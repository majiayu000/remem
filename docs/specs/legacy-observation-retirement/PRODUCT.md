# Legacy Observation Retirement Product Spec

Status: Current contract
Date: 2026-07-02

Tracking:
- Epic issue: #684
- Related contracts: `current-memory-contracts/` (anti-rewrite convergence,
  Refs #381/#383/#384)

## Problem

Two storage generations run side by side. The 2026-07-02 verification pass
(inventory in TECH.md) sharpened what "legacy" actually means here:

- `pending_observations` is a dead queue: no default-path writer remains,
  and the dogfood database shows zero rows in every state. Its claim/lease
  machinery ships in the binary with no production caller.
- `session_summaries` is dual-written on every session end: the current
  `SessionRollup` task and the pre-v006 summarize job chain
  (`JobType::Summary` -> `finalize_summarize`) both fire from the same Stop
  hook, unconditionally. The legacy chain also accounts for thousands of
  failed jobs and unattributed AI spend on the dogfood database.
- `observations` (+ `observations_fts`) turned out to be a live intermediate
  of the current extraction pipeline. GH684-T8 fixes the MCP/docs wording that
  previously advertised it as "legacy observations".

So the debt is one dead surface, one duplicated writer chain, and one
mislabeled current surface — not a wholesale parallel pipeline.

Costs of the dual path:

- every retrieval/ranking/staleness feature pays a dual-read tax and grows
  edge cases (which source wins, which FTS index is authoritative);
- audits flagged compounding dual-schema failure modes;
- new contributors must learn two pipelines to change one behavior.

## Goals

- One explicit, committed decision per legacy surface: retire (migrate then
  drop) or freeze (read-only, labeled, with a removal date).
- A complete writer/reader inventory so the decision is made on facts, not
  memory.
- Zero data loss: rows carrying unique value are migrated before any drop,
  behind a deprecation window.
- Users can see legacy state: doctor reports legacy row counts and whether
  legacy writes still occur.

## Non-Goals

- No second rewrite. `current-memory-contracts/` explicitly forbids it; this
  spec converges surfaces onto the pipeline that already won.
- No behavior change to the capture-ledger path itself.
- No silent dropping of tables in a routine migration. Every drop ships with
  its own migration, release note, and doctor pre-check.
- Timeline and context features do not lose capability; they change data
  source only when the replacement is proven equivalent.

## Product Principles

### Freeze Before Remove

Each legacy surface passes through explicit states: live -> frozen
(no new writes, reads labeled legacy) -> migrated -> removed. A surface
never skips frozen, and each transition is observable in doctor.

### Reads Move Before Writes Die

Consumers (timeline, context, MCP, REST) switch to ledger-backed sources
first, with equivalence evidence (fixtures comparing old vs new output).
Only then do legacy writers stop, so no user-visible feature regresses
during the window.

### Legacy-Only Surfaces Are Opt-In After Freeze

Once frozen, default surfaces stop advertising surfaces that are truly
legacy-only, such as `pending_observations` and the legacy Summary writer
chain. `observations` is different: it is reclassified as a current
intermediate of the capture pipeline, so MCP `source='observation'` remains
an explicit observation audit path after the wording is fixed. It is not
deprecated or removed by this contract.

## User Stories

### Inventory And Decision

As a maintainer, I can read one document listing every writer and reader of
the four legacy surfaces with file references, and the retire-vs-freeze
decision for each.

Acceptance:

- The TECH spec contains the inventory table.
- Each surface has a recorded decision, rationale, and target release for
  each state transition.

### Observable Legacy State

As a user, `remem doctor` tells me whether my database still has legacy
rows, whether anything still writes them, and what will happen to them.

Acceptance:

- Doctor reports row counts for `pending_observations`, `observations`,
  `session_summaries`, and last-write timestamps.
- After freeze, a legacy write triggers a doctor error, not a silent
  success.

### Safe Migration

As a user with years of legacy observations, upgrading does not lose
history: whatever still has value lands in the ledger or curated memories
with provenance, and I get a release-note warning before any drop.

Acceptance:

- Migration commands are idempotent and report migrated/skipped counts.
- A drop migration refuses to run while unmigrated valuable rows remain.

## Rollout

1. Inventory + per-surface decisions (spec-only deliverable inside this
   contract; no code).
2. Doctor visibility: legacy row counts, last-write tracking.
3. Reader migration with equivalence fixtures; freeze writers.
4. Value migration + deprecation window + drop migrations.

Each code phase ships independently with focused tests plus:

```bash
cargo fmt --check
cargo check
cargo test
```

## Open Questions

- Do `session_summaries` rows retain standalone value after session rollups
  land in the ledger, or is their value fully represented by promoted
  memories plus `raw_messages`?
- How long is the deprecation window (one minor release vs a time window)?
- Does MCP `get_observations` keep its name after legacy removal, or is the
  legacy source parameter retired with it?
