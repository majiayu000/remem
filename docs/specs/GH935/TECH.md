# Cross-Host Continuity Benchmark — Technical Contract

Status: Current contract; v1 infrastructure shipped, completion unimplemented
Issue: #935

## Existing Implementation Facts

- `eval/cross-host/benchmark-charter.json` is
  `cross-host-v1` / `infrastructure_only_no_runs`.
- `eval/cross-host/tasks/` contains 24 `skeleton_todo` JSON tasks.
- `cross-host-task.schema.json`, `cross-host-run.schema.json`,
  `schema_validate.py`, `scan_artifacts.py`, and `run_dry.py` are offline
  infrastructure. There is no Rust cross-host runner or report command.
- Production project identity is the canonical absolute Git root returned by
  `project_from_cwd` in `src/project_id.rs`.
- Production `ContextRequest` has no user field, and startup user-owned state
  currently resolves through `user:default`.
- The host-native import paths in `docs/specs/host-native-memory/` produce
  untrusted candidates. They do not auto-promote active memories.
- Claude Code `Write`/`Edit` observe hooks can call `sync_native_memory` and
  insert `claude_native` candidates into the normal source database before a
  diagnostic importer runs; cloning that database is therefore not a neutral
  import control.
- PR #965 removed the repo-local SpecRail workflow, route/PR gates, sensitive
  registry, and their executable checks. They are not dependencies of this
  design.

## Versioning

Implementation must bump the charter and all changed schemas from
`cross-host-v1` infrastructure to the executable `cross-host-v2` contract.
Old skeletons and examples cannot silently become result-bearing artifacts. A
converter, if provided, must validate every field and write v2; otherwise old
input is rejected.

The status lifecycle is:

```text
infrastructure_only_no_runs
  -> executable_no_runs
  -> partial_evidence
  -> complete_evidence
```

`partial_evidence` and `complete_evidence` describe artifact availability, not
claim success. Verdict is separately `PASS`, `FAIL`, or `INSUFFICIENT`.

## Planned Layout

The implementation should extend existing surfaces instead of adding a second
benchmark framework:

| Area | Planned ownership |
|---|---|
| Charter, task fixtures, schemas, offline validators | `eval/cross-host/` |
| Shared process/HOME/workspace isolation | reusable primitives extracted from `src/eval/coding_bench/` |
| Cross-host plan, source seal, conditions, runner, score, artifacts | `src/eval/cross_host/` plus `src/eval/cross_host.rs` |
| CLI wiring | existing `bench` command modules under `src/cli/` |
| Sanitized release evidence and reports | versioned paths under `eval/cross-host/evidence/` and `eval/cross-host/reports/` |
| Current-status documentation | `eval/cross-host/README.md`, this spec pair, the spec index, and public-memory benchmark contracts |

No implementation slice owns a removed `workflow.yaml`, `checks/route_gate.py`,
`checks/pr_gate.py`, or SpecRail state/label file.

## Contract Objects

All JSON uses snake_case and closed enums. Unknown fields are rejected on
claim-bearing artifacts unless a schema version explicitly permits them.

### Executable Task

An executable task adds to the v1 skeleton:

- immutable fixture repository revision and canonical workspace recipe;
- two or more chronological source episodes;
- source-host expected evidence and allowed/forbidden paths;
- hidden tests and non-empty score commands;
- target-blind gold facts;
- foreign-project decoy fixture and canary;
- for `stale_superseded_decision`, at least one deterministic
  stale/superseded challenge per run with its production state and expected
  query-relevant pre-filter match assertion;
- target-invisible `causal_oracle_v1` rules covering every scorer-recognized wrong-action class,
  each binding a unique memory/content/state hash, artifact matcher, and one plan-hashed proof method plus its closed outcome table;
- source-native snapshot policy for the diagnostic arm;
- per-host executable/profile requirements;
- `status: "ready"` and `todo: []`.

The task validator rejects empty values, aliases, direction/host mismatch,
target-visible hidden paths, and any canary copied into prompts or gold facts.

### Source Seal

One source seal exists per
`(direction, task_id, run_index, source_attempt_id)`. It
contains:

- task/fixture, executable, profile, model, schema, and migration hashes;
- source host/session/event IDs and transcript/tool-event/Git hashes;
- canonical project path and the exact `project_from_cwd` value;
- terminal extraction/review state and queue counts;
- a sorted data-only manifest for the quiesced full source
  `REMEM_DATA_DIR`, with typed exclusions for `.key`, environment-provided
  keys, WAL/SHM, sockets, and other runtime-only secret/process files;
