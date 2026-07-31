# Cross-Host Continuity Benchmark — Product Contract

Status: Current contract; v1 infrastructure shipped, completion unimplemented
Issue: #935 (refs #849, #852, #385)
## Current Truth

PR #937 shipped `cross-host-v1` infrastructure under `eval/cross-host/`:

- a versioned charter and task/run schemas;
- 12 `skeleton_todo` tasks in each direction;
- an artifact leak scanner and offline dry-run validator.

There are still no executable fixtures, runner, live runs, result bundle, or conclusion.
The charter remains `infrastructure_only_no_runs`. This defines remaining work;
it is not outcome evidence and does not authorize a
live run.

Root-level `specs/GH*/` packets are historical planning evidence. This
directory is the normative GH935 contract.

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

For each direction, the source host creates history before any byte of the
target's complete host-channel prompt stream is revealed. Every target
condition receives the same fixture revision, task-prompt segment, scoring
commitment, model/profile configuration, and source-episode identity. The
complete stream includes every benchmark-controlled system, developer,
context/memory, task, tool prelude, separator, and framing byte written to a
prompt-bearing host channel; only the declared memory-surface segment differs.

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
| `remem_shared_startup` | The source host's automatic capture, extraction, review/promotion, followed by the target host's production SessionStart/Context Bundle selector only. Interactive MCP retrieval is excluded and this condition must never be described as the full normal production retrieval path. | Claim-bearing startup-treatment comparison; not a claim about the complete remem product path. |

`target_host_native` may be empty in a fresh target HOME. That is intentional:
it proves source history does not appear in an unrelated native store. Reports
must not relabel it as a populated native-memory comparison or claim that remem
beats native cross-host transfer.

The native-import diagnostic compares
`remem_without_host_native_import` with
`remem_with_host_native_import`. In both arms, "host native" means the
**source host's** native-memory snapshot. The target host never receives a raw
source seal. The runner first derives `native_neutral_base_v1` from the full
production store without changing the primary `remem_shared_startup` evidence. That
base has no `claude_native`/`codex_native` candidate or transitive provenance.
Both arms start from byte-identical neutral-base clones and derive independent
raw-free target projections. The with arm alone imports the same sealed
source-native snapshot. Before import, the sealed plan classifies every native
record by production project identity as either `authorized_project` or the
plan-registered `foreign_project_canary`. Every authorized-project record must
return `status = "inserted"` and preserve the immutable chain
`import_attempt_id -> native_record_id -> candidate_id -> promoted review
outcome -> memory_id`, with `promoted = true` and
`memory.source_candidate_id = candidate_id`. Every registered foreign-project
canary must instead return `status = "excluded_wrong_project"` with a null
`candidate_id` and zero candidate, review, or memory rows. Each pair requires
at least one authorized inserted record. A relation/outcome mismatch,
duplicate, quarantine, promotion noop, pre-existing-memory reuse, or lineage
mismatch makes the pair `INSUFFICIENT`.

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
   Its `pre_run_scorer_commitment_v1` also freezes all result-affecting scorer
   semantics and the exact sanitized oracle bytes, or the one deterministic
   derivation that produces those bytes, before any live host/provider call.
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
once. When the host episodes succeed, the runner first durably seals
`condition_neutral_episode_evidence_v1` over the transcript/tool-event, Git,
workspace, fixture, and source-native manifest or typed native absence. This
common-source-success boundary does not depend on remem materialization and
cannot be rewritten by a later condition failure. After automatic extraction
and condition preparation reach terminal states, the source record also seals:

- the condition-neutral evidence hash;
- the canonical project identity and fixture revision;
- a quiesced, content-addressed full source `REMEM_DATA_DIR` snapshot and
  sorted data-only manifest; the source SQLCipher key remains in a separate
  runner-private secret root; or a typed `remem_preparation_failed` absence;
- a deterministic logical target-transfer projection derived from that
  snapshot, plus its independently rekeyed SQLCipher ciphertext hash and
  non-secret projection key ID,
  preserving the current database schema and production state flags while
  containing the curated rows needed to exercise production candidate
  filtering—including registered stale/superseded challenge rows—and no source
  transcript/session/capture rows or archives; or the matching typed absence;
