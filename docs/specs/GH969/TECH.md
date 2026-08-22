# GH969 Technical Contract — Stabilization And Surface Governance

Status: Current contract; activation boundary implemented; later guard slices pending; Issue: #969

Last reconciled against `origin/main`: 2026-08-21 (`86fee409`)

## Contract Boundaries

This file defines the cross-module mechanisms required to enforce the product
contract. It does not supersede the schema, API, or rollout details in narrower
current specs. The implementation must reuse existing provenance, operation,
candidate, poisoning, and audit primitives before adding a parallel store or
planner.

## Current Runtime Truth

The current source paths are:

```text
host evidence
  -> captured_events / event_blobs / extraction_tasks
  -> observation or session-rollup extraction
  -> governed candidates + source/evidence/trust metadata
  -> review or bounded auto-promotion
  -> curated memories / relations
        ├-> retrieval service -> ordinary or explicitly routed MCP search
        │                      (no CurrentTruth or Context Bundle audit)
        └-> context loaders
              ├-> default `bundle` render mode
              │     -> Context Bundle + CurrentTruth v1 projection
              │        ├-> SessionStart + durable bundle audit
              │        └-> experimental MCP context_bundle response
              │            (no durable SessionStart audit row)
              └-> explicit `legacy` rollback render mode
                    -> legacy relevance compilation + CurrentTruth-governed
                       Core rendering (no durable bundle audit)
```

The path intentionally keeps capture cheap but not optional. Capture does not
create durable active memory directly. LLM output is treated as derived content
and cannot raise the trust of its sources.

The first implementation slice now consolidates production activation through
`memory::activation`, backed by the immutable
`memory_activation_requests` ledger introduced in schema v86. Route adapters
bind trust, provenance, payload digest, exact supersede targets, and poisoning
verdict before the active mutation runs in one savepoint. The boundary then
compares the stored payload/evidence fields to the request, rechecks poisoning,
and records a result digest; an inactive result cannot satisfy an idempotent
replay. Supplemental saves also bind an immutable claim receipt (`saved` with
the original claim id, `disabled`, or `failed` with its diagnostic) into the
same ledger insert so retries reproduce the first durable outcome. A semantic
no-op records the weaker incoming caller in activation evidence but never
relabels the already-active row's trust or acknowledgement metadata. The
activation request therefore binds incoming source trust separately from the
expected result-row trust; route policy validates the former while the durable
postcondition validates the latter.
Best-effort backup normalization uses governed `backup_import` rather
than claiming `ExactRecovery`. A repository-owned
CI guard inventories reviewed raw implementations and rejects new production
bypasses. Later slices still need the surface lifecycle and dependency-direction
guards described below.

## Target Module Direction

Allowed compile-time dependencies point from outer orchestration toward inner
policy/storage/foundation layers:

```text
host adapters, hooks, CLI, MCP, REST
        ↓ depends on
application workflows
        ↓ depends on
curated-memory and retrieval primitives
        ↓ depends on
storage, migrations, immutable ledgers
        ↓ depends on
foundation and identity types

evaluation and doctor may observe lower layers;
lower layers never depend on evaluation or user-facing adapters.
```

The initial ownership map uses non-overlapping top-level roots. Nested DTOs
inherit their root's layer until a reviewed extraction moves them; the guard
does not classify `truth` or `context_bundle` differently based on which child
file is referenced.