- a deterministic data-only full-store archive hash, length, and creation
  policy; excluded secret/process entries are recorded by type, never value;
- target-transfer projection logical hash, sealed SQLCipher ciphertext hash,
  sorted manifest, non-secret `projection_key_id`, derivation/rekey/provisioning
  policy versions, and proof that source raw/capture/session rows, auxiliary
  index entries, and files are absent;
- `native_neutral_base_v1` hash, derivation policy, zero native
  candidate/active/provenance counts, and referential-integrity proof;
- source-host native file manifest/hash, or typed absence;
- `maintained_export_v1` hash plus every cycle input/state/usage/budget; a
  commit carries prior/output/freeze hashes, while failure carries a typed
  reason and no output/freeze hash;
- creation/cleanup timestamps and scanner status.

The full store archive and target-transfer projection are private execution
material, not committed public artifacts. In v2, a source and all dependent
targets run in one single-operator execution root. The full archive is
data-only: its source SQLCipher key stays in a runner-private secret root only
until attribution completes. The deterministic projection logical content and
the one sealed ciphertext object have separate hashes. The sealed ciphertext
is read-only; only its fresh byte-identical clones can back `remem_shared`.
Initial clones may diverge only through normal target-runtime writes after
verification.
The full archive lives outside every target-visible root and is added to the
target host-read deny policy; a target process that can open it invalidates the
run even if no transcript is emitted.

The projection is derived mechanically from the full store after the fixed
primary review policy completes. A trusted runner opens the source with its
private source key, sanitizes into the logical projection, and SQLCipher
exports/rekeys it under a fresh projection-scoped 256-bit key that is distinct
from the source key. The same sealed ciphertext and key back every initial
clone of that projection. It preserves the complete current schema, migration
metadata, required tables, triggers, and indexes so normal `open_db()`
schema-drift checks and MCP preflight succeed. It contains curated memory rows,
relations, current-state, and minimal opaque provenance needed to reproduce
production candidate filtering. Registered stale/superseded challenge rows
remain present with their production state flags unchanged; they are not made
retrieval-eligible or exposed through a target-only surface.

The sanitizer removes source rows from raw messages, captured events/blobs,
observations, extraction payloads, and other target-ineligible session
surfaces; removes their FTS/auxiliary index entries and resolvable private
paths; checkpoints into a fresh database; and ships no source transcript,
host-native file, credential, `.key`, WAL, or SHM file. It must not delete a
registered challenge row merely because the row is stale or superseded.

The plan-hashed `projection_secret_channel_v1` fixes IPC path/type,
authentication, lifecycle, and failure behavior. A trusted runner outside the
host sandbox starts a remem sidecar and provisions one projection key over a
private inherited pipe. Host hook/MCP config has only a keyless authenticated
client endpoint. The host cannot read the sidecar environment, argv, process
metadata, pipe, or secret root; no key enters HOME, config, projection,
manifest, log, or `.key`.
The runner checks the sealed ciphertext and clone hashes before any SQLite
open. Schema/open/MCP preflight uses a disposable clone; an actual target clone
records its pre-open hash, then alone becomes mutable. Validation proves schema
and migration invariants, referential integrity, `PRAGMA foreign_key_check`,
encrypted header, and production open; source, wrong, and missing keys fail
closed, and raw/sparse searches return zero source hits.
A private registry tracks each `projection_key_id`,
`remaining_target_tuple_ids`, `active_attempt_ids`, and destruction proof.
Attempt exit releases only its active ref; the tuple reservation survives a
retryable pre-prompt failure. First prompt reveal, exhausted retry budget, or a
non-retryable terminal result removes that tuple exactly once; zero destroys
the key. Live scans exclude expected trusted live holders. After raw attribution
materialization, the runner destroys the source key, then scans trusted holders,
environment, argv, logs, process metadata, and retained artifacts for raw, hex,
and versioned encodings. It records cleanup before finalizing the manifest and
report. Exposure is a security breach; cleanup failure is partial/insufficient.

`native_neutral_base_v1` is a separate mechanical preparation object and never
replaces or mutates the primary full store/projection. It removes candidates
whose `source_kind` is `claude_native` or `codex_native` and the transitive
candidate-derived memory, operation, graph, and current-state provenance
closure, then proves native candidate/active/provenance counts are zero,
foreign keys and schema are valid, and production open succeeds. Both native
diagnostic arms start from byte-identical neutral-base clones.