- a sealed `native_neutral_base_v1` hash and proof that source-native
  candidates and their transitive provenance closure are absent, or a typed
  diagnostic-preparation absence;
- the source-host native-memory snapshot, when present;
- executable, model, profile, schema, and migration versions.

Every dependent condition uses that exact source-episode record. A
`remem_shared_startup` target receives a fresh byte-identical private clone only when
the sealed target-transfer projection exists; a typed preparation absence
terminates that condition under the failure rule below. The full source store
is never target visible. Re-running a source episode for only one condition,
regenerating either store, or accepting a mismatched derivation/hash invalidates
the pair.
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
- `remem_shared_startup` uses automatic capture-to-startup selection but not
  interactive MCP retrieval. Its primary
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
  import and target-blind review/promotion mutation before projection. Its
  closed per-record result must match the presealed project relation:
  authorized records resolve the full inserted candidate/review/memory lineage,
  while each registered foreign-project canary is excluded with no persisted
  candidate. An unknown relation, wrong outcome, duplicate, quarantine, noop,
  or pre-existing-memory reuse is insufficient.

For every condition, `host_channel_prompt_stream_v1` is rendered into one
immutable byte object before launch. Its ordered surface manifest binds segment
role/channel, offset, length, and SHA-256, including an explicit zero-length
hash for an absent memory surface. `condition_surface` is a framing closure:
its hash covers content plus every derived length prefix, checksum,
content-length field, separator, or wrapper byte changed by that content. The
task segment is byte-identical across conditions; the full stream may differ
only inside that closure, supplied by the exported envelope or plan-selected
production SessionStart/Context Bundle adapter. Claim-bearing v2 forbids a
later interactive MCP/context delivery of condition-specific bytes; all such
memory is in the committed initial stream. Every surface must label it
`remem_shared_startup`, say it measures only startup selection, and never
generalize to remem's SessionStart-plus-MCP path. An uncapturable host prelude makes the profile unsupported.

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
  prompt attempt, regardless of outcome; the only no-reveal selection is one
  non-retryable condition-preparation attempt created after common-source
  success under the rule below;
- semantic/task failure, post-reveal timeout, security breach, scope leak,
  scoring result, or cleanup failure cannot be retried into a better claim
  outcome;
- if all three target attempts exhaust eligible failures before prompt reveal,
  there is no selected claim attempt: the tuple is
  `missing_pre_prompt_exhausted`, `resolved = null`, and the candidate report is
  `partial_non_security` / `INSUFFICIENT`; no hidden score runs and no false or
  zero value is imputed;
- if all three source attempts fail before condition-neutral evidence is
  sealed, dependent tuples are recorded as not started and the candidate report
  is `partial_non_security` / `INSUFFICIENT`.

Immediately before spawning a process capable of writing prompt bytes or writing the first
byte of `host_channel_prompt_stream_v1`, the runner appends, fsyncs, and
hash-seals `prompt_reveal_committed`. It binds the target attempt ID, exact
full-stream hash/length, ordered surface-manifest hash, task-segment hash,
condition-surface hash/length, planned and realized sequence, adapter/framing
version, immutable-byte-object ID, commit timestamp, and prior journal hash.
The adapter sends from that object without rerendering and records a rolling
sent hash/length. Its presence means revealed even if a later send partly
writes or fails. No system, context/memory, task, tool-prelude, separator, or
framing byte may precede it.

For one attempt let `R` be the count of valid reveal commits, `N` the count of
valid sealed channel/launcher no-write proofs, and `T` the count of valid
terminal records whose attempt ID, prior-journal hash, and terminal class agree;
`M` counts malformed, unhashed, out-of-order, conflicting, or unexpected
artifacts, including ones otherwise excluded from `R`/`N`/`T`.
Selection uses the following closed table; it never invents a missing event or
picks a later better result:

| Evidence and terminal class | Attempt/tuple result | Selection and final evidence |
|---|---|---|
| `M = 0`, `R = 1`, `N = 0`, `T = 1` | `reveal_state = committed`; `resolved` is boolean only after the frozen-input scorer/oracle checks succeed, otherwise null. | This attempt is terminally `selected_claim_attempt`; no later attempt is allowed. Any null/unverified score makes the final manifest `partial_non_security` / `INSUFFICIENT`. |
| `M = 0`, `R = 0`, `N = 1`, `T = 1`, exact retryable reason, budget remains | `reveal_state = proven_no_write`; `resolved = null`. | No attempt is selected yet; exactly the next authorized attempt may run. The checkpoint manifest is `partial_non_security` / `INSUFFICIENT` until a terminal selection exists. |
| `M = 0`, `R = 0`, `N = 1`, `T = 1`, exact retryable reason, budget exhausted | Tuple is terminal `missing_pre_prompt_exhausted`; `resolved = null`. | `selected_claim_attempt = null`; no scorer or later attempt; final manifest is `partial_non_security` / `INSUFFICIENT`. |
| `M = 0`, `R = 0`, `N = 1`, `T = 1`, registered non-retryable post-common-source preparation failure (`terminal_reason="post_common_source_preparation_failed"`) | Terminal `ordinary_failure`; `resolved = false`. | This no-reveal attempt is selected and remains in the denominator; no later attempt. It may appear in `complete` evidence if every other invariant holds. |
| Any other combination, including `M > 0`, `T != 1`, duplicates, both reveal/no-write, or neither | Verifier-derived tuple terminal `reveal_evidence_invalid`; `reveal_state = conservatively_revealed_invalid`; `resolved = null`; conflicting attempt artifacts remain immutable. | The current attempt ID is terminally selected, all later attempts and hidden scoring are forbidden, and the final manifest is `partial_non_security` / `INSUFFICIENT`, unless a verified breach overrides with `partial_security` / `FAIL`. |

Every attempt terminal seal binds its reason, ordered journal root, every
reveal/no-write artifact including duplicates, counts, derived reveal state,
selection, `resolved`, and scorer refs. The verifier applies the table only
after finalization or crash recovery, rejects hash/class drift and
timestamp/status inference, preserves conflicting artifacts, and applies one
result to attempt, tuple, manifest, and verdict.

Every target record references exactly one `source_attempt_id` and has its own
`target_attempt_id`. All attempts remain immutable and visible in operational
reliability and cost metrics, including attempts exhausted before prompt reveal.

If condition-neutral evidence successfully seals, any failure confined to remem capture,
extraction, quiescing, full-store materialization, projection/rekey, clone, or
pre-open verification is `remem_preparation_failed`, never a source failure or
pre-prompt transient. Unless a verified security breach takes precedence, the
runner continues unaffected `no_memory`, `target_host_native`, and
`exported_file`, recording the affected `remem_shared_startup` tuple as
one terminal `target_attempt_id`: `ordinary_failure`, `resolved = false`, with
typed `absent_due_to` references and no retry. That condition-attributable
failure remains in the registered denominator; target-infrastructure exhaustion
above remains missing rather than failed.

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
a verified security breach or the narrow complete-direction lower-bound rule
below, makes the comparative metric/verdict `INSUFFICIENT`.

Origin is a closed set:
`remem_canonical_capture`, `host_native_import`,
`generated_projection`, and `manual_save`. Primary `remem_shared_startup` evidence may
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

The primary claim family is `remem_shared_startup` versus both `no_memory` and
`exported_file` in each direction. `target_host_native` is not a superiority
comparator. For direction `d` and comparator `c`, the registered estimand is:

```text
delta[d,c] =
  (1 / 12) * sum_task(
    (1 / 3) * sum_run(
      I[remem_shared_startup.resolved] - I[c.resolved]
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

Verdict order is unique: verified security breach -> safety `FAIL`; in an
identity-valid, otherwise complete direction, a verified numerator whose
conservative lower bound already exceeds its stop-loss despite only
causal/applicability gaps -> `FAIL`; missing identity, incomplete/invalid
evidence, a non-security partial manifest, or any other missing
causal/applicable data -> comparative `INSUFFICIENT`; only then may adjusted
regression yield `FAIL`, all-four improvement yield `PASS`, or the result remain
`INSUFFICIENT`.

The closed security-breach predicates are `wrong_project_injection > 0`,
`source_private_session_leak > 0`, `private_benchmark_byte_leak > 0`, any key
exposure, and, once the production user-scope prerequisite makes the metric
numeric, `wrong_user_injection > 0`. The private-byte predicate covers exact,
hex, standard/URL-safe base64 with and without padding, percent/JSON escaping,
the fixed archive/compression formats, normalization, and recursion limits in
normative `private_byte_encoding_registry_v1` in [`VECTORS.md`](VECTORS.md); a plan
may add but not remove coverage. Scanner failure, a limit breach, or an unclassifiable candidate yields
`partial_non_security` / `INSUFFICIENT`. A verified predicate produces
`security_breach` / `partial_security` / safety `FAIL` even with missing
identity or an incomplete matrix. The non-security claim-bearing stop-losses
are:

- `stale_memory_followed <= 1%`;
- `memory_hurt <= 2%`.

`memory_hurt` counts a paired tuple only when `no_memory` resolves the task,
`remem_shared_startup` does not, and `causal_oracle_v1` returns `proven` for the same
exposed memory and wrong action under the registered proof method. Its
denominator is all 36 registered paired `no_memory`/`remem_shared_startup` tuples in
that direction; a missing/invalid pair never shrinks it. Missing causal
attribution makes the rate blank and verdict insufficient unless identity,
manifest, pairs, and all non-causal evidence are otherwise valid and the count
of proven events divided by 36 exceeds 2%. Missing identity or partial evidence
can never use this non-security override.

`stale_memory_followed` uses a direction-specific applicable set: complete,
valid `remem_shared_startup` tuples whose hashed candidate inventory, captured from the
sealed target-transfer projection immediately before the production
freshness/validity filter, contains at least one query-relevant
stale/superseded challenge item that would otherwise match. Projection
sanitization must retain registered challenge rows with their production state
flags; this evaluator-only inventory is not exposed as a target memory surface.
The numerator is an applicable tuple where that item is cited or used and has
a `causal_oracle_v1 = proven` record for the wrong action. Missing
inventory-stage proof, challenge inventory, or causal evidence makes the metric
and verdict insufficient unless already proven events divided by the maximum
36 `remem_shared_startup` tuples exceed 1% in an identity-valid, otherwise complete
direction whose only gap is causal/applicability evidence. Missing identity or
partial evidence cannot use this override. Missing evidence does not remove a
tuple from the denominator. Reports show both the applicable count and all 36
tuples per direction. A claim-bearing report requires at least one registered
challenge across all three runs of the relevant task and therefore at least
three applicable tuples per direction; fewer or none forces `INSUFFICIENT`.
Empty metrics remain blank with a reason, never zero-filled.

Before the first live host/provider call, the canonical plan seals
`pre_run_scorer_commitment_v1`. It binds the exact hidden scorer engine/wrapper
bytes, runtime and invocation semantics, result-affecting
`scoring_semantics_ir_v1`, hidden-input manifest root, oracle-rule revision,
release revision ID, and either the exact already-built
`sanitized_scorer_oracle_v1` bytes or one deterministic derivation. A derivation
binds the sanitizer/deriver executable bytes, runtime/toolchain, canonical
static inputs, typed future freeze slots, output framing, and equivalence
vectors; concrete target outputs fill those slots only after freezing. The
same plan commits `scorer_read_contract_v1`, a deny-by-default set of allowed
paths, streams, metadata fields, environment, and ambient state. The scorer and
public oracle must call the same committed scoring IR; no post-run
implementation, branching, regeneration, output selection, or semantic
redaction is allowed.

After a target terminates and before any hidden scorer/test byte becomes
visible to a scoring process, the runner closes writers and freezes the final
workspace and complete target output as immutable, content-addressed objects.
`scoring_input_freeze_v1` binds their exact hashes, lengths, sorted manifests,
allowed exclusions, and freeze/scanner policy. The precommitted read-set
projector derives `public_scoring_projection_v1`; its manifest/hash, actual
read-set hash, and inclusion proofs bind every scorer-readable byte to those
frozen objects.
The hidden scorer and public oracle are sandboxed to byte-identical read-only
clones of that projection and cannot read the larger private root. The exact
projection is published after terminal sealing. The run record, selected-attempt record,
manifest, report, and verdict all bind the plan commitment, frozen input
hashes, scorer invocation/result hash, and oracle recomputation hash.

For every `committed` selected attempt, the release bundle must also publish
the complete, scanner-approved immutable `host_channel_prompt_stream_v1` byte
object and canonical RFC-8785 `prompt_surface_manifest_v1`; the registered
no-reveal preparation failure instead carries its typed absence. The manifest
uses plan-fixed segment IDs/order/ownership and exactly partitions
`[0, stream_length)` once, with no gap, overlap, duplicate, relabeling, or
post-run closure expansion. Verification independently slices the published
object, rehashes every segment and the whole stream, proves all bytes outside
the single `condition_surface` framing closure are byte-identical across the
paired conditions, and proves the terminal rolling sent hash/length equals the
published object exactly. Recorded hashes are inputs to compare, never trusted
as proof. A partial send, slice/root mismatch, out-of-closure difference,
unpublished/redacted byte, or scanner failure makes `resolved = null` and the
evidence `partial_non_security` / `INSUFFICIENT`; a verified private-byte leak
retains security precedence. Privacy or result sensitivity is never a reason
to omit a byte from claim-bearing evidence.

After all authorized attempts are terminally sealed, the core evidence set opens
the pre-run commitment: it contains the exact non-secret scorer
engine/scoring-IR and oracle bytes (or rerunnable deterministic derivation),
the minimum non-secret frozen fixtures/replay inputs, and inclusion/derivation
proofs needed to recompute every selected `resolved` and
`action_counterfactual_v1`. A result-affecting input that cannot be opened in a
sanitized form makes the task non-claimable; raw private roots, credentials,
transcripts, and irrelevant private test material remain non-public. A clean
checkout must verify published hashes against the pre-run commitment, verify
every scorer-readable projection object and its proof into the committed
full-freeze roots, rebuild under the committed Git/runtime/toolchain with no
network or ambient state when derivation was chosen, and recompute scores,
report, and verdict without trusting recorded `resolved`. An out-of-set read,
undisclosable input, missing object/proof, root mismatch, scorer/oracle
disagreement, or other tampered/unrunnable evidence yields
`partial_non_security` / `INSUFFICIENT`.

The oracle and its commitment opening remain unavailable to every target,
exporter, source host, and run-visible root until all attempts seal. Early
visibility is benchmark-integrity failure and yields
`partial_non_security` / `INSUFFICIENT`; exposure of registered private bytes
is instead a verified security breach.

Publication uses an acyclic two-layer object graph. First, the runner builds
and scans the closed `core_evidence_v1` member set: sanitized attempt/run
records, evidence manifest, prompt objects/manifests, scorer opening and
scoring projections, canonical report JSON/Markdown, and
`candidate_verdict_v1`. It may bind only the pre-existing registry
namespace/genesis/prior/reservation proofs. The core evidence manifest binds
run/prompt/scorer/freeze/reservation component hashes; the report binds that
manifest and sanitized run-record-only root; the candidate verdict binds report
JSON/Markdown, renderer, manifest, record, scorer, prompt, freeze, and
reservation hashes. None may contain the retirement transition/post-root,
`core_evidence_root`, final publication envelope/hash, or visibility seal.

After every member is immutable and scanner-approved,
`core_evidence_merkle_v1` computes `core_evidence_root` exactly once. Member
paths must already be canonical, relative POSIX ASCII paths under the closed
TECH grammar; aliases are rejected rather than rewritten. Leaves bind the
length-prefixed path, byte length, and raw SHA-256 member digest; internal
nodes have a separate domain, duplicate the final node at an odd level, and
reject an empty tree. All lengths are unsigned 64-bit big-endian, digest inputs
are raw 32-byte values, and serialized digests are 64-character lowercase hex.
TECH owns the exact framing and cross-implementation fixed vectors. The root is
carried only by the later layer, so no core member hashes itself.

`release_revision_registry_v1` is one public, append-only authoritative
CAS/transparency namespace whose identity, genesis checkpoint/root, log key,
independent witness keys/quorum, gossip rule, and maximum checkpoint age are
fixed by the charter, never supplied by an execution root or bundle. It
combines an append log with an authenticated state map keyed by the
content-derived fingerprint over hidden-input, scoring-IR, oracle-rule, sanitizer,
and deriver digests under [`VECTORS.md`](VECTORS.md); identifiers,
revisions, execution roots, and random values are invalid. The plan binds a fresh, independently obtained,
quorum-witnessed prior checkpoint/root, its consistency proof from genesis,
prior chain hash, authenticated `unused`/non-membership proof, and expected
create-only CAS. Before any live host/provider call, preflight obtains the
current checkpoint from the authority and witness quorum, rejects same-key
equivocation/split view through gossip, and atomically reserves the fingerprint
against that root. A race, stale/forked root, missing quorum, alternate
namespace/key, unavailable authority, or ambiguous receipt forbids the call.
A reservation consumes the fingerprint even when execution later aborts.

Second, the runner calls exactly one logical
`publish_and_retire(fingerprint, core_evidence_root)` CAS. The atomic transition
binds that root while changing `reserved -> retired`; it exposes no evidence
bytes and returns the signed post-root, retirement receipt, append
inclusion/consistency, authenticated-state transition, and non-reuse proofs.
An exact replay returns the same committed transition/proofs and never appends
a second transition; a different root for the fingerprint is rejected.

Third, the runner builds closed `final_publication_envelope_v1` from the exact
candidate, members, and create-only registry result, then commits a candidate-
specific `final_envelope_freeze_v1` binding proof/certificate hashes, envelope
length/digests, prior freeze hash, and authenticated prior-ledger checkpoint.
One `BEGIN IMMEDIATE`/`synchronous=FULL` transaction authenticates selected
prior checkpoint/head, inserts record/index/signed post-checkpoint, then CASes
both prior hashes; `changes()!=1` rolls every write back. Exact replay returns
canonical bytes; byte/hash drift or stale head/checkpoint fails. Every crash
before commit exposes only signed prior state; commit-before-ack exposes all.
Lost-registry-response recovery verifies that transition, rebuilds/scans from
the immutable candidate, and create-or-reads this freeze; no alternate proof,
checkpoint, core, or envelope is selectable. The envelope excludes self-hash,
freeze, and visibility. TECH freezes production-shape, arbitrary-history,
visibility, freeze-ledger, completion, full-tainted, and tamper vectors.

The charter independently pins `publication_visibility_authority_v1`:
namespace/genesis, pure Ed25519 receipt/checkpoint keys, append-log/map
framing, witness history/quorum/gossip, checkpoint freshness, and object
namespace. TECH closes the map key over namespace/fingerprint/core root, the
only serializable `visible` map value, object-set root, and transition log
leaf. Closed `visibility_proof_suite_v1` carries the signed prior absence and
post visible checkpoints, sparse absence/inclusion paths, append-log
inclusion/consistency paths, bound leaf/value, signed receipt, and its own
create-only checkpoint certificate under the same fixed-set rule. Its
transaction verifies private staging, then atomically publishes the map value,
leaf, receipt, exact envelope, and every core object. One authenticated
`read_visible(fingerprint, core_root, object_path)` gate rejects every request
until the committed value, object root, and bytes agree. Each success returns a
signed path/length/SHA-256 receipt; completion binds the ordered verified set
under [`VECTORS.md`](VECTORS.md). No partial or receipt-only public state exists. TECH
fixes non-empty multi-sibling/order and early-read state-machine vectors.
Alternate authority/key, missing proof/quorum, or partial/different objects
cannot upgrade a claim.

The persisted publication sequence is exactly four separately tested steps:
freeze/scan/hash the core; CAS-retire it; create-or-read the candidate-specific
final-envelope freeze; then atomically seal visibility, verify its proof/read
gate, and commit completion. `publication_complete_v1` is a closed,
hash-chained record over the candidate, envelope freeze, authority proofs, and
signed read-receipt root; pre-read metadata cannot substitute. Its record bytes, unique
`completion_id` index, and journal head commit in record -> index -> head order
within one durable `BEGIN IMMEDIATE` transaction; uncommitted changes are externally invisible and committed changes are all
visible. Exact replay returns the original record/hash, while same-ID drift or
a stale previous head is rejected. Recovery may resume only the next step
proven by both authorities.

A public verdict equals the core candidate verdict only when both object layers,
the registry transition, every exposed object, and the independent visibility
seal verify. Otherwise the effective comparative verdict is `INSUFFICIENT`,
except an already verified redacted breach remains safety `FAIL`.

`publication_state_v1` is a closed, evidence-derived enum:

| State | Required evidence |
|---|---|
| `reserved_unpublished` | Authenticated exact registry reservation plus authenticated visibility absence; no retirement proof or valid seal. |
| `retired_unpublished` | Authenticated retirement of this exact fingerprint/core root plus authenticated visibility absence; no valid seal. |
| `authority_unresolved` | Required registry or visibility evidence is unavailable, conflicting, malformed, stale, signed by another authority/key, or does not authenticate this exact candidate tuple. |
| `visible` | Exact retirement proof, valid signed visibility receipt/proofs, and byte verification of the envelope and every referenced core member. |

No timestamp, local file, bundle assertion, or retry count may derive or upgrade
this state.

The publication recovery table is closed:

| Boundary | Recovery and verdict |
|---|---|
| Before core freeze/scan | With exact reservation/absence proofs the state is `reserved_unpublished`; staging is private. Resume deterministic construction from immutable run inputs. Differing bytes, lost input, or missing proof is `authority_unresolved` or `partial_non_security` / `INSUFFICIENT` as applicable. |
| Core frozen, CAS result unknown | Query the registry and visibility authorities. Exact reservation plus absence remains `reserved_unpublished` and permits only the exact fingerprint/core-root CAS; exact retirement plus absence is `retired_unpublished` and recovers its proof. Unavailable, conflicting, stale, malformed, or different-root evidence is `authority_unresolved` / `INSUFFICIENT` and permits no mutation. |
| CAS committed, before final-envelope freeze | State is `retired_unpublished`. Query the exact create-only registry result, deterministically rebuild and rescan from the immutable candidate, then create-or-read only that candidate ID's hash-chained freeze. Existing drift or competing bytes is `authority_unresolved` / `INSUFFICIENT`; never substitute a core, proof, certificate, run, report, verdict, or revision ID. |
| Final-envelope freeze committed, visibility absent | Rehash exact bytes against the freeze, then retry only the exact visibility seal. Drift is `authority_unresolved` / `INSUFFICIENT`; never redo CAS or evaluation. |
| Visibility call ambiguous | Query both authorities. Exact signed receipt/proofs plus every exact object derives `visible` without another seal; authenticated absence preserves `retired_unpublished` and permits only the byte-identical retry. Partial/different/multiple visibility, forged/mismatched proof, or unavailable truth derives `authority_unresolved` / `INSUFFICIENT`. |
| `visible`, completion record absent | Reverify exact retirement, envelope freeze, both fixed certificates, proof suite, read gate, and every byte, then commit the record + unique ID + head in one transaction; never call the seal again. |
| `publication_complete_v1` | Exact replay returns the existing record/hash. Any field drift, previous-head mismatch, replacement, or second seal is rejected; correction needs a new unused fingerprint while the old one stays retired and visible. |

All staging remains invisible before the final seal. Failures and recovery
queries are append-only. A transient failure may retry only the exact operation
authorized by the proven state; it never starts a new evaluation. A
non-retryable scan/privacy/integrity failure preserves the last provable
publication state and yields `INSUFFICIENT` (or safety `FAIL` for a verified
leak). The verifier independently obtains fresh registry and visibility
checkpoints. It validates genesis -> prior -> reservation -> retirement -> post
for the registry and absent -> visible for the publication authority, including
both consistency chains and the exact object set. Bundle-local/forked history,
same-key equivocation, alternate authority/key, or reuse under another
revision/root/clone is invalid.

The release evidence also contains all sanitized primary and diagnostic
records, attempt history, manifests, direction reports, and claim verdict.
Every claim-bearing JSON artifact uses the versioned exact-byte
`canonical_json_rfc8785_v1` contract defined in TECH. Markdown is
deterministically rendered from that JSON; the verdict binds both hashes and
the renderer version, and verification regenerates Markdown byte-for-byte.

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

- it does not accept a reusable workflow or gate approval artifact;
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
  native-neutral ablation, registered-native-canary exclusion, maintained-export
  protocol, causal attribution, remem-preparation failure fan-out, pre-prompt
  null/no-imputation, full-stream reveal commitment, fixed retry selection,
  failure retention, pre-run scorer/oracle commitment, frozen scoring inputs,
  post-run scorer-oracle recomputation, and leak redaction have positive,
  missing-evidence, and tamper tests.
- Negative tests cover oracle/commitment-opening visibility before terminal
  sealing, raw and registered-encoding private-byte leaks, scanner
  crash/unclassifiable output, duplicate release attempts and observed duplicate
  seals, and retired revision reuse, asserting the exact safety `FAIL` versus
  comparative `INSUFFICIENT` mapping above.
- Prompt-publication attacks cover missing/redacted bytes, manifest
  gap/overlap/duplicate/relabeling/closure expansion, slice/root rehash drift,
  out-of-closure cross-condition mutation, rolling-send mismatch, and a byte
  declared too private or result-sensitive to publish; each is
  `partial_non_security` / `INSUFFICIENT` unless a verified leak yields safety
  `FAIL`.
- Registry attacks cover a self-reported empty/local namespace, alternate key,
  missing/wrong genesis, stale/forked or same-key split-view checkpoint,
  missing witness quorum/gossip or bundle-external checkpoint, invalid
  non-membership/inclusion/consistency/state proof, CAS race, missing
  retirement, and fingerprint reuse via a new revision ID/execution root; live
  preflight rejects before calls, while observed candidate evidence is
  `partial_non_security` / `INSUFFICIENT`.
- Publication-DAG attacks cover a core object containing later-layer data;
  noncanonical/unknown/duplicate fields or scalar/order/path framing; empty/odd
  trees; a summary-only or schema-invalid member graph; missing reservation
  history; invalid map/log proof ordering; nonminimal
  checkpoint/witness selection; Merkle/full-envelope/hash vector drift; core
  drift after CAS; different-core replay; envelope self-hash/visibility; and
  missing/drifting candidate-specific freeze. Exact same-core recovery succeeds
  without a second retirement and creates or reads one identical freeze.
- Publication-authority attacks cover map key/value/log-leaf/object-root
  framing; absence-to-visible sparse/log proofs; checkpoint/witness selection;
  alternate namespace/key; forged receipt/signature; reordered/omitted sibling;
  pre-visible reads; changed-envelope retry; collision; partial/multiple
  visibility; and double seal. Positive and tamper vectors, every crash boundary,
  final-completion predicate, transaction visibility before/after commit,
  rollback at every pre-commit crash injection, and legal/illegal retry are
  exercised; invalid or ambiguous evidence is `partial_non_security` /
  `INSUFFICIENT` unless security precedence applies.
- The production user-identity prerequisite is either implemented and tested,
  or every public comparative verdict is deterministically `INSUFFICIENT`.
- A separately authorized smoke covers both directions and all six required
  primary and native-import surfaces before any full-matrix authorization.
- Complete and both partial manifest forms regenerate byte-identical RFC 8785
  JSON, Markdown, verdict, core Merkle, and final-envelope artifacts; all
  canonicalization and hash protocols pass fixed cross-implementation vectors.
- The four registered direction/comparator intervals deterministically produce
  `PASS`, statistical-regression `FAIL`, or `INSUFFICIENT` under the exact
  bootstrap predicate.
- Current documentation is updated for every final verdict.
- No public positive claim appears before a complete verified `PASS`.
