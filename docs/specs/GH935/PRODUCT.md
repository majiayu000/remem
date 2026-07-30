# Cross-Host Continuity Benchmark — Product Contract

Status: Current contract; v1 infrastructure shipped, completion unimplemented
Issue: #935 (refs #849, #852, #385)

## Current Truth

PR #937 shipped `cross-host-v1` infrastructure under `eval/cross-host/`:

- a versioned charter and task/run schemas;
- 12 `skeleton_todo` tasks in each direction;
- an artifact leak scanner and offline dry-run validator.

There are still no executable fixtures, real-host runner, live benchmark runs,
sanitized result bundle, statistical report, or public cross-host conclusion.
The charter remains `infrastructure_only_no_runs`. This contract defines the
remaining completion work; it is not outcome evidence and does not authorize a
live run.

Root-level `specs/GH*/` packets are historical planning evidence after the
SpecRail workflow retirement in PR #965. This directory is the normative GH935
contract.

## Product Question

Can history produced in Claude Code improve a new, isolated Codex continuation
task, and can Codex history improve a new Claude Code task? How does remem
compare with no source history and a maintained exported handoff, without
hiding directional failures, stale-memory harm, scope leaks, or operating cost?

## Goal

Complete a reproducible bidirectional suite:

```text
Claude Code -> remem -> Codex
Codex -> remem -> Claude Code
```

For each direction, the source host creates history before the target task is
revealed. Every target condition receives the same fixture revision, task
prompt, scorer, model/profile configuration, and source-episode identity.
Only the declared memory surface differs.

The primary outcome is hidden-test `resolved_rate`. Reports also include
failure rates, recall/use attribution, stale-memory harm, project leakage,
source-session leakage, tokens, wall time, turns, and exported-file maintenance
cost.

## Comparison Contract

The four primary conditions remain:

| Condition | Allowed source-history surface | Claim role |
|---|---|---|
| `no_memory` | None. | Negative control. |
| `target_host_native` | Only memory created by the target host in its fresh target environment. It must not ingest the source seal, source transcript, source-native files, or a projection of them. | No-cross-host-transfer control. It is not evidence that remem beats a native cross-host bridge. |
| `exported_file` | A target-blind, sanitized export generated from the source episodes and delivered through one versioned host-neutral context envelope. | Claim-bearing manual-handoff baseline; generation and maintenance cost count. |
| `remem_shared` | The source host's automatic capture, extraction, review/promotion, and the target host's normal production retrieval path. | Claim-bearing treatment. |

`target_host_native` may be empty in a fresh target HOME. That is intentional:
it proves source history does not appear in an unrelated native store. Reports
must not relabel it as a populated native-memory comparison or claim that remem
beats native cross-host transfer.

The native-import diagnostic compares
`remem_without_host_native_import` with
`remem_with_host_native_import`. In both arms, "host native" means the
**source host's** native-memory snapshot. The target host never receives a raw
source seal. The runner first derives `native_neutral_base_v1` from the full
production store without changing the primary `remem_shared` evidence. That
base has no `claude_native`/`codex_native` candidate or transitive provenance.
Both arms start from byte-identical neutral-base clones and derive independent
raw-free target projections. The with arm alone imports the same sealed
source-native snapshot. Every claim-bearing native record must preserve the
immutable chain `import_attempt_id -> native_record_id -> inserted candidate_id
-> promoted review outcome -> memory_id`, with `promoted = true` and
`memory.source_candidate_id = candidate_id`. Duplicate, quarantine, promotion
noop, pre-existing-memory reuse, or lineage mismatch makes the pair
`INSUFFICIENT`.

Other diagnostic conditions may remain for debugging, but they never enter a
primary comparative claim.

## Completion Invariants

### 1. Tasks and Matrix

1. The executable `cross-host-v2` set contains 24 ready tasks: 12 per
   direction, covering
   the existing 12 categories in each direction.
2. A task becomes `ready` only when it has a deterministic fixture, at least
   two chronological source episodes, hidden tests, non-empty score commands,
   allowed/forbidden paths, gold facts, and an empty TODO list. Each
   `stale_superseded_decision` task also registers at least one deterministic,
   query-relevant stale/superseded challenge, its production state, and its
   expected pre-filter match assertion for every run. Every task also
   pre-registers target-invisible `causal_oracle_v1` rules covering each
   scorer-recognized wrong-action class with a unique memory/state hash,
   deterministic artifact matcher, and exactly one target-blind proof method
   with its parser/intervention and outcome table.