| Layer | Primary roots | May depend on |
|---|---|---|
| Foundation/domain | Roots `atomic_file`, `build_info`, `git_util`, `identity`, `log`, `perf`, `project_alias`, `project_id`, `runtime_config` | Standard/library utilities and other foundation roots |
| Storage | Roots `captured_git`, `db`, `git_evidence`, `git_trace`, `migrate`, `spill_queue`, plus `src/migrations/**` | Foundation/domain |
| Memory/retrieval | Roots `graph_candidate`, `memory`, `memory_candidate`, `retrieval`, `rules`, `truth`, `user_context`, `workstream` | Foundation/domain and storage |
| Application | Roots `ai`, `context`, `context_bundle`, `dream`, `extraction_worker`, `ingest`, `maintenance`, `observation_extract`, `retrieval_router`, `session_activity`, `session_rollup`, `summarize`, `timeline`, `worker` | Foundation/domain, storage, memory/retrieval |
| Adapters | Roots `adapter`, `api`, `cli`, `cursor_hook`, `hook_cli`, `hook_integrity`, `hook_runtime`, `hook_stdin`, `install`, `mcp`, `observe`, plus `src/main.rs` and `src/bin/**` | Application and all inward layers |
| Evidence/diagnostics | Roots `eval`, `doctor` | Read-only access to lower layers and explicit test/harness adapters; never a runtime dependency of inward layers |

This map describes the target, not a claim that current main is acyclic.
Splitting crates before the guard demonstrates stable inward boundaries is
out of scope.

The implementation derives the exhaustive production-root set from `src/lib.rs`
plus `src/main.rs`, `src/bin/**`, and `src/migrations/**`. Every discovered root
must match exactly one row; an unknown or multiply assigned root is a guard
error. Feature gates affect whether an edge is active for a profile, never
whether its root is classified.

## Dependency-Direction Guard

The implementation must add a repository-owned guard, invoked by preflight and
CI, with these properties:

1. Collapse Rust source references into the owned roots above.
2. Record a reviewed baseline containing each currently accepted reverse edge
   and its source sites.
3. Reject a new reverse edge, a new source site on an accepted reverse edge,
   or an increase in the largest cyclic component.
4. Permit an exception only through an explicit allowlist record containing
   owner, rationale, tracking issue, and expiry/decision date.
5. Ignore test-only edges only when the parser can prove they are under a test
   cfg/module; do not hide production edges based on path naming alone.
6. Print actionable `source -> target`, source file, violated layer rule, and
   remediation/allowlist guidance.
7. Update the baseline only when the measured violation set shrinks or a
   reviewed temporary exception is added. Regeneration cannot silently accept
   all current output.

The guard does not need to parse full Rust semantics in v1. A conservative
scanner is acceptable if self-tests cover `use crate::x`, absolute
`crate::x::...`, grouped imports, `pub use`, test cfgs, comments/strings, and
module-relative references that cross an owned root.

## Active-Memory Activation API

The target logical boundary lives under the curated-memory application owner,
not under `db`. Names below are normative concepts; an implementation may
choose equivalent Rust names if the contract stays explicit.

```rust
struct ActiveMemoryWriteRequest {
    operation_id: String,
    actor: MemoryActor,
    source_operation: MemorySourceOperation,
    source_project: String,
    target_project: Option<String>,
    branch: Option<String>,
    owner_scope: OwnerScope,
    owner_key: String,
    scope: MemoryScope,
    trust: SourceTrustClass,
    provenance: ActivationProvenance,
    payload: CuratedMemoryPayload,
    poisoning: PoisoningEvidence,
    supersede: SupersedePolicy,
}

enum ActivationProvenance {
    SupplementalSave {
        caller_evidence: SupplementalCallerEvidence,
        request_digest: String,
    },
    Candidate { candidate_id: i64, evidence_digest: String },
    PackImport { manifest_digest: String, entry_digest: String },
    ExactRecovery { plan_id: String, prior_memory_id: i64 },
}
```

The operation/request fingerprint includes the branch option's presence and
normalized value; `None` and an explicit branch never share an idempotency or
supersede/no-op identity accidentally.

