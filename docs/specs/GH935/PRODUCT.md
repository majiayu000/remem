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
source seal. The with arm alone imports the same sealed source-native snapshot
into remem, applies the pre-registered target-blind review/promotion policy,
and preserves external origin/trust.

Other diagnostic conditions may remain for debugging, but they never enter a
primary comparative claim.

## Completion Invariants

### 1. Tasks and Matrix

1. The executable `cross-host-v2` set contains 24 ready tasks: 12 per
   direction, covering
   the existing 12 categories in each direction.
2. A task becomes `ready` only when it has a deterministic fixture, at least
   two chronological source episodes, hidden tests, non-empty score commands,
   allowed/forbidden paths, gold facts, and an empty TODO list.
3. The complete primary matrix is exactly
   `24 tasks * 4 conditions * 3 runs = 288` unique tuples.
4. The complete source-native import diagnostic is exactly
   `24 tasks * 2 conditions * 3 runs = 144` unique tuples.
5. Missing, duplicate, substituted, schema-invalid, or unverified tuples make
   the comparative verdict `INSUFFICIENT`; they are never filled with zeroes.

### 2. One Source Episode, Many Target Conditions

For each `(direction, task_id, run_index)`, the source episode sequence runs
once. After automatic extraction reaches a terminal state, the runner seals:

- source transcript/tool-event and Git evidence hashes;
- the canonical project identity and fixture revision;
- a quiesced, content-addressed `REMEM_DATA_DIR` snapshot and sorted file
  manifest;
- the source-host native-memory snapshot, when present;
- executable, model, profile, schema, and migration versions.

Every dependent condition uses that exact source-episode seal. Each
`remem_shared` target receives a fresh byte-identical private clone of the
sealed store. Re-running a source episode for only one condition, regenerating
the store, or accepting a mismatched hash invalidates the pair.

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
  Generation and every maintenance cycle record tokens, wall time, turns, and
  byte/diff size.
- `remem_shared` uses the real automatic capture-to-retrieval path. Direct gold
  inserts, manual `save_memory`, target-visible hidden data, or preloaded
  answers invalidate the run.
- The source-native import diagnostic snapshots actual native files produced
  by the source host before toggling import. The with and without arms share
  the same source episodes, native snapshot, target task, and non-import
  configuration.

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
and cleanup failure are explicit outcomes. A retry has a new `attempt_id` and
cannot overwrite an earlier attempt.

For `remem_shared`, capture, extraction, review/promotion, selection, and
citation/use each contain either a resolvable reference or a typed
`absent_due_to`. An upstream failure may make downstream references absent,
but the failed run remains in the denominator.

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

Treatment/control comparisons use task-cluster paired bootstrap with a fixed
algorithm version, seed, confidence level, and task-level resampling. A missing
pair, config/hash drift, insufficient clusters, or a direction whose confidence
interval includes zero forbids a positive "improved" or "beats" statement.

The claim-bearing stop-losses are:

- `wrong_project_injection = 0`;
- `source_private_session_leak = 0`;
- `stale_memory_followed <= 1%`;
- `memory_hurt <= 2%`.

`memory_hurt` counts a paired tuple only when `no_memory` resolves the task,
`remem_shared` does not, and attribution shows injected/cited/used memory caused
the wrong action. Its denominator is all complete, valid paired
`no_memory`/`remem_shared` tuples in that direction. Missing causal attribution
makes the metric and verdict insufficient; it does not shrink the denominator.

`stale_memory_followed` counts a `remem_shared` tuple when a cited or used
stale/superseded item causes a wrong action. Metrics with no applicable data are
blank with a reason, never zero-filled.

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

- No live Claude Code/Codex session bridge.
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
  isolation, attribution, failure retention, and leak redaction have positive
  and negative tests.
- The production user-identity prerequisite is either implemented and tested,
  or every public comparative verdict is deterministically `INSUFFICIENT`.
- A separately authorized smoke covers both directions and all four primary
  surfaces before any full-matrix authorization.
- Complete and both partial manifest forms regenerate deterministic JSON,
  Markdown, and verdict artifacts.
- Current documentation is updated for every final verdict.
- No public positive claim appears before a complete verified `PASS`.