3. The complete primary matrix is exactly
   `24 tasks * 4 conditions * 3 runs = 288` unique tuples.
4. The complete source-native import diagnostic is exactly
   `24 tasks * 2 conditions * 3 runs = 144` unique tuples.
5. Missing, duplicate, substituted, schema-invalid, or unverified tuples make
   the comparative verdict `INSUFFICIENT`; they are never filled with zeroes.
6. The canonical plan fixes a counterbalanced order for the four primary and
   two required native-import conditions across 36 source seals per direction
   (72 total). Each condition appears exactly six times in every serial
   position per direction. The plan hash binds the schedule; realized order
   and timestamps are recorded, and any deviation invalidates the affected
   comparison.

### 2. One Source Episode, Many Target Conditions

For each `(direction, task_id, run_index)`, the source episode sequence runs
once. After automatic extraction reaches a terminal state, the runner seals:

- source transcript/tool-event and Git evidence hashes;
- the canonical project identity and fixture revision;
- a quiesced, content-addressed full source `REMEM_DATA_DIR` snapshot and
  sorted data-only manifest; the source SQLCipher key remains in a separate
  runner-private secret root;
- a deterministic logical target-transfer projection derived from that
  snapshot, plus its independently rekeyed SQLCipher ciphertext hash and
  non-secret projection key ID,
  preserving the current database schema and production state flags while
  containing the curated rows needed to exercise production candidate
  filtering—including registered stale/superseded challenge rows—and no source
  transcript/session/capture rows or archives;
- a sealed `native_neutral_base_v1` hash and proof that source-native
  candidates and their transitive provenance closure are absent;
- the source-host native-memory snapshot, when present;
- executable, model, profile, schema, and migration versions.

Every dependent condition uses that exact source-episode seal. Each
`remem_shared` target receives a fresh byte-identical private clone of the
sealed target-transfer projection; the full source store is never target
visible. Re-running a source episode for only one condition, regenerating
either store, or accepting a mismatched derivation/hash invalidates the pair.
All initial clones of one projection are byte-identical and use one
projection-scoped key distinct from the source key. A plan-hashed
`projection_secret_channel_v1` starts the trusted remem sidecar outside the host
sandbox and provisions the key over a private inherited pipe. Host hook/MCP
configuration contains only a keyless authenticated client endpoint; the host
cannot read the sidecar environment, argv, process metadata, or key bytes.
Ciphertext hashes are checked before any SQLite open, preflight uses a
disposable clone, and each actual target clone records its pre-open hash before
entering the mutable-runtime phase.

The source and target phases use separate HOME/config/session roots but reuse
the same run-scoped canonical absolute Git workspace path sequentially. This
preserves production project identity while keeping host sessions isolated.
A same-name decoy repository must use a different canonical path and project
identity.

### 3. Condition Isolation

- `no_memory` receives no memory/context surface.
- `target_host_native` starts from a fresh target HOME and receives no
  source-derived payload.
- `exported_file` is generated after the first source episode, updated after
  every later source episode, and frozen before the target task is revealed.
  Its plan-hashed `maintained_export_v1` protocol fixes exact prompt bytes,
  exporter/model/profile/sampling identity, canonical evidence projection,
  generate-versus-incremental-update inputs, envelope schema, stable-key and
  conflict/abstention rules, and per-cycle/cumulative call, token, size, time,
  and cost caps before any live episode. Generation and every maintenance
  cycle record those inputs, outputs, costs, and budget results. The exporter
  runs in an exporter-only HOME/config/session with remem hooks, automatic
  capture, and host-native persistence disabled. It receives read-only
  allowlisted source evidence, writes only the envelope and its usage log, and
  must leave the source store, source-native state, and canonical workspace
  unchanged during each exporter window. Each immutable cycle is exactly one
  of `committed` (one atomic schema-valid commit), `failed` (zero commits), or
  `not_started_after_prior_failure`. It can never regenerate, select another
  output, or fall back to a prior envelope. The first non-retryable failure
  terminally disables later export cycles and records typed failure instead of
  a final/freeze hash. A whitelisted pre-prompt transient aborts the whole
  source attempt; another non-security/non-integrity failure makes
  `exported_file` an `ordinary_failure` while other conditions continue.