Cross-clone continuation is deliberately unsupported. A `source_attempt_id`
owns the immutable full-store seal and projection; each dependent target has a
separate `target_attempt_id`. The runner never regenerates an object under an
existing seal. Before any dependent target prompt is revealed, it may start a
new source attempt from scratch only when the failure has one of the five exact
`pre_prompt_transient_v1` retry reason codes and the source-preparation
three-attempt limit has not been exhausted. Archive absence, hash mismatch,
integrity or security
failure, cleanup failure, and interruption after prompt reveal are not
retry-eligible: they preserve the immutable partial record and yield
`partial_non_security`/`INSUFFICIENT` or, for a verified security breach,
`partial_security`/`FAIL`. This avoids pretending a machine-local archive is a
durable cross-clone object. A future cross-clone mode would require a
separately specified encrypted immutable store, retention policy, access
protocol, and independently verified fetch path.

### Maintained Export Protocol

`maintained_export_v1` is closed and plan-hashed before live execution. It
contains:

- exact system, generation, and update prompt bytes and hashes;
- exporter executable, model, profile, sampling, and protocol versions;
- `evidence_projection_v1`, which enumerates each visible source ref/file and
  content hash in canonical order, includes the foreign-project canary as an
  exclusion test, and rejects target prompt, gold, hidden scorer, and prior
  condition output;
- generation input as episode-one evidence, and each update input as the
  previous frozen envelope plus only the newly registered episode delta;
- envelope schema/version, stable-key update rules, and a conflict rule that
  replaces prior state only with explicit chronology/supersession evidence and
  otherwise preserves both sides with an abstention marker;
- per-cycle and cumulative host-call, LLM-call, input/output-token, turn,
  wall-time, byte, and estimated-cost caps.

Each boundary creates one immutable `cycle_id` in exactly one state:
`committed` (one atomic schema-valid envelope commit), `failed` (zero commits),
or `not_started_after_prior_failure`. Regeneration, output selection, and prior
envelope fallback are forbidden. The first non-retryable failure terminally
disables later cycles. A whitelisted pre-prompt transient aborts the entire
source attempt; a fresh attempt may restart under `pre_prompt_transient_v1`
while retaining all records/cost. Other non-security/non-integrity failures
make `exported_file` an `ordinary_failure`; other conditions continue from the
unmodified source seal. Security/integrity failures use global breach rules.
Retry never changes prompts, evidence, conflict rules, schema, or caps.

### Causal Oracle

Each task's evaluator-only `causal_oracle_v1` is fixed and hashed before target reveal. A rule
fixes `rule_id`, logical memory selector, wrong-action artifact type/matcher, scorer assertion,
allowed evidence fields, and exactly one plan-hashed `proof_method`: `pre_action_use_v1` or
`action_counterfactual_v1`. The selector resolves after source sealing and before reveal to a
unique `(projection_logical_hash, memory_id, content_hash, state_hash)`; absence, ambiguity, or two rules mapping one wrong action is `missing_evidence`.

`pre_action_use_v1` fixes a target-authored event type, deterministic parser, memory-use predicate,
and ordering fields. The append-only host event is captured before dispatch/execution, resolves the
same memory tuple, and satisfies `use_event_order < wrong_action_event_order`; runner-authored injection, rendering, and retrieval-result events prove exposure only.

`action_counterfactual_v1` fixes a transform, allowed changed bytes/fields, pre-action fixture recipe,
isolated factual/counterfactual commands, assertions, and outcome table. From the same hashed state,
the factual replay executes the exact recorded action and the other changes only the registered
memory-derived input. `proven` requires factual matcher/scorer failure and a transformed replay where
the matcher is false and the required assertion passes. The same registered failure is
`not_proven/refuted_counterfactual`; non-unique transforms, unsupported side effects, replay
drift/errors, different failures, or conflicting results are `missing_evidence`.

The closed record binds method/intervention hashes, resolved tuple, use and first-wrong-action event IDs/orders, replay hashes, matcher/scorer results, reason, and status. A Stop-created `memory_usage_events` row and its `memory_citation_events` parent are post-action corroboration only
and cannot satisfy either proof method. Missing, ambiguous, competing, or post-hoc evidence becomes
`missing_evidence` and, after security-breach precedence, comparative `INSUFFICIENT`; a no-memory
surface is `not_proven/no_memory_surface`. Sanitized inputs support clean recomputation.

### Run Record

Each target attempt records:

- tuple key, `source_attempt_id`, `target_attempt_id`, condition, direction,
  task, and run index;
