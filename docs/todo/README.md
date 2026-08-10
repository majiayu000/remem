# Remem Memory Governance Execution Tracker

Status: Active
Owner: remem maintainers
Started: 2026-08-08
Last verified: 2026-08-09

This is the canonical execution tracker for repairing memory quality and
governance. It does not replace the current contracts in
`docs/specs/current-memory-contracts/`, `docs/specs/review-queue-throughput/`,
`docs/specs/candidate-auto-promotion/`, or `docs/specs/GH932/`–`GH934/`.

## Outcome

Only evidence-backed, current, in-scope, non-conflicting claims may be emitted
as current context. Historical and uncertain material remains recoverable, but
it must be explicitly labelled and kept out of current truth.

```text
captured event (immutable)
  -> candidate
  -> review / deterministic promotion gate
  -> canonical memory + evidence + lifecycle
  -> CurrentTruth projection
  -> retrieval / SessionStart / app

governance -> audited lifecycle transitions; never rewrites source evidence
```

## Non-negotiable data safety rules

1. Do not bulk-delete curated memories. Use `stale`, `superseded`,
   `quarantined`, or `search_only` unless privacy/security requires deletion.
2. Every data-changing batch starts with a snapshot-bound plan and dry-run.
   Apply must use the same input digest, explicit actor, reason, and bounded
   batch size.
3. Model output is a candidate judgment, never mutation authority. A model may
   classify or explain rows, but deterministic validation and a human-approved
   plan decide what is applied.
4. Do not send secrets, raw transcripts, credentials, or unrelated project
   data to a hosted model. Redact before inference and retain only hashes or
   canonical evidence references in audit artifacts.
5. Do not mutate the live database with a binary older than the source and
   schema contract being used to prepare the plan.
6. One memory ID has exactly one final verdict per audit run. Conflicting
   verdicts block the run.
7. Every applied batch must be replayable, auditable, and reversible by an
   explicit inverse lifecycle transition where the schema permits it.

## Model-assisted data processing policy

OpenCode with DeepSeek V4 Flash may be used for high-volume candidate
classification after deterministic pre-filtering. Each run must record:

- exact provider/model identifier reported by OpenCode;
- prompt/protocol version and input schema version;
- source database snapshot digest and selected ID digest;
- per-row verdict, confidence, reason codes, and evidence references;
- parse failures, abstentions, duplicate IDs, and disagreement counts;
- the deterministic validator result.

Allowed verdicts are `keep`, `stale`, `split`, `merge_candidate`,
`quarantine`, and `abstain`. The model cannot emit `delete` or directly call a
mutation command. Confidence below the configured threshold becomes
`abstain`. Compound memories must be split into atomic claim candidates before
any child claim can become current.

## Verified baseline

Read-only snapshot taken with `/opt/homebrew/bin/remem 0.6.1 (schema v69)` on
2026-08-08. The current governance branch is staged as `0.6.66` with
migrations through schema v82. The live database remains mutation-blocked until
that runtime is deliberately installed; the alias migration was verified on an
encrypted copied database before its integration renumbering from v81 to v82.

| Signal | Baseline | Interpretation |
|---|---:|---|
| Curated memories | 80,628 | Historical corpus; not a count of verified truth |
| Pending review | 12,525 | Queue is operationally backlogged |
| Review inflow, 7d | 676 | New work continues to arrive |
| Review resolved, 7d | 0 | Queue cannot converge |
| Median pending age | 1,571,112 s (~18.2 d) | Health/SLO failure |
| Maximum pending age | 6,415,005 s (~74.2 d) | Long-lived deadlock |
| Summary candidates blocked by low source trust | 6,406 | Largest explicit blocked class |
| Pending candidates with null block reason | 1,703 | Diagnostic contract gap |
| Raw archive messages | 137,566 | Immutable source volume |
| Raw archive insert errors | 763 | Requires separate loss audit |
| Extraction failures | 644 | Requires failure-lifecycle triage |
| Retryable replay ranges | 527 | Recoverable work exists |

Observed historical project aliases include both
`/Users/lifcc/Desktop/code/AI/tool/remem` and
`/Users/lifcc/Desktop/code/AI/tools/remem`. The former no longer exists, so
historical ownership cannot be repaired by current-path lookup alone.

## Execution order

Tasks are intentionally sequential. A task may move to `done` only when its
acceptance checks and evidence are recorded here.

### G0 — Establish the safety boundary and reproducible baseline

Status: Done

- [x] Capture aggregate queue, archive, and extraction health without reading
  memory bodies.
- [x] Confirm governance commands support dry-run and explicit destructive
  confirmation.
- [x] Record the live-runtime/source mismatch.
- [x] Add a repository-owned read-only baseline command that emits the stable
  governance metrics and a digest, without memory content.
- [x] Verify the current source binary against a copied database through schema
  v80; do not migrate the live database in this task.
- [x] Add a checked-in JSON schema for audit runs and deterministic validator
  fixtures.