- `remem_shared` uses the real automatic capture-to-retrieval path. Its primary
  review policy is fixed as `automatic_only_v1`: only candidates activated by
  the shipped automatic promotion policy before the target task is known enter
  the transfer projection. Pending/quarantined candidates remain inactive;
  manual approve/edit/reject actions are forbidden and the report records
  `manual_review_time = 0`, `manual_review_turns = 0`, and
  `manual_review_cost = 0`. Automatic extraction/promotion cost remains in the
  normal remem cost ledger. A separately named diagnostic may measure human
  review later, but it cannot enter the primary claim. Direct gold inserts,
  manual `save_memory`, target-visible hidden data, or preloaded answers
  invalidate the run.
- The source-native import diagnostic snapshots actual native files produced
  by the source host before toggling import. The with and without arms share
  the same source episodes, native snapshot, target task, and non-import
  configuration. Each arm starts from a fresh `native_neutral_base_v1` clone
  whose native candidate/active/provenance counts are zero, then derives and
  verifies its own raw-free target projection. Only the with arm performs the
  import and target-blind review/promotion mutation before projection; the
  importer must report `Inserted`, while `Duplicate` makes the pair
  `INSUFFICIENT`. Each native record must resolve the full
  import/candidate/review/memory lineage described above; quarantine, noop, or
  pre-existing memory reuse is also insufficient.

The target task prompt is byte-identical across conditions. A condition may
add only its declared, separately hashed memory envelope or production
SessionStart/MCP context surface.

### 4. Scope and Privacy

Every memory-bearing source surface includes a target-blind conflicting
foreign-project canary. Production selection, export, and import must exclude
it. Any selected, injected, cited, or used foreign-project canary is
`wrong_project_injection`.

The current production `ContextRequest` has no user identity and startup
queries use `user:default`. An eval-only `authorized_user_id` cannot prove
multi-user isolation. Until a separate production identity contract is
implemented and carried through capture, retrieval, ContextRequest, and
SessionStart:

- `wrong_user_injection` is `not_testable`, not `0`;
- no decoy-user result may be presented as production evidence; and
- the public comparative verdict is `INSUFFICIENT`.

A verified redacted security breach remains an overriding safety `FAIL`; it is
the only exception to the identity-driven comparative `INSUFFICIENT`.

Once that prerequisite ships, the benchmark version must be bumped and each
memory-bearing surface must include a distinct same-project decoy user with a
negative selection assertion.

Real HOME paths, auth/config material, source host sessions, hidden tests, and
private benchmark roots must never reach target-visible or committed
artifacts. A detected leak produces a redacted, schema-valid
`security_breach` record; leaked bytes are discarded.

### 5. Failures, Attempts, and Attribution

Every attempted tuple produces an immutable record. Auth failure, cancellation,
timeout, host crash, capture/extraction failure, scoring failure, scan failure,
and cleanup failure are explicit outcomes.

The attempt rule is fixed before live execution. It applies independently to a
source preparation unit and to each dependent target tuple:

- at most three immutable `source_attempt_id` values are allowed before any
  dependent target prompt is revealed, and at most three immutable
  `target_attempt_id` values are allowed per target tuple;
- a retry before the relevant target prompt is revealed is allowed only for
  `transient_auth_unavailable`, `provider_unavailable`,
  `host_bootstrap_failed`, `runner_io_before_prompt`, or
  `pre_prompt_timeout`;
- the first target attempt that reveals the prompt is the sole claim-bearing
  attempt, regardless of success, failure, timeout, or unresolved outcome;
- semantic/task failure, post-reveal timeout, security breach, scope leak,
  scoring result, or cleanup failure cannot be retried into a better claim
  outcome;
- if all three target attempts fail before prompt reveal, the tuple is an
  `ordinary_failure` with `resolved = false`;
- if all three source attempts fail before any dependent prompt reveal, the
  dependent tuples are recorded as not started and the candidate report is
  `INSUFFICIENT`.

Every target record references exactly one `source_attempt_id` and has its own
`target_attempt_id`. All attempts remain immutable and visible in operational
metrics.

For `remem_shared`, capture, extraction, review/promotion, selection, and
citation/use each contain either a resolvable reference or a typed
`absent_due_to`. An upstream failure may make downstream references absent,
but the failed run remains in the denominator.