- planned condition position, realized position, and start/end timestamps;
- source seal hash, even when the condition cannot read its contents;
- task prompt, condition surface, executable/profile, and scorer hashes;
- projection logical/ciphertext hashes and `projection_key_id`, where
  applicable;
- target HOME/config/session/workspace identities;
- status: `success`, `ordinary_failure`, or `security_breach`;
- resolved and secondary metrics with nullable values and reasons;
- host calls, LLM calls, tokens, wall time, turns, and estimated cost;
- cleanup and leak-scan results;
- attribution refs or typed `absent_due_to` for every production stage;
- maintained-export protocol/boundary/budget records or native-neutral/import
  attempt records for the applicable condition;
- a closed `causal_oracle_v1` record for every scorer-recognized wrong action.

A record can be schema-valid while its task outcome failed. Failed records stay
in the registered denominator.

### Evidence Manifest

The manifest has one of three closed kinds:

```text
complete
partial_non_security
partial_security
```

Every kind contains the planned tuple set, recorded attempts, the fixed attempt
policy/version, the selected claim attempt, missing/not-started tuples, artifact
hashes, and reason codes.

- `complete` requires 288 unique primary tuples and 144 unique source-native
  import tuples.
- `partial_non_security` is valid for cancellation, unavailable auth,
  ordinary host failure, scanner failure that cannot safely classify a leak,
  missing pairs, or incomplete execution. It can only yield
  `INSUFFICIENT`.
- `partial_security` contains at least one verified redacted breach record and
  can stop remaining paid work. Security precedence yields `FAIL`.

Earlier attempt and partial manifests are immutable. A later retry creates a
new manifest that references them; it never overwrites them.

### Report and Verdict

The canonical JSON report includes:

- planned, recorded, selected, failed, missing, and not-started counts;
- direction-first primary and diagnostic tables;
- numerator, denominator, applicability, and missing-data reason for every
  metric;
- paired task-cluster bootstrap inputs, algorithm version, seed, confidence
  level, intervals, and verdict;
- causal matcher inputs/results and native-neutral/import lineage needed to
  recompute stop-losses and diagnostic pairing;
- per-task and aggregate export generation/maintenance cost;
- stop-loss values and source attribution;
- hashes of the manifest and complete sanitized record bundle.

Markdown is rendered deterministically from canonical JSON. The verdict binds:

- JSON report hash;
- Markdown report hash;
- renderer version/hash;
- evidence-manifest and sanitized-record-bundle hashes.

Verification regenerates Markdown and compares it byte-for-byte before any
public surface may link the report.

## Execution State Machine

### 1. Offline Validate and Plan

The planner:

1. validates the charter, schemas, all 24 tasks, fixtures, scorer paths,
   maintained-export protocol, and causal-oracle rules;
2. proves 288 primary and 144 required diagnostic tuple keys with no
   duplicates;
3. resolves exact host/model/profile/exporter binaries and hashes, plus
   projection derivation/rekey/provisioning and bootstrap versions;
4. builds `balanced_latin_square_v1` over the four primary and two required
   native-import conditions for 36 source seals per direction. For six
   seed-permuted labels, `williams_6_v1` starts with positions
   `[0, 1, 5, 2, 4, 3]` and forms the other five rows by adding `1..5` modulo
   six. The seed hashes the label permutation and deterministic row assignment;
   the complete six-row block repeats six times across the pre-sorted source
   seals. The verifier proves every condition appears exactly six times in
   every serial position and every ordered pair of distinct adjacent conditions
   appears exactly six times per direction;
5. calculates upper bounds for host calls, LLM calls, export bytes, and
   estimated cost;
6. writes the schedule, all protocol/seed/algorithm hashes, and canonical plan
   hash without starting a host or network call.

Dry-run and verification must have tests proving their call graph cannot reach
the host adapter.

### 2. Prepare One Run-Scoped Workspace

For one `(direction, task_id, run_index)`:

1. create separate source and target HOME/config/session roots;
2. create one canonical absolute Git workspace path;
3. materialize the approved fixture at that path;
4. create a separate same-name decoy repository at a different canonical
   path/project identity;
5. strip the environment to an allowlist and install host-read restrictions;
6. ensure no real user HOME, default remem DB, auth file, hidden test, or
   previous run root is visible.

Source and target phases run sequentially at the same canonical workspace
path. Between phases the runner destroys source process/session state and
resets the fixture at the same path. Different worktree paths are not
substitutable because production project identity is path-based.

### 3. Run and Seal the Source Once

