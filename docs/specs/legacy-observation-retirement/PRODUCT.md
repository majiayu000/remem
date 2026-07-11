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

### Durable Commit Traceability

As a user, a commit shown by `remem why` or the commit lookup tools is linked
to my coding session only when remem captured real Git evidence for that event.
The link must survive delayed processing and spill replay without being changed
to whatever `HEAD` happens to be later.

Acceptance:

- A successful explicit, non-quiet `git commit` result proves the SHA only when
  the command's standard Git summary contains it; trusted capture resolves
  metadata for that exact SHA before the event is written or spilled, and
  stores the evidence atomically with the capture event. Explicit quiet commit
  commands remain eligible for ordinary event capture but produce no commit
  evidence or link. Success requires a numeric zero exit status or a Claude
  payload explicitly identified as the success-only `PostToolUse` event;
  an explicit failure event always wins over contradictory response fields,
  while unknown status and failure events preserve capture without commit
  evidence. Evidence command parsing is fail-closed: it accepts only literal
  workdir changes,
  non-interactive add/commit forms with an explicit message source, safe
  identity configuration, and the documented exact status suffix; environment
  prefixes, arbitrary Git configuration, help/viewer/editor paths, dry runs,
  interactive add modes, shell expansion, redirection, globbing, and process
  substitution, or unquoted shell comments produce no evidence.
- Ordinary edits, Stop events, and a repository's baseline `HEAD` do not create
  commit links. A byte-bounded Codex transcript may prove multiple commits;
  one ambiguous call or one call whose candidate metadata cannot be resolved
  is logged and skipped without erasing earlier proven calls. Relative workdirs
  are anchored to the Stop cwd, and an exact trailing `git status --short` is
  supported without accepting environment overrides, Git configuration, help
  viewers, or arbitrary trailing shell output. Codex success comes only from
  the wrapper status before `Final output:`; status-like command output cannot
  override a failed wrapper.
- Deterministic linking uses the exact claimed event range and durable
  `session_row_id`; it does not depend on an LLM result or a synthetic
  observation-session prefix.
- Every distinct commit in a range is linked, while no evidence produces no
  link. Retries and later ranges do not duplicate links.
- If idempotent replay recovers evidence only after the original extraction
  cursor passed its event, a bounded link-only task consumes that evidence
  without rerunning model extraction, summaries, or their side effects.
  Same-identity Stop spill retries use one deterministic evidence event and
  the same link-only path. Legacy capture-spill rows without an `event_id`
  receive stable, occurrence-distinct identities, so byte-identical historical
  rows do not collapse and a failed replay keeps the identity assigned to it.
- Missing or ambiguous commit proof never drops the surrounding capture.
  Evidence that was durably captured but cannot be linked remains a visible
  extraction failure instead of a successful no-op.

### Bounded Rollup Evidence

As a user, a transcript-backed Stop capture produces a summary from the actual
conversation text captured at that Stop boundary rather than from transcript
path metadata alone.

Acceptance:

- Selected transcript paths use the widest boundary covered by the claimed
  event range and never read bytes appended after that boundary.
- User/assistant transcript messages enter the rollup prompt as bounded,
  deterministic, redacted, XML-escaped data anchored to a covered Stop event;
  candidate support and persisted retries consume the same bounded slice.
- Exact text already represented by a captured event is not repeated, and a
  legacy missing boundary may use captured conversational events only. Without
  that fallback it fails permanently; a missing, malformed, or unusable
  required bounded snapshot fails before a metadata-only summary can persist.
- Successful raw ingest and the exact-range evidence slice are checkpointed so
  remaining side effects can retry after the source transcript disappears.
- Per-Stop citation facts and the original assistant-message hash are persisted
  separately from the lossy prompt slice, so per-message or global prompt
  eviction cannot change citation usage during a source-free retry. Distinct
  Stop boundaries on one repeated path remain distinct citation evidence.

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