`SupplementalCallerEvidence` is constructed by a trusted adapter, not parsed
from a caller-supplied `caller` string. It is a closed union of a bound
UserPrompt captured-event identity/digest, an independently issued human-review
attestation bound to the exact content digest, or an agent/tool invocation
bound to host, session, and invocation digest. The human attestation issuer is
not reachable with the MCP/app/API bearer token used by agents. Transport auth,
an OS process identity, a TTY, or a request parameter alone never establishes
human provenance. MCP and model-facing API `save_memory` default to the agent
form. Only the bound UserPrompt or independent human-review forms may establish
`user_prompt`; all other missing/unverifiable evidence uses `external_content`.

The boundary must, in one transaction/savepoint:

1. validate closed enums and canonical identity;
2. verify route/project/presence-sensitive normalized branch/owner consistency;
3. validate source trust and required evidence;
4. require applicable source/generated poisoning scans;
5. reject quarantined or unacknowledged risky input;
6. calculate and validate the exact supersede/no-op set;
7. create/update the active row and derived indexes;
8. persist source trust, provenance, operation log, and lifecycle changes;
9. persist any route-specific response receipt in the same savepoint;
10. make the same `operation_id` idempotent and reject a conflicting replay.

An error rolls back every activation side effect. Logging an error after an
active row was committed is not an acceptable failure mode.

### Route Rules

| Route | Required proof | Forbidden behavior |
|---|---|---|
| Supplemental save | Exact payload scan, server-constructed caller evidence, trust derived from authenticated/bound evidence, and an immutable claim receipt for exact replay | Treating a request field or MCP/agent call as user attestation; weakening an existing row on semantic no-op; exposing human acknowledgement through an agent save schema; or fabricating a replay response when the original receipt is missing |
| Candidate auto-promotion | Current auto-promotion decision, evidence binding, trust at or above the current policy threshold, no poison hit | Bypassing candidate policy through `insert_memory*` |
| Candidate manual approval | Review identity/token and immutable candidate/provenance digest | Approval of a changed candidate or unbounded Dream supersede set |
| Dream | Candidate plus exact source-memory provenance and generated-surface verdict | Direct active insert/update or superseding source rows on quarantine/failure |
| Governed pack import | Verified manifest/content/entry digests, instruction scan, `pack` trust, target repo ownership, and the safe-add/no-resurrection decision from `project-memory-pack/` | Activating a conflict, quarantined row, changed plan entry, or locally suppressed/invalidated identity |
| Other import | Import plan/digest and candidate route unless its current contract proves an equivalent governed activation | Direct activation based only on file ownership or format validity |
| Recovery | Exact prior row identity, plan/apply digest, acknowledgement where required | Creating a semantically new claim or raising trust |

### Bypass Guard

CI must inventory production call sites that can set `memories.status` to
`active`, call raw active insert/update helpers, or execute equivalent SQL.
With the activation service in place:

- normal production call sites must route through it;
- non-activating migration DDL, test-only scaffolding, and the activation
  implementation itself may be allowlisted by exact file and reason;
- any migration or recovery operation that inserts/updates an active row must
  invoke the boundary with governed import or `ExactRecovery` provenance;
- a new call site or raw SQL pattern fails with a user-readable error;
- the guard has positive and negative self-tests;
- reviewed raw sites are pinned by normalized statement/helper signature and
  occurrence count, so an allowlisted file cannot silently gain another bypass;
- dynamic SQL or helper renaming cannot be used to evade review; ambiguous
  matches fail for manual classification rather than passing silently.

## Scope, Owner, And Relation Integrity

All replacement/suppression decisions use a typed subject identity containing
at least owner scope/key, project route, memory scope, and subject/state key.

- `supersedes` and `refutes` may select a winner only when both endpoints
  resolve to the same typed subject identity.
- Cross-subject `supports`, `derived_from`, or `applies_to` may remain
  provenance but cannot suppress a winner.
- A missing, malformed, or out-of-scope endpoint cannot suppress an in-scope
  claim.
- Production relation reads are bounded, deterministic, and return visible
  errors on schema/query failure.
- Current v1 context isolation is enforced by #1038. Typed persisted identity,
  history replayability, and breaking migration remain under GH933 v2.