The source host executes all chronological source episodes before seeing the
target task. remem uses normal automatic capture. At each episode boundary, the
runner drains and quiesces capture, records source-store, source-native, and
workspace manifests and, unless export already failed, launches the
target-blind exporter in a separate exporter-only HOME/config/session. It has
remem hooks, automatic
capture, and host-native persistence disabled; it receives read-only mounts of
only `maintained_export_v1`'s canonical `evidence_projection_v1` and can write
only the envelope and its usage log. It uses the exact plan-hashed input/rules;
the first terminal failure marks later cycles
`not_started_after_prior_failure` and launches no exporter for them. The runner
proves protected manifests unchanged; mutation/protocol drift invalidates
every dependent condition. After the final episode, the runner:

1. records the final committed envelope/freeze hash, or typed terminal failure
   with no output/freeze hash;
2. drains extraction and applies primary review policy `automatic_only_v1`:
   only the shipped automatic promotion path may activate candidates;
   pending/quarantined rows stay inactive, manual review mutations are rejected,
   and `manual_review_time`, `manual_review_turns`, and `manual_review_cost` are
   recorded as zero while automatic pipeline cost remains attributed to remem;
3. stops the worker, checkpoints the database, closes writers, and fsyncs the
   run root;
4. snapshots the exact data-only remem store and source-host native files while
   retaining the source key only in the runner-private secret root;
5. derives and verifies `native_neutral_base_v1` without altering the primary
   full store;
6. derives the curated logical projection, SQLCipher exports/rekeys it under a
   fresh projection key, and proves forbidden surfaces absent while schema and
   referential integrity remain valid;
7. seals all store/native hashes plus committed export hashes or typed failure;
8. verifies sealed/clone ciphertext hashes before any open, then performs
   production-open/MCP checks on a disposable clone.

Failure to quiesce, seal, clone, or verify produces an ordinary failure or
security record. The runner cannot continue with a regenerated or partially
copied store.

### 4. Fan Out Conditions

Targets run serially in fresh HOME/config/session roots and in the exact
plan-hashed counterbalanced order. Each condition gets a fresh fixture reset
and only its declared memory surface. The runner records planned/realized
position and timestamps; skipped, reordered, or duplicated positions make the
comparison insufficient.

#### `no_memory`

No source archive, exported envelope, remem data root, or native file is
mounted.

#### `target_host_native`

The target uses only its own fresh native state. It cannot read the source
seal, source transcript, source-native snapshot, exported projection, or remem
store. The record may reference the source seal hash for pairing, but the host
process cannot resolve it.

This condition is expected to be empty unless the target itself creates native
state before the task. The v2 suite does not add such a preparation phase and
does not present this arm as a populated cross-host native baseline.

#### `exported_file`

A source-side exporter creates a sanitized host-neutral envelope after the
first source episode and updates it after every later episode. The final
envelope is frozen before the target task is revealed. Every byte of generation
uses the plan-hashed `maintained_export_v1` prompts, evidence projection,
incremental update/conflict rules, envelope schema, and budgets. Each
invocation uses the exporter-only isolation root defined by the source state
machine; no remem or host-native capture runs there, the source
store/native/workspace mounts are read-only, and before/after manifests must
match.

The runner exposes the envelope through the same versioned system/context
adapter for Claude Code and Codex. The target task prompt remains identical;
the separately hashed envelope is the treatment surface. A condition-only task
prompt note is forbidden.

The envelope contains only allowed handoff facts and provenance, never raw
transcripts, tool logs, hidden paths, or the foreign-project canary.

#### `remem_shared`

The target receives a fresh verified clone of the sealed curated transfer
projection, never the full automatic-capture store. Retrieval runs through the
production SessionStart/MCP/Context Bundle path. The raw MCP/CLI surfaces and
ordinary-search raw fallback may exist in the binary, but the projection
contains no source raw rows or transcript files for them to return. Direct
memory inserts, gold seeding, manual saves, manual candidate review, and
special eval-only retrieval are rejected by attribution validation.

#### Source-Native Import Diagnostic

The runner captures actual native-memory files produced by the **source host**
before cleanup:

- Codex source: supported Codex rollout-summary input;
- Claude Code source: supported Claude topic-file input.