Every scorer-recognized wrong action has a closed causal record: `proven`,
`not_proven`, or `missing_evidence`. It resolves the registered selector before
target reveal to `(projection_logical_hash, memory_id, content_hash,
state_hash)` and records the deterministic first matching
`wrong_action_event_id/order`. Before any target reveal, its rule chooses
exactly one proof method: either a deterministic parser for a target-authored
memory-use event recorded before the wrong action is dispatched/applied, or a
deterministic action counterfactual whose transform, allowed byte/field delta,
pre-action fixture, commands, assertions, and outcome table are all plan
hashed. The counterfactual replays the exact recorded action and a copy with
only the registered memory-derived input removed/replaced from the same hashed
pre-action state. It proves causation only when the factual replay reproduces
the registered wrong action/scorer failure and the transformed replay removes
that action and passes the required assertion.

Injection/retrieval records prove exposure, not use.
`memory_usage_events` and citations derived from the final Stop output are
post-action corroboration only. A valid counterfactual with the same registered
failure records `not_proven/refuted_counterfactual`; a no-memory surface records
`not_proven/no_memory_surface`. Missing or non-unique events/order, an
unsupported or non-exact transform, conflicting outcomes, competition between
memories, or any post-hoc human/LLM label records `missing_evidence` and, absent
a verified security breach, makes the comparative metric/verdict
`INSUFFICIENT`.

Origin is a closed set:
`remem_canonical_capture`, `host_native_import`,
`generated_projection`, and `manual_save`. Primary `remem_shared` evidence may
not be relabeled from a diagnostic or manual origin.

### 6. Partial and Security Evidence

The report builder accepts three manifest forms:

- `complete`: all required primary and diagnostic tuples are present;
- `partial_non_security`: planned, recorded, failed, missing, and not-started
  counts are explicit; verdict can only be `INSUFFICIENT`;
- `partial_security`: contains at least one verified, redacted security-breach
  record; security takes precedence and verdict is `FAIL`.

A security breach may stop remaining billable work. Non-security interruption
must still produce a candidate report instead of stranding evidence before the
reporting stage.

### 7. Metrics and Public Claims

Reports show each direction first, then aggregate values. Aggregate improvement
cannot hide a missing or regressing direction.

The primary claim family is `remem_shared` versus both `no_memory` and
`exported_file` in each direction. `target_host_native` is not a superiority
comparator. For direction `d` and comparator `c`, the registered estimand is:

```text
delta[d,c] =
  (1 / 12) * sum_task(
    (1 / 3) * sum_run(
      I[remem_shared.resolved] - I[c.resolved]
    )
  )
```

`paired_task_cluster_bootstrap_v1` performs 100,000 resamples, each with exactly
12 replacement draws from UTF-8 task-ID-sorted indices `0..11`, retaining all
three registered run pairs. The normative fixed-width SHA-256 framing,
rejection sampling, exact rational replicate, and one-based ranks are defined
in TECH and bound with implementation bytes and test vectors. Ranks 2,500 and
97,500 form the descriptive two-sided 95% interval; rank 98,750 is the
Bonferroni one-sided 1.25% familywise regression upper bound.

After completeness, identity, attribution, and stop-loss validation:

- any of the four adjusted regression upper bounds below zero is a
  familywise-5% statistical regression and yields `FAIL`;
- all four intervals with `lower > 0` yield `PASS`;
- every other complete result yields `INSUFFICIENT`, including an interval
  touching/crossing zero or an aggregate gain hiding a direction.

This is an all-four intersection-union claim, so no comparison can be selected
post hoc. A future independently publishable comparison requires a new
pre-registered multiplicity contract.

Verdict order is unique: verified security breach -> safety `FAIL`; missing
identity, incomplete/invalid evidence, non-security partial manifest, or
missing causal/applicable data -> comparative `INSUFFICIENT`; only then may
complete valid denominators trigger a non-security stop-loss `FAIL`, adjusted
regression `FAIL`, all-four `PASS`, or otherwise `INSUFFICIENT`.

The closed security-breach predicates are `wrong_project_injection > 0`,
`source_private_session_leak > 0`, and any key exposure. Each produces
`security_breach` / `partial_security` / safety `FAIL` even with missing
identity or an incomplete matrix. The non-security claim-bearing stop-losses
on complete-valid denominators are:

- `stale_memory_followed <= 1%`;
- `memory_hurt <= 2%`.