Acceptance:

- baseline collection is content-free and produces byte-stable JSON for a
  fixed database snapshot;
- no command in G0 modifies `~/.remem/remem.db`;
- copied-database verification reports the exact binary version, schema
  version, snapshot digest, and migration result.

Baseline usage:

```bash
# Live read-only collection. Only allowlisted aggregates are emitted.
python3 scripts/governance/memory_baseline.py --remem /path/to/remem

# Deterministic re-render of an already captured status snapshot.
python3 scripts/governance/memory_baseline.py --status-json status.json

# Contract tests.
python3 -m unittest scripts/governance/test_memory_baseline.py
```

### G1 — Canonical project identity and alias reconciliation

Status: Done

- [x] Define a durable project identity using canonical worktree identity and
  repository remote evidence; path is an alias, not the primary key.
- [x] Inventory historical aliases and classify exact, moved, deleted, and
  ambiguous paths.
- [x] Add dry-run alias reconciliation with collision and cross-repository
  guards.
- [x] Apply the alias registry to a copied database first, then compare corpus
  and per-project counts before any live application.
- [x] Ensure capture, candidates, state keys, retrieval, status, and review
  filters resolve through the same identity contract.

Acceptance:

- `/AI/tool/remem` and `/AI/tools/remem` resolve to one owner only when Git
  evidence proves they are the same repository;
- ambiguous aliases abstain;
- project totals are conserved and no record changes repository ownership.

### G2 — Quarantine legacy-unverified data from current context

Status: Not started; depends on G1

- [ ] Define an explicit legacy trust/visibility classification without adding
  a second durable memory store.
- [ ] Identify active rows missing required provenance, confidence, validity,
  or mutable-state identity.
- [ ] Keep those rows searchable with visible labels, but exclude them from
  CurrentTruth and default SessionStart injection.
- [ ] Add injection audit reasons for every exclusion.

Acceptance:

- no `legacy_unverified`, quarantined, expired, or superseded row is emitted as
  current truth;
- search/detail can still recover the row and explain why it was excluded;
- current high-confidence behavior does not regress in deterministic evals.

### G3 — Make CurrentTruth the production current-state boundary

Status: Not started; depends on G2

- [ ] Route current-state candidates through `remem::truth` rather than mapping
  every loaded core memory directly to a current channel.
- [ ] Preserve immutable canonical references and populate evidence/projection
  references in Context Bundle items.
- [ ] Make explicit conflicts abstain; equal-trust conflicts do not use
  newest-wins.
- [ ] Wire the compiled Context Bundle into the production SessionStart path
  behind an audited shadow comparison before activation.

Acceptance:

- one mutable owner/state/scope slot emits at most one current claim;
- unresolved contradiction emits an abstention with both claim references;
- direct raw-active bypass is covered by a failing regression test;
- shadow comparison explains every changed inclusion/exclusion.

### G4 — Enforce atomic claim and evidence contracts on new writes

Status: Not started; depends on G3

- [ ] One candidate contains one falsifiable claim; compound candidates are
  rejected or split with `derived_from` links.
- [ ] Require claim-class-specific evidence, trust, scope, and validity fields.
- [ ] Keep proposals/discussion explicitly non-current.
- [ ] Update mutable state atomically: insert replacement, link supersession,
  close prior validity, and move the state-key pointer.

Acceptance:

- code facts identify commit/file/symbol when applicable;
- runtime/PR/CI facts have observation time and TTL;
- preferences require explicit user evidence and stable state identity;
- proposal and model-summary fixtures cannot auto-promote to CurrentTruth.

### G5 — Restore review-queue throughput without weakening gates

Status: Not started; depends on G4

- [ ] Explain and eliminate the 1,703 null block reasons.
- [ ] Segment structurally unpromotable, low-value, recoverable, and true human
  judgment classes.
- [ ] Add priority by retrieval exposure and risk, not FIFO alone.
- [ ] Use model assistance only to propose verdicts for bounded review batches.
- [ ] Apply batch outcomes with preview, confirmation, actor, reason, run ID,
  and per-row audit metadata.

Acceptance:

- `resolved_7d >= inflow_7d` for two consecutive weeks;
- pending median age is under seven days;
- no promotion threshold is loosened merely to reduce backlog;
- sampled post-review precision meets the documented threshold.

### G6 — Backfill the historical corpus in bounded tiers

Status: Not started; depends on G5

Process tiers in this order:

1. retrieval-exposed preferences, decisions, and mutable current state;
2. evidence-backed active core memories;
3. active long-form memories with recoverable source evidence;
4. short summaries and model-only legacy rows, defaulting to search-only;
5. unknown or conflicting material, defaulting to abstain.

Every batch is copied-DB first, at most the configured batch cap, and followed
by invariants, retrieval diff, sampled review, and a rollback rehearsal.

Acceptance:

- 100% evidence/state-key coverage for mutable claims eligible for
  CurrentTruth;