Both diagnostic arms start in separate preparation roots from byte-identical
`native_neutral_base_v1` clones and the same sealed source-native snapshot.
Before toggling, each clone must have zero native candidate, active-memory, and
transitive-provenance rows; neither base is target-visible. The without arm
does not import or review, then derives/rekeys its projection. The with arm
invokes the shipped importer: Codex input uses its dry-run plan digest; Claude
input is bound to the sealed snapshot and runner plan. For every sealed
`native_record_id`, it must return `Inserted(candidate_id)`, record
`import_attempt_id -> native_record_id -> candidate_id`, apply the registered
review with `promoted = true`, and produce a `memory_id` whose
`source_candidate_id = candidate_id`. It records the immutable chain,
time/turns/cost, `host_native_import` origin, and external trust before deriving
and rekeying its projection. Duplicate, quarantine, noop, pre-existing-memory
reuse, or any lineage mismatch makes the pair `INSUFFICIENT`.
The import/review mutation is the only allowed difference before projection;
both targets receive fresh projection clones and never a full source-store or
neutral-base clone.

Empty, unsupported, unsealed, or different native inputs make the pair
insufficient. No target-host native preparation and no raw source transcript
is used for this diagnostic.

### 5. Score, Scan, and Record

After the target exits, the runner:

1. freezes target output before revealing hidden tests;
2. runs the scorer in a separate restricted process;
3. scans target-visible and candidate artifact roots;
4. records outcome, attribution, cost, and cleanup;
5. redacts leaked bytes before persisting a `security_breach` record;
6. destroys target HOME/session roots and verifies cleanup;
7. releases its active-attempt key ref, and releases the tuple reservation only
   when that tuple becomes terminal under `pre_prompt_transient_v1`;
8. after all targets/raw attribution, destroys the source key, post-scans
   trusted scopes, records cleanup, then finalizes the manifest/report.

Scanner or cleanup failure is never a warning-only success. If a leak cannot be
safely classified and redacted, the manifest is
`partial_non_security`/`INSUFFICIENT`; if a redacted breach is verified, it is
`partial_security`/`FAIL`.

## Project and User Scope

### Project Scope

The automatic store and every memory-bearing source surface include a
target-blind conflicting memory for a distinct canonical project. Tests must
prove:

- source and target at the authorized canonical path share the intended
  production project identity;
- the decoy path has a different production project identity;
- the decoy is excluded from import, export, selection, injection, citation,
  and use;
- moving source and target to distinct worktree paths is rejected.

### User Scope Prerequisite

The benchmark must not manufacture user identity only in eval artifacts.
Before `wrong_user_injection` can be numeric, production must have a separate
current contract and implementation that carries explicit user identity
through:

- capture/session ownership;
- active memory ownership and retrieval filters;
- `ContextRequest`;
- SessionStart data-version hints and diagnostics;
- API/MCP boundaries that expose the same behavior.

Until then, schemas require:

```json
{
  "wrong_user_injection": null,
  "user_scope_status": "not_testable_single_user_runtime"
}
```

The report verifier rejects `0` or `pass` for that metric and forces the public
comparative verdict to `INSUFFICIENT`. This PR does not add that runtime
identity plumbing.

## Metrics and Verdict Rules

The attempt policy is `pre_prompt_transient_v1` and applies independently to
each source preparation unit and each target tuple:

- maximum three immutable `source_attempt_id` values before any dependent
  prompt reveal, and maximum three immutable `target_attempt_id` values per
  target tuple;
- retry before the relevant target prompt is revealed only for
  `transient_auth_unavailable`, `provider_unavailable`,
  `host_bootstrap_failed`, `runner_io_before_prompt`, or
  `pre_prompt_timeout`;
- the first prompt-revealed target attempt is selected for the claim regardless
  of its outcome, and no later claim retry is allowed;
- semantic failure, unresolved result, post-reveal timeout, scorer result,
  security/scope event, and cleanup failure are never retry-eligible;
- when all three target attempts fail before reveal, the tuple is selected as
  `ordinary_failure` / `resolved = false`;
- when all three source attempts fail before any dependent prompt reveal, the
  dependent tuples are recorded as not started due to source failure and the
  candidate report is `partial_non_security` / `INSUFFICIENT`.

The report validates this rule from immutable lifecycle timestamps and reason
codes. It includes every attempt in operational reliability/cost tables and
cannot choose a successful retry after seeing outcomes.

The primary statistical family has four registered comparisons:
`remem_shared` versus `no_memory` and `exported_file`, separately in both
directions. `target_host_native` is not a superiority comparator. For each
comparison, the estimand is the mean across 12 tasks of the within-task mean
across three paired runs of
`I[remem_shared.resolved] - I[comparator.resolved]`.

