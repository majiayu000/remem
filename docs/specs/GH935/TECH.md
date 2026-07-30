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
- source-native snapshot policy for the diagnostic arm;
- per-host executable/profile requirements;
- `status: "ready"` and `todo: []`.

The task validator rejects empty values, aliases, direction/host mismatch,
target-visible hidden paths, and any canary copied into prompts or gold facts.

### Source Seal

One source seal exists per `(direction, task_id, run_index, attempt_id)`. It
contains:

- task/fixture, executable, profile, model, schema, and migration hashes;
- source host/session/event IDs and transcript/tool-event/Git hashes;
- canonical project path and the exact `project_from_cwd` value;
- terminal extraction/review state and queue counts;
- a sorted manifest for the quiesced `REMEM_DATA_DIR`;
- a deterministic store archive hash, length, and creation policy;
- source-host native file manifest/hash, or typed absence;
- export generation/update/freeze hashes and cost records;
- creation/cleanup timestamps and scanner status.

The store archive is private execution material, not a committed public
artifact. In v2, a source and all dependent targets run in one
single-operator execution root. The archive is content-addressed, read-only
after sealing, and retained until every target fan-out and verifier step for
that source seal completes.

Cross-clone continuation is deliberately unsupported. If the execution root or
archive is missing, hash-mismatched, or interrupted before fan-out completes,
the runner writes a partial attempt record and starts a new full source attempt;
it never regenerates a store under the old seal. This avoids pretending a
machine-local archive is a durable cross-clone object. A future cross-clone
mode would require a separately specified encrypted immutable store, retention
policy, access protocol, and independently verified fetch path.

### Run Record

Each target attempt records:

- tuple key, attempt ID, condition, direction, task, and run index;
- source seal hash, even when the condition cannot read its contents;
- task prompt, condition surface, executable/profile, and scorer hashes;
- target HOME/config/session/workspace identities;
- status: `success`, `ordinary_failure`, or `security_breach`;
- resolved and secondary metrics with nullable values and reasons;
- host calls, LLM calls, tokens, wall time, turns, and estimated cost;
- cleanup and leak-scan results;
- attribution refs or typed `absent_due_to` for every production stage.

A record can be schema-valid while its task outcome failed. Failed records stay
in the registered denominator.

### Evidence Manifest

The manifest has one of three closed kinds:

```text
complete
partial_non_security
partial_security
```

Every kind contains the planned tuple set, recorded attempts, selected-attempt
policy, missing/not-started tuples, artifact hashes, and reason codes.

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

1. validates the charter, schemas, all 24 tasks, fixtures, and scorer paths;
2. proves 288 primary and 144 required diagnostic tuple keys with no
   duplicates;
3. resolves exact host/model/profile binaries and hashes;
4. calculates upper bounds for host calls, LLM calls, and estimated cost;
5. writes a canonical plan hash without starting a host or network call.

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
target-blind exporter runs against only the allowed source evidence: it creates
the handoff after episode one, updates it after every later episode, and records
generation/update cost before the next episode starts. After the final episode,
the runner:

1. freezes the maintained export and records its final hash;
2. drains extraction and required review/promotion work to a recorded terminal
   state;
3. stops the worker, checkpoints the database, closes writers, and fsyncs the
   run root;
4. snapshots the exact remem store and source-host native files;
5. seals all hashes and makes the store archive read-only;
6. verifies a fresh private clone before launching a target.

Failure to quiesce, seal, clone, or verify produces an ordinary failure or
security record. The runner cannot continue with a regenerated or partially
copied store.

### 4. Fan Out Conditions

Targets run serially in fresh HOME/config/session roots. Each condition gets a
fresh fixture reset and only its declared memory surface.

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
envelope is frozen before the target task is revealed.

The runner exposes the envelope through the same versioned system/context
adapter for Claude Code and Codex. The target task prompt remains identical;
the separately hashed envelope is the treatment surface. A condition-only task
prompt note is forbidden.

The envelope contains only allowed handoff facts and provenance, never raw
transcripts, tool logs, hidden paths, or the foreign-project canary.

#### `remem_shared`

The target receives a fresh verified clone of the sealed automatic-capture
store. Retrieval runs through the production SessionStart/MCP/Context Bundle
path. Direct memory inserts, gold seeding, manual saves, and special eval-only
retrieval are rejected by attribution validation.

#### Source-Native Import Diagnostic

The runner captures actual native-memory files produced by the **source host**
before cleanup:

- Codex source: supported Codex rollout-summary input;
- Claude Code source: supported Claude topic-file input.

Both diagnostic arms start from the same automatic-capture store clone and the
same sealed source-native snapshot. The without arm does not import. The with
arm invokes the shipped importer/ingestion path: Codex input uses the existing
dry-run plan digest, while Claude input is bound to the sealed native snapshot
and runner plan because its shipped ingestion path has no dry-run digest. The
arm then applies a pre-registered target-blind review/promotion decision while
preserving `host_native_import` origin and external trust.

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
6. destroys target HOME/session roots and verifies cleanup.

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

The report uses all selected attempts under one pre-registered attempt policy;
it cannot choose successful retries after seeing outcomes.

`memory_hurt` is the paired predicate:

```text
no_memory.resolved = true
AND remem_shared.resolved = false
AND remem attribution proves an injected/cited/used memory caused the error
```

Its denominator is all complete valid `no_memory`/`remem_shared` pairs in the
direction. Missing causal attribution makes the result insufficient.

`stale_memory_followed` requires a cited/used stale or superseded item and a
causal wrong action. `wrong_project_injection` and
`source_private_session_leak` are zero-tolerance.

Verdict precedence is:

1. verified security breach or exceeded stop-loss -> `FAIL`;
2. missing production user-scope prerequisite, incomplete matrix, invalid
   pair, missing attribution, or non-security partial manifest ->
   `INSUFFICIENT`;
3. complete evidence and all safety checks pass -> statistical wording rules
   decide `PASS` or directional/insufficient wording.

No metric with an empty applicable set is serialized as zero.

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
- task/fixture/scorer/schema/version hashes;
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
   - extend offline validation to exact 288/144 plans and partial manifests.
2. **Isolation, source seal, and runner**
   - reuse coding-bench process/HOME restrictions;
   - implement same-canonical-path sequencing, quiesced store sealing,
     byte-identical fan-out, attempts, cleanup, and cost caps.
3. **Condition surfaces**
   - implement no-memory, target-native negative control, maintained export,
     production remem, and source-native import diagnostic boundaries.
4. **Artifacts, report, and verdict**
   - implement attribution, complete/partial manifests, paired bootstrap,
     deterministic JSON/Markdown, hash binding, and sanitized bundles.
5. **Evidence and status**
   - run separately authorized smoke;
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
- source episode single-execution, store seal/clone/hash drift, interruption,
  and new-attempt behavior;
- same canonical project path positive transfer and decoy path exclusion;
- condition-surface contamination and target-native source-seal denial;
- maintained export cost and source-native with/without import pairing;
- typed attribution absence and failed-run denominator retention;
- complete, non-security partial, and security partial reports;
- deterministic Markdown regeneration and hash-bound verdict;
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