- unknown remains unknown; missing evidence is never synthesized;
- totals reconcile across keep/stale/split/quarantine/abstain outcomes;
- hard deletes are separately enumerated and justified.

### G7 — Close the loop with SLOs and regression gates

Status: Not started; depends on G6

- [ ] Add governance health to `status`, `doctor`, API, and evaluation
  artifacts using the same runtime semantics.
- [ ] Track CurrentTruth conflicts, abstentions, evidence coverage, staleness,
  retrieval exposure, citation matches, queue age, and ingest failures.
- [ ] Add a release gate for current-truth safety and a capacity curve for
  corpus growth.
- [ ] Document operational recovery and the final live-data apply record.

Acceptance:

- all current-context items are explainable from capture evidence to injection;
- new raw insert errors stay at zero;
- health failures are user-visible and fail closed where correctness requires;
- the final report is hash-bound to the database snapshot and code revision.

## Run log

| Date | Task | Result | Evidence |
|---|---|---|---|
| 2026-08-08 | G0 baseline | Started; read-only aggregate baseline captured. Live mutation blocked because installed runtime is 0.6.1/schema v69 while source is 0.6.59/schema v80. | `remem status --json`, `remem govern --help`, `remem memory cleanup --help`, source commit `d458bfebcac3db98a7db220843016cee614b7309` |
| 2026-08-08 | G0 content-free baseline tool | Done; allowlisted aggregate projection, canonical SHA-256 digest, fail-closed parsing, and leakage regression tests added. Current baseline digest: `430681e8294bdd1e316625ac1ef840d64288b992f1b31dad1ec22520d7d6995c`. | `scripts/governance/memory_baseline.py`; four unit tests pass |
| 2026-08-09 | G0 copied-database migration rehearsal | Done; encrypted online backup migrated from v69 to v80 using source v0.6.59. Curated memories remained 80,628 and pending review remained 12,525. Live runtime remained v0.6.1/schema v69. Temporary encrypted snapshot and key symlink were removed after verification. | source commit `d458bfebcac3db98a7db220843016cee614b7309`; before SHA-256 `d8be6e6aba9585a6079895d6991ca2ac5f9e5487f0da6ffefd7f88139cea7b50`; after SHA-256 `61efa9325dc5e2e61061429b708375ded5c1a8c87033771a3fbe6bf91675380e` |
| 2026-08-09 | G0 audit-run contract | Done; schema and fail-closed validator forbid mutation authority/delete verdicts, require redaction and snapshot binding, reject duplicate IDs, and force low-confidence judgments to abstain. | `docs/todo/memory-audit-run.schema.json`; `scripts/governance/validate_memory_audit.py`; five validator tests pass |
| 2026-08-09 | G1 project alias inventory | Done; content-free schema inventory found 889 observed values: 223 exact, 52 moved, 281 missing, 333 non-path. Requiring the destination to be an existing canonical `projects` row produced 36 proof-backed alias proposals and blocked 270 ownership paths lacking unique proof. `/AI/tool/remem` maps to `/AI/tools/remem` because both stored commits resolve in the live repository; the plan covers 39,616 ownership references while leaving 7,226 context-evidence rows unchanged. | inventory/report SHA-256 `2e1e03b63328bd2c06268feb720f7d7b43749fe01349c6e102a6d19de2c03635`; `examples/project_alias_inventory.rs`; seven tests pass |
| 2026-08-09 | G1 copied-database alias apply | Done; migrated the encrypted copy with the then-v81 alias migration, previewed and inserted 36 active aliases, then re-previewed with 0 inserts and 36 unchanged. The integration branch now carries the identical migration as v82 after main claimed v81. Curated memories remained 80,628, pending review remained 12,525, and the old remem path retained exactly 39,616 ownership references plus 7,226 context-evidence rows. No historical row was rewritten and the live database remained on v0.6.1/schema v69. | `src/migrations/v082_project_identity_aliases.sql`; `examples/project_alias_apply.rs`; apply result `inserted=36`; idempotence result `unchanged=36` |
| 2026-08-09 | G1 shared identity resolver | Done; new capture/state-key writes canonicalize known aliases while memory/context retrieval, candidate review, current-state, and status expand or aggregate active aliases. The final copied-DB check with pre-integration source v0.6.61/schema v81 still reported 80,628 memories, 12,525 pending review, and an idempotent preview of 0 inserts/36 unchanged. Integration now uses source v0.6.66/schema v82. | `src/project_alias.rs`; schema v82 invariants; Rust: 3,666 passed/1 ignored plus all integration suites passed; Node: 50 passed; pre-integration version sync 0.6.61 passed |

## Next action

Begin G2 by defining the `legacy_unverified` trust/visibility projection and a
content-free inventory of rows that cannot qualify for CurrentTruth. Keep this
stage read-only: no live lifecycle mutation until the projection, exclusion
reasons, and copied-database retrieval diff are deterministic.