`paired_task_cluster_bootstrap_v1` UTF-8-sorts task IDs to indices `0..11`.
Enums are `direction: 0=claude_to_codex, 1=codex_to_claude` and
`comparator: 0=no_memory, 1=exported_file`. For `r=0..99999`, `j=0..11`:

```text
k = 0
p = ASCII("cross-host-v2/bootstrap-v1\0") || raw32(plan_hash)
    || u8(direction) || u8(comparator) || u32be(r) || u8(j) || u32be(k)
u = u64be(SHA256(p)[0:8])
if u >= 18446744073709551612: k += 1; retry this draw
sample[r,j] = u % 12
value[r] = sum(sampled clusters' three paired differences) / 36
```

`raw32(plan_hash)` decodes its canonical lowercase hex. Exact rationals are
sorted; one-based ranks 2,500/97,500 form the descriptive 95% interval and rank
98,750 is the Bonferroni one-sided 1.25% regression upper bound. The plan binds
algorithm bytes/hash and fixed sampling/quantile vectors.

`memory_hurt` is the paired predicate:

```text
no_memory.resolved = true
AND remem_shared.resolved = false
AND causal_oracle_v1.status = proven
```

Its denominator is all complete valid `no_memory`/`remem_shared` pairs in the direction. The
causal record must bind the same memory across exposure,
pre-action use or registered counterfactual, wrong-action matcher, and scorer
failure. Missing, ambiguous, competing, or post-hoc attribution makes the metric and verdict insufficient.

The `stale_memory_followed` denominator is every complete valid
`remem_shared` tuple in the direction whose hashed candidate inventory,
captured from its sealed target-transfer projection immediately before the
production freshness/validity filter, has at least one query-relevant
stale/superseded challenge item that would otherwise match. Projection
sanitization cannot remove a registered challenge based on freshness or
validity. The inventory is evaluator-only evidence and is never a target memory
surface. The numerator additionally requires that item to be cited/used and
have a `causal_oracle_v1 = proven` record for the wrong action. Missing
inventory-stage proof, challenge inventory, or causal evidence makes the
metric and verdict insufficient. The report also shows all 36 `remem_shared`
tuples per direction so the applicable set cannot be hidden.
Each `stale_superseded_decision` task registers a challenge for all three runs,
so a claim-bearing report requires at least three applicable tuples per
direction. A missing expected match or a smaller/empty applicable set forces
`INSUFFICIENT`; it is not a passing zero.
Wrong-project injection, source-private-session leak, and key exposure each map
to `security_breach`/`partial_security`/safety `FAIL` regardless of completeness;
only stale-memory-followed and memory-hurt are non-security stop-losses.

Verdict precedence is unique:

1. verified redacted security breach -> safety `FAIL`;
2. missing identity, incomplete/invalid evidence or pairs, non-security partial
   manifest, or missing causal/applicable data -> comparative `INSUFFICIENT`;
3. complete-valid evidence exceeding a non-security stop-loss -> `FAIL`;
4. any rank-98,750 adjusted regression upper bound `< 0` -> `FAIL`;
5. all four rank-2,500 descriptive lower bounds `> 0` -> `PASS`;
6. every other complete result -> `INSUFFICIENT`.

No metric with an empty applicable set is serialized as zero.
The four-comparison PASS is an intersection-union claim; comparisons cannot be
selected after evidence collection. A separately publishable subset requires a
new pre-registered multiplicity contract.

## Live Execution and Cost Safety

The v2 live runner supports one operator and one execution root. It requires:

- an offline-generated canonical plan;
- exact Git head, plan, fixture, executable, profile, and tuple hashes;
- explicit `--confirm-live-run`;
- non-zero `--max-host-calls`, `--max-llm-calls`, and
  `--max-estimated-cost-usd`;
- a local exclusive lock for the execution root;
- streaming counters that stop before the next call would exceed a cap.

The caps are per invocation. The design makes no false claim that a local file
or Git ref provides a global cross-clone budget. There is no reusable approval
key and no same-repository ledger credential.

Ordinary CI, pull-request checks, dry-run, verification, and report rendering
must reject live host/provider calls. Smoke and full matrix commands require
separate explicit human invocations. The full matrix is not authorized by a
successful smoke.

If future requirements add concurrent operators or cross-clone resume, a
separate broker must expose an enforceable authenticated reserve/settle API
whose credentials are unavailable to host processes. That capability is a new
security contract, not an implicit extension of this benchmark.

## Sanitized Release Evidence

Raw source stores, host sessions, credentials, hidden tests, and private roots
are never committed. The release bundle contains enough sanitized data to
recompute every reported numerator, denominator, interval, cost, attribution
status, stop-loss, and verdict:

- all selected primary and diagnostic run records;
- immutable prior-attempt summaries and selection policy;
- complete or partial manifest;
- task/fixture/scorer/schema/version hashes, maintained-export boundary and
  budget records, and native-neutral/import lineage;
- sanitized causal inputs/results, exact bootstrap algorithm bytes/hash, and
  sampled-index/quantile vectors needed to recompute stop-losses and intervals;
- projection logical/ciphertext hashes, key IDs, and rekey/provisioning
  protocol hashes, but never source or projection key bytes;
- candidate JSON and deterministic Markdown reports;
- verdict binding both report hashes and renderer version.

The scanner validates the bundle before report construction and again before
publication. A clean checkout can recompute the report and verdict without
host credentials or private artifacts.

## Implementation Slices

These are sequential handoffs, not permission to run live hosts:

1. **Executable contracts and fixtures**
   - bump charter/schemas;
   - build the 24 deterministic tasks;
   - validate export/causal protocols, exact 288/144 plans, and partial
     manifests.
2. **Isolation, source seal, and runner**
   - reuse coding-bench process/HOME restrictions;
   - implement same-canonical-path sequencing, quiesced full-store sealing,
     native-neutral derivation, private SQLCipher rekey/provisioning,
     byte-identical projection fan-out, fixed attempts, cleanup, and cost caps.
3. **Condition surfaces**
   - implement no-memory, target-native negative control, maintained export,
     production remem, and source-native import diagnostic boundaries.
4. **Artifacts, report, and verdict**
   - implement causal records, complete/partial manifests, exact paired
     bootstrap/verdict predicates, deterministic rendering, and sanitized
     bundles.
5. **Evidence and status**
   - run a separately authorized smoke that exercises both directions and all
     six required primary/native-import surfaces, including both importer
     with/without paths;
   - only after smoke review, optionally authorize the full matrix;
   - publish PASS/FAIL/INSUFFICIENT artifacts and update every current-status
     document in the same result PR.

Shared schema or report files have one owner at a time. Parallel lanes may use
disjoint files, but ownership transfers before another lane edits a shared
file.

The production user-identity capability is an external prerequisite. It needs
its own current contract and implementation review; GH935 may exercise it only
after it exists.

## Verification

Current v1 infrastructure checks:

```bash
PYTHONDONTWRITEBYTECODE=1 \
  python3 eval/cross-host/scripts/schema_validate.py --self-test
PYTHONDONTWRITEBYTECODE=1 \
  python3 eval/cross-host/scripts/scan_artifacts.py --self-test
PYTHONDONTWRITEBYTECODE=1 \
  python3 eval/cross-host/scripts/run_dry.py
```

Future executable-version focused coverage must include:

- exact 288/144 tuple planning, missing/duplicate rejection, and dry-run zero
  host spawns;
- source episode single-execution, seal/clone/hash drift, interruption,
  new-attempt behavior, and retry-safe tuple/attempt key lifetime;
- same canonical project path positive transfer and decoy path exclusion;
- pre-open sealed/clone hash checks, disposable-open preflight, full-store
  denial, raw-free searches, and correct/wrong/source/missing-key checks;
- exporter immutability, atomic commit/retry/failure scope, fixed protocol and
  budget rules, condition isolation, and target-native source-seal denial;
- counterbalanced schedule equality and planned/realized-order drift;
- maintained-export cost and native import pairing, including neutral counts,
  complete Inserted-to-memory lineage, and raw-free projections;
- causal-oracle pre-action/counterfactual positive, refuted, Stop-only, ambiguous/missing cases, and failed-run retention;
- automatic-only primary review, exact source/target attempt references, and
  exact pre-prompt retry selection;
- required stale challenge pre-filter inventory and empty-applicable-set
  rejection;
- complete, non-security partial, and security partial reports;
- deterministic Markdown regeneration and hash-bound verdict;
- fixed bootstrap framing/rejection/quantile vectors, adjusted regression, and
  PASS/FAIL/INSUFFICIENT edges;
- secret-channel topology and raw/hex/versioned key residue scans;
- user-scope `not_testable` rejection of fabricated zero;
- per-invocation call/cost cap enforcement and CI live-call denial.

Before any implementation PR is submitted:

```bash
cargo fmt --check
cargo check
cargo clippy --all-targets -- -D warnings
cargo test
python3 scripts/ci/check_plugin_version_sync.py
```

Live smoke or matrix output is not a substitute for these tests, and these
tests are not authorization for live execution.