## Surface Manifest And Consistency Guard

The PRODUCT inventory is canonical until a machine-readable manifest lands.
The manifest must cover every listed entry point and include:

```text
id, surface_kind, owner, status, public_entry_points, real_callers, default_state,
spec_refs, eval_commands, compatibility, rollback, decision_due
```

`surface_kind` is a closed discriminator such as `rust_export`, `mcp_tool`,
`rest_route`, `cli_command`, `default_feature`, `offline_harness`, or
`spec_contract`. An `offline_harness` record declares exact checked roots and
must resolve every named executable script, schema, and required fixture/data
inventory under those roots; the guard validates its documented invocation
without treating it as a Rust production caller. This is the entry type for the
current `eval/cross-host/` infrastructure.

The CI guard must reject:

- an unknown status or missing owner/spec; staged, experimental, deprecated,
  and spec-only entries also require a dated decision (a `staged` row may bind
  it to the next release only when that release version is explicit in the
  manifest);
- a public MCP/CLI/REST tool or default-on/staged feature absent from the manifest;
- a reachable Rust `pub mod`, public item, or `pub use` absent from the
  manifest; discovery starts at `src/lib.rs` and recursively follows public
  module/re-export paths, while private modules remain implementation detail;
- any non-`spec-only` manifest entry that no longer resolves according to its
  `surface_kind`: a discovered Rust/MCP/REST/CLI/default-feature surface, or
  for `offline_harness`, the exact declared scripts, schemas, fixtures/data,
  and checked command under its repository roots;
- a manifest `production` entry with no production caller;
- an `experimental` entry used by a default production path without a
  separately classified production entry point;
- a `recovery-only` entry with a normal new-work writer or an undeclared
  automatic/manual caller;
- an overdue experimental decision without an integrate/continue/remove
  update;
- conflicting lifecycle wording between the manifest, specs index, served tool
  description, README, and Cargo feature default.

Generated documentation may render from the manifest, but generators must not
silently rewrite high-context files. Drift is reported and fixed in a reviewed
change.

## Unified Ship Matrix

The executable matrix must distinguish these gates:

| Gate | Primary command/artifact | Blocks |
|---|---|---|
| Deterministic retrieval | `cargo run -- eval-gates --json-out /tmp/remem-eval-gates.json` and relevant channel decision reports | Merge of retrieval behavior that violates checked-in quality/safety thresholds |
| Capacity | capacity axis in `remem eval-gates` | Merge/default changes that degrade as store size grows beyond its contract |
| SessionStart | SessionStart/context gate in `remem eval-gates` plus Context Bundle audit tests | Merge/release of context changes with relevance, budget, safety, or audit regressions |
| Production security E2E | memory-bench production governance diagnostics | Merge/release of trust/quarantine behavior not proven from persisted production state |
| Cross-host | GH935 sealed matrix | Cross-host continuity claims |
| Coding outcome | GH931 official matrix and stop-loss artifact | Default/public claims about coding-task improvement |
| Public claim | Hash-bound public report plus independent verification/authority | Any comparative, superiority, or SOTA wording |

Each report must include implementation SHA, dataset/fixture hash, config and
model identity, environment/platform, condition completeness, metric deltas,
stop-loss verdict, exclusions, and claim level. A missing arm is `incomplete`,
not zero and not pass.

## Outcome Scorecard

The implementation should extend an existing eval/report artifact rather than
create an unrelated dashboard. Required fields are:

```text
task_completion_rate
correct_memory_help_rate
repeated_explanation_rate
wrong_memory_injection_rate
stale_memory_injection_rate
cross_scope_injection_rate
poison_policy_leak_rate
abstention_rate
foreground_latency_p50_p95
maintenance_time_and_ai_usage
```