`memory_hurt` counts a paired tuple only when `no_memory` resolves the task,
`remem_shared` does not, and `causal_oracle_v1` returns `proven` for the same
exposed memory and wrong action under the registered proof method. Its
denominator is all complete, valid paired
`no_memory`/`remem_shared` tuples in that direction. Missing causal attribution
makes the metric and verdict insufficient; it does not shrink the denominator.

`stale_memory_followed` uses a direction-specific applicable set: complete,
valid `remem_shared` tuples whose hashed candidate inventory, captured from the
sealed target-transfer projection immediately before the production
freshness/validity filter, contains at least one query-relevant
stale/superseded challenge item that would otherwise match. Projection
sanitization must retain registered challenge rows with their production state
flags; this evaluator-only inventory is not exposed as a target memory surface.
The numerator is an applicable tuple where that item is cited or used and
has a `causal_oracle_v1 = proven` record for the wrong action. Missing
inventory-stage proof, challenge inventory, or causal evidence makes the metric
and verdict insufficient; it does not
remove the tuple from the denominator. Reports show both the applicable count
and all 36 `remem_shared` tuples per direction. A claim-bearing report requires
at least one registered challenge across all three runs of the relevant task
and therefore at least three applicable tuples per direction; fewer or none
forces `INSUFFICIENT`. Empty metrics remain blank with a reason, never
zero-filled.

The release evidence contains all sanitized primary and diagnostic records,
attempt history, manifests, scorer/version hashes, direction reports, and
claim verdict. JSON is canonical. Markdown is deterministically rendered from
that JSON; the verdict binds both hashes and the renderer version, and
verification regenerates Markdown byte-for-byte.

For `PASS`, `FAIL`, and `INSUFFICIENT`, the same reporting change updates:

- `eval/cross-host/README.md`;
- this PRODUCT/TECH pair and `docs/specs/README.md`;
- the public-memory benchmark contract;
- report/evidence links and the honest current status.

README, README.zh-CN, CHANGELOG, release notes, or marketing surfaces may cite a
positive cross-host result only after a complete, hash-bound `PASS`.

## Live-Run Authorization

Schema checks, dry-run, verification, and report regeneration are offline and
must never start a host or network call.

Every live invocation requires an explicit human command bound to the exact
Git head, fixture/plan hash, host/model profiles, tuple set, and hard
per-invocation host-call, LLM-call, and estimated-cost caps. The v2 runner is
single-operator only:

- it does not accept a reusable SpecRail approval, route-gate result, or PR-gate
  artifact;
- it does not claim a local lock or same-repository Git ref is a global budget
  ledger;
- it does not support concurrent or cross-clone execution under one
  authorization.

If multi-operator execution is added later, it requires a separately reviewed
coordinator with an enforceable narrow reserve/settle API. A credential model
that can write a repository Git ref while supposedly lacking repository
content access is not acceptable.

Smoke, full matrix, public wording, merge, and release remain separate human
decisions. This spec PR authorizes none of them.

## Non-Goals

- No live source-host-to-target-host session bridge.
- No generic agent lease, queue, or remote budget service.
- No CI-triggered paid benchmark run.
- No full source transcript as a primary target context.
- No automatic trust upgrade for host-native content.
- No claim that `target_host_native` is a cross-host transfer baseline.
- No implementation or fabricated benchmark outcome in this PR.

## Acceptance Criteria

- All 24 tasks are executable and schema-valid without placeholders.
- Offline validation plans exactly 288 primary and 144 native-import tuples
  without launching hosts.
- Source sealing, byte-identical fan-out, canonical project identity, condition
  isolation, counterbalanced ordering, encrypted/rekeyed target projection,
  native-neutral ablation, maintained-export protocol, causal attribution,
  fixed retry selection, failure retention, and leak redaction have positive
  and negative tests.
- The production user-identity prerequisite is either implemented and tested,
  or every public comparative verdict is deterministically `INSUFFICIENT`.
- A separately authorized smoke covers both directions and all six required
  primary and native-import surfaces before any full-matrix authorization.
- Complete and both partial manifest forms regenerate deterministic JSON,
  Markdown, and verdict artifacts.
- The four registered direction/comparator intervals deterministically produce
  `PASS`, statistical-regression `FAIL`, or `INSUFFICIENT` under the exact
  bootstrap predicate.
- Current documentation is updated for every final verdict.
- No public positive claim appears before a complete verified `PASS`.
