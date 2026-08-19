# Legacy Observation Retirement Product Spec

Status: Current contract
Date: 2026-07-28

Tracking:
- Epic issue: #684
- Capture projection follow-up: #992
- Related contracts: `current-memory-contracts/` (anti-rewrite convergence,
  Refs #381/#383/#384)
- Related drain implementation: #943

## Problem

Two storage generations run side by side. The 2026-07-02 verification pass
(inventory in TECH.md) sharpened what "legacy" actually means here:

- `pending_observations` is a frozen queue input: no default-path writer,
  enqueue API, or claim API remains. Residual rows are consumed only by a
  bounded drain bridge that migrates their value into the current
  capture/extraction pipeline; this transitional consumer does not revive the
  legacy queue.
- `session_summaries` is a kept current table. After GH684-T7, Stop records a
  `SessionRollup` extraction task and no longer enqueues `JobType::Summary`,
  Compress, or Dream jobs directly. A new Summary enqueue fails closed; residual
  Summary jobs remain drain-only. The table stays because context, timeline,
  and user-context readers still load it.
- `observations` (+ `observations_fts`) turned out to be a live intermediate
  of the current extraction pipeline. GH684-T8 fixes the MCP/docs wording that
  previously advertised it as "legacy observations".

So the debt is one frozen drain-only surface, one duplicated writer chain, and
one mislabeled current surface — not a wholesale parallel pipeline.

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
- Residual legacy pending rows drain without starving current extraction:
  current work has priority, once workers run at most one 25-row batch, and
  daemons run at most one 25-row batch every 60 seconds.
- An empty or fully drained store persists `legacy_surface_state.state =
  exhausted` for `pending_observations`. Ordinary workers then skip the drain
  entirely until a residual auto-actionable row is reintroduced through the
  admin/test fixture path. The table itself stays until the guarded remem
  0.7.0 drop.
- Users can see legacy state: doctor reports legacy row counts, whether the
  automatic drain is halted, and whether legacy writes still occur.

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
never skips frozen, and each transition is observable in doctor. A
transactional drain that marks an existing frozen row migrated is a value
migration, not a restored legacy writer.

### Reads Move Before Writes Die

Consumers (timeline, context, MCP, REST) switch to ledger-backed sources
first, with equivalence evidence (fixtures comparing old vs new output).
Only then do legacy writers stop, so no user-visible feature regresses
during the window.

### Frozen Surfaces May Drain Without Revival

Once frozen, default APIs stop advertising truly legacy-only write/claim
surfaces. An ordinary worker may drain existing `pending_observations` into
the current pipeline under the bounded #943 contract, but it cannot enqueue or
claim new legacy work. The legacy Summary writer chain remains retired.
`observations` is different: it is reclassified as a current intermediate of
the capture pipeline, so MCP `source='observation'` remains an explicit
observation audit path after the wording is fixed. It is not deprecated or
removed by this contract.

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
- Doctor distinguishes rows eligible for the automatic drain from permanent
  or unknown-host archived rows that are `admin-required`, and gives the exact
  dry-run/apply recovery commands.
- When no residual auto-actionable row remains, doctor reports that the
  automatic drain is halted and that guarded table drop stays remem 0.7.0.
- After freeze, a legacy write triggers a doctor error, not a silent
  success.

### Safe Migration

As a user with years of legacy observations, upgrading does not lose
history: whatever still has value lands in the ledger or curated memories
with provenance, and I get a release-note warning before any drop.

Acceptance:

- Bulk migration commands are idempotent and report migrated/skipped counts;
  exact archived recovery never falls back or duplicates work and rejects a
  target whose state changed after preview.
- The automatic bridge runs only when current extraction has no ready work,
  migrates at most 25 oldest eligible rows per allowed interval, and never
  deletes a source row.
- Eligible rows have a known host and are pending, expired-processing, due
  transient failures, or controlled historical archived transient failures.
  Permanent and unknown-host rows never enter automatic recovery.
- Success atomically migrates value and clears old failure/archive state.
  Any replay error rolls back current-pipeline writes, uses exponential
  backoff capped at 900 seconds, and stops the batch. The bridge never guesses
  that a shared replay failure is row-local permanent.
- `remem pending recover-archived --id <id> [--host
  claude-code|codex-cli] --dry-run` previews one exact archived failed row;
  apply removes `--dry-run`, unknown host requires the option, success clears
  failure/archive only after atomic replay, and failure preserves the source.
- A process-level fault-injection test kills and restarts a worker around a
  durable failed backlog and proves the backlog drains to zero.
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
  workdir changes, non-interactive add/commit forms with an explicit message
  source (including ordinary `--fixup <commit>` autosquash commits), safe
  identity configuration, and the documented exact status suffix;
  editor-opening amend/reword fixups, environment prefixes, arbitrary Git
  configuration, help/viewer/editor paths, dry runs, interactive add modes,
  shell expansion, redirection, globbing, process substitution, or unquoted
  shell comments produce no evidence.
- Ordinary edits, Stop events, and a repository's baseline `HEAD` do not create
  commit links. A byte-bounded Codex transcript may prove multiple commits;
  one ambiguous call, malformed shell call, or call whose candidate metadata
  cannot be resolved is logged and skipped without erasing earlier proven
  calls. Relative workdirs are anchored to the Stop cwd, and an exact trailing
  `git status --short` is supported without accepting environment overrides,
  Git configuration, help viewers, or arbitrary trailing shell output. Codex
  success comes only from the wrapper status before `Final output:`;
  status-like command output cannot override a failed wrapper.
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
  On platforms without process-liveness probing, orphan claims use the same
  minimum-age gate instead of being treated as permanently live.
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

### Idempotent Legacy Event Projection

As a user, a hook retry cannot duplicate the compatibility `events` row for a
canonical captured event, and a legacy projection failure cannot leave the
canonical capture committed by itself.

Acceptance:

- Hook-originated `events` rows carry the exact `captured_events.id` that
  produced them; non-capture audit writers remain valid without that link.
- Canonical capture, extraction-task enqueue, Git evidence, and the legacy
  event projection commit or roll back together, including spill replay.
- Replaying the same canonical identity returns the existing legacy event only
  when its projected payload matches exactly. A payload mismatch is a visible
  error and never silently overwrites history.
- Cursor's success-to-failure precedence updates the one linked projection in
  the same transaction as the canonical failure marker. Upgrade-era unlinked
  history remains untouched.
- Focused tests prove retry produces one linked row and injected projection
  failure leaves neither a capture nor an extraction task behind.

## Rollout

1. Inventory + per-surface decisions (spec-only deliverable inside this
   contract; no code).
2. Doctor visibility: legacy row counts, last-write tracking.
3. Reader migration with equivalence fixtures; freeze writers.
4. Idle-only value migration through the bounded worker bridge plus exact
   archived admin recovery, followed by a deprecation announcement in doctor
   and release notes. After the store has no residual auto-actionable rows,
   persist `exhausted` and stop admitting the bridge. Guarded drop migrations
   remain remem 0.7.0 and still refuse to run while unmigrated valuable rows
   remain.

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
- Does MCP `get_observations` keep its name after legacy removal, or is the
  legacy source parameter retired with it?