Every field must declare its numerator, denominator, eligible population, and
whether it is measured, unavailable, or not applicable. Unavailable fields do
not pass a gate. Security leak metrics have a zero-tolerance threshold unless a
narrower contract is stricter.

## Migration And Compatibility

### Schema changes

1. Add new columns/tables/indexes without changing active readers.
2. Ship readers that accept both staged and completed state.
3. Rehearse a bounded plan/apply backfill on an encrypted copy where relevant.
4. Enable the new writer only after validation.
5. Retire compatibility reads/writes only after residual-state and release
   evidence meet the owning contract.

Schema migrations are forward-only. Operational rollback uses backup/restore,
feature/default rollback, or dual-readable staged state; it does not execute an
untested reverse migration over user data.

### Public/API changes

- Additive fields remain additive and must preserve legacy text when the
  owning contract requires byte compatibility.
- Experimental versioned APIs may make breaking changes only with an explicit
  schema/policy version bump and updated served description.
- Production breaking changes require migration/cutover/rollback documents and
  a release boundary.
- Removing a deprecated or recovery-only surface requires usage/residual-state
  evidence and release notes.

## Implementation Slices

After this spec is accepted, create focused implementation issues rather than
one cross-repository PR:

1. **Active-memory activation boundary and bypass guard**
   - Status: implemented by #1040 for the v0.6.82 release line.
   - Primary scope: `src/memory/`, candidate promotion callers, direct-save
     callers, and a dedicated CI check.
   - Acceptance: every production activation route is classified; bypass
     self-tests and route regressions pass.
2. **Surface manifest and lifecycle consistency guard**
   - Primary scope: manifest, served surface inventory, specs-index/README
     consistency check.
   - Acceptance: every PRODUCT inventory row is represented; stale/overdue or
     contradictory declarations fail CI.
3. **Module dependency-direction baseline and no-expansion guard**
   - Primary scope: module scanner, reviewed baseline, CI/preflight wiring.
   - Acceptance: self-tests pass; current violations are visible; synthetic new
     reverse edges and cycle growth fail.
4. **Executable ship matrix and outcome scorecard**
   - Primary scope: existing eval gate/report aggregation.
   - Acceptance: gate scope and claim level are machine-readable; unavailable
     measures cannot appear as pass.
5. **Architecture/current-spec synchronization**
   - Primary scope: `docs/ARCHITECTURE.md`, specs index, manifest-linked drift
     checks.
   - Acceptance: production CurrentTruth/Context Bundle flow and all lifecycle
     classifications match callers.

GH933 v2, GH931 governed official runs, and GH935 completion retain their own
contracts and issues; these slices must link rather than duplicate them.

## Validation

Spec-only PR (the preflight supplies the same PR body to the lifecycle guard):

```bash
python3 scripts/ci/check_pr_preflight.py --base origin/main \
  --pr-body-file /tmp/pr-body.md
```

Each implementation slice runs focused tests first, then at minimum:

```bash
cargo fmt --check
cargo check
cargo test
cargo clippy --all-targets -- -D warnings
python3 scripts/ci/check_plugin_version_sync.py
node --test plugins/remem/scripts/remem-runtime.test.js \
  plugins/remem/apps/remem/server.test.js \
  npm/remem/scripts/install.test.js
cargo run -- eval-extraction --json --check-baseline
cargo run -- eval-gates --json-out /tmp/remem-eval-gates.json
```

Binary-impacting implementation PRs also satisfy the version-bump guard.
Passing historical output from another SHA is not completion evidence.

## Rollout And Close Audit

The epic close audit must produce one table mapping every #969 checkbox to:

- implementation PR and final SHA;
- current spec section;
- focused regression test or immutable artifact;
- lifecycle status and decision date;
- rollback path;
- unresolved external dependency, if any.

Repository-local stabilization and external benchmark authority are separate.
GH931 may remain externally blocked without hiding unfinished local #969 work;
conversely, local code completion cannot manufacture the independent authority
required for a public claim.
