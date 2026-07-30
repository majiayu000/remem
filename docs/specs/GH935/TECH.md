# Cross-Host Continuity Benchmark — Technical Contract

Status: Current contract; v1 infrastructure shipped, completion unimplemented; Issue: #935

## Existing Implementation Facts

- `eval/cross-host/benchmark-charter.json` is `cross-host-v1` / `infrastructure_only_no_runs`.
- `eval/cross-host/tasks/` contains 24 `skeleton_todo` JSON tasks.
- `cross-host-task.schema.json`, `cross-host-run.schema.json`, `schema_validate.py`,
  `scan_artifacts.py`, and `run_dry.py` are offline; no Rust runner/report exists.
- Production project identity is the canonical Git root from `src/project_id.rs`.
- `ContextRequest` has no user field; startup ownership resolves through `user:default`.
- Host-native import produces untrusted candidates, never auto-promoted memories.
- Claude hooks can call `sync_native_memory` before diagnostics, so a source clone is not a neutral control.

## Versioning

Implementation bumps charter/schemas from `cross-host-v1` to executable `cross-host-v2`; old artifacts need a field-validating v2 converter.

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

## Contract Objects

All claim-bearing JSON uses snake_case, closed enums, and `canonical_json_rfc8785_v1`: parsed I-JSON is serialized exactly by RFC 8785 JCS.
JCS fixes recursive raw-property sorting by unsigned UTF-16 code units, ECMAScript primitive serialization, preserved array order, no Unicode normalization, and zero token whitespace.
Output is UTF-8 with no BOM or trailing newline. Schemas reject unknown/duplicate keys, lone surrogates, NaN/Infinity, and numbers outside finite IEEE-754 binary64.
Precision-sensitive integers outside `[-(2^53-1), 2^53-1]` use schema-typed strings matching `0|[1-9][0-9]*|-[1-9][0-9]*`. The plan binds the algorithm/version.

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
- `pre_run_scorer_commitment_v1`, which commits all result-affecting scorer/oracle semantics and bytes or their sole deterministic derivation before any live call;
- source-native snapshot policy for the diagnostic arm;
- per-host executable/profile requirements;
- `status: "ready"` and `todo: []`.

The task validator rejects empty values, aliases, direction/host mismatch,
target-visible hidden paths, and any canary copied into prompts or gold facts.

### Source Seal

One source seal exists per `(direction, task_id, run_index, source_attempt_id)` and contains:

- task/fixture, executable, profile, model, schema, and migration hashes;
- `condition_neutral_episode_evidence_v1`, binding source host/session/event IDs,
  transcript/tool-event/Git/workspace hashes, and the source-native manifest or typed native absence;
- canonical project path and the exact `project_from_cwd` value;
- terminal extraction/review state and queue counts;
- a sorted data-only manifest for the quiesced full source
  `REMEM_DATA_DIR`, with typed exclusions for `.key`, environment-provided
  keys, WAL/SHM, sockets, and other runtime-only secret/process files;
- a deterministic data-only full-store archive hash, length, and creation policy, or
  `remem_preparation_failed(stage, reason)` typed absence; exclusions record type, never value;
- target-transfer projection logical/ciphertext hashes, sorted manifest, non-secret
  `projection_key_id`, policy versions, forbidden-surface proof, or matching typed absence;
- `native_neutral_base_v1` hash, policy, zero native candidate/active/provenance
  counts, referential-integrity proof, or typed diagnostic-preparation absence;
- source-host native file manifest/hash, or typed absence;
- `maintained_export_v1` hash plus every cycle input/state/usage/budget; a
  commit carries prior/output/freeze hashes, while failure carries a typed
  reason and no output/freeze hash;
- creation/cleanup timestamps and scanner status.

The full store archive and target-transfer projection are private execution material, not committed public artifacts. In v2, a source and all dependent targets run in one single-operator execution root. The data-only full archive keeps its source SQLCipher key in a runner-private secret root only until attribution completes. Logical projection and sealed ciphertext have separate hashes; the ciphertext is read-only and only fresh byte-identical clones back `remem_shared`. Clones diverge only through normal target-runtime writes after verification. The full archive stays outside target-visible roots and in the host-read deny policy; target access invalidates the run even without transcript output.

After fixed primary review, a trusted runner opens the source with its private key, mechanically sanitizes the logical projection, and SQLCipher exports/rekeys it under a distinct fresh projection-scoped 256-bit key. The same ciphertext/key back every initial clone. It preserves current schema/migrations/tables/triggers/indexes for normal `open_db()` drift and MCP checks, plus curated rows, relations, current state, and minimal opaque provenance for production filtering. Registered stale/superseded challenges keep their state flags without becoming retrieval-eligible or target-exposed.

The sanitizer removes raw messages, captured events/blobs, observations, extraction payloads, other target-ineligible session surfaces, their FTS/auxiliary entries, and resolvable private paths; checkpoints a fresh database; and ships no transcript, host-native file, credential, `.key`, WAL, or SHM. It never deletes a registered challenge merely for being stale/superseded.

Plan-hashed `projection_secret_channel_v1` fixes IPC type/path, authentication, lifecycle, and failure. A trusted runner outside the host sandbox starts the sidecar and provisions one key over a private inherited pipe; host hook/MCP config has only a keyless authenticated endpoint. The host cannot read sidecar environment/argv/process metadata/pipe/secret root, and no key enters HOME, config, projection, manifest, log, or `.key`.

The runner hashes sealed ciphertext/clones before SQLite opens. A disposable clone handles schema/open/MCP preflight; only an actual target clone records its pre-open hash then becomes mutable. Validation proves schema/migrations, referential integrity, `PRAGMA foreign_key_check`, encrypted header, and production open; source/wrong/missing keys fail closed and raw/sparse searches find no source hits.

A private registry tracks each `projection_key_id`, reserved tuple IDs, active attempts, and destruction proof. Attempt exit releases only its active ref; reservation survives an eligible no-write retry. First committed/conservative reveal, exhausted budget, or non-retryable terminal removes the tuple once; zero destroys the key. Live scans exclude only expected trusted holders. After attribution, the runner destroys the source key and scans trusted holders, environment, argv, logs, process metadata, and artifacts for raw/registered encodings before manifest/report finalization. Exposure is a breach; cleanup failure is partial/insufficient.

`native_neutral_base_v1` never replaces/mutates the primary store/projection. It removes `claude_native`/`codex_native` candidates and the transitive candidate-derived memory, operation, graph, and current-state provenance closure; proves zero native candidate/active/provenance counts, valid foreign keys/schema, and production open; and supplies byte-identical clones to both diagnostic arms.

Cross-clone continuation is unsupported. A source attempt owns immutable evidence/preparation; every target has its own attempt, and sealed objects are never regenerated. Before dependent reveal and common-evidence seal, only the five `pre_prompt_transient_v1` reasons may start a new source attempt within the three-attempt limit. After that seal, remem-only archive/extraction/projection/rekey/clone/verification failure is a non-retryable condition failure, not source retry. Verified breach is `partial_security`/`FAIL`; unsafe cleanup is `partial_non_security`/`INSUFFICIENT`. Cross-clone support needs a separate encrypted retention/fetch contract.

### Maintained Export Protocol

`maintained_export_v1` is closed and plan-hashed before live execution. It contains:

- exact system, generation, and update prompt bytes and hashes;
- exporter executable, model, profile, sampling, and protocol versions;
- `evidence_projection_v1`: canonically ordered visible source refs/files and hashes, foreign-project-canary exclusion test, and rejection of target prompt, gold, hidden scorer, or prior-condition output;
- episode-one generation input; each update gets the previous frozen envelope plus only its registered new episode delta;
- envelope schema/version, stable-key rules, and conflict handling that replaces state only with explicit chronology/supersession evidence, otherwise retaining both with abstention;
- per-cycle/cumulative host/LLM calls, input/output tokens, turns, wall time, bytes, and estimated-cost caps.

Each boundary creates one immutable `cycle_id`: `committed` (one atomic schema-valid commit), `failed` (zero commits), or `not_started_after_prior_failure`. Regeneration, output choice, and prior-envelope fallback are forbidden. First non-retryable failure disables later cycles. A whitelisted pre-prompt transient aborts the source attempt; a fresh attempt may restart under `pre_prompt_transient_v1` while retaining records/cost. Other non-security/non-integrity failures make `exported_file` an `ordinary_failure` while other conditions continue from the unmodified seal. Security/integrity uses global breach rules. Retry never changes prompts, evidence, rules, schema, or caps.

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

The closed record binds method/intervention hashes, resolved tuple, use and first-wrong-action event IDs/orders, replay hashes, matcher/scorer results, reason, and status. A Stop-created `memory_usage_events` row and its `memory_citation_events` parent are post-action corroboration only and cannot satisfy either proof method. Missing, ambiguous, competing, or post-hoc evidence becomes `missing_evidence` and, outside the narrow lower-bound rule, comparative `INSUFFICIENT`; a no-memory surface is `not_proven/no_memory_surface`. Sanitized inputs support recomputation.

### Prompt Stream and Reveal

`host_channel_prompt_stream_v1` is one immutable object containing the complete initial ordered bytes written to prompt-bearing channels: benchmark-controlled system/developer content, condition context/memory, task content, tool prelude, separators, and adapter framing. `prompt_surface_manifest_v1` binds each segment's role/channel, offset, length, SHA-256, and condition owner; absent surfaces have a zero-length hash. The task segment is identical across conditions. The sole differing `condition_surface` is a framing closure containing content plus every derived prefix/checksum/content-length/separator/wrapper byte. Claim-bearing v2 forbids later condition-specific MCP/context delivery; an opaque/nondeterministic prelude is unsupported.

Before spawning a process that can emit prompt bytes or writing any byte, the runner appends, fsyncs, and seals `prompt_reveal_committed(target_attempt_id, immutable_object_id, prompt_stream_hash, prompt_stream_length, prompt_surface_manifest_hash, task_segment_hash, condition_surface_hash, condition_surface_length, adapter_version, planned_sequence, realized_sequence, prior_journal_hash)`. It then sends from that object without rerendering and records rolling sent hash/length. No stream byte may precede the seal. A no-write proof binds the planned hashes and launcher/channel state and proves no prompt-bearing process spawned and zero bytes written.

After finalization or crash recovery, `R`/`N` count valid reveal/no-write artifacts, `T` counts terminal seals whose ID/prior hash/class agree, and `M` counts every malformed, unhashed, out-of-order, conflicting, or unexpected artifact excluded from those counts. The verifier applies this closed table:

| Evidence / terminal class | `reveal_state`; `resolved` | `selected_claim_attempt`; manifest/verdict; retry |
|---|---|---|
| `M=0,R=1,N=0,T=1` | `committed`; boolean only after frozen-input score/oracle verification, else null | current attempt; null score forces `partial_non_security`/`INSUFFICIENT`; never retry |
| `M=0,R=0,N=1,T=1`, exact retryable reason, budget remains | `proven_no_write`; null | null in checkpoint `partial_non_security`/`INSUFFICIENT`; exactly next attempt allowed |
| Same, budget exhausted | `proven_no_write`; null; tuple `missing_pre_prompt_exhausted` | null; final `partial_non_security`/`INSUFFICIENT`; no scorer/retry |
| `M=0,R=0,N=1,T=1`, registered post-common-source non-retryable preparation failure | `proven_no_write`; false `ordinary_failure` | current attempt; `complete` remains possible; never retry |
| Every other combination (`M>0`, `T!=1`, duplicates, both, or neither included) | verifier-derived `conservatively_revealed_invalid`; null `reveal_evidence_invalid` | current attempt ID; final `partial_non_security`/`INSUFFICIENT` unless breach override; no scorer/retry |

Each terminal seal binds the ordered journal root, every reveal/no-write artifact including duplicates, counts, derived state, selection, `resolved`, and scorer refs. The last row preserves conflicts and fabricates no reveal event. Selection never uses timestamps/status inference or a better later score. Precommit spawn, prefix write, rerender drift, send reordering/hash/length mismatch, or undeclared bytes sets `M>0`.

### Scorer Commitment and Frozen Inputs

Before the first live host/provider call, the canonical plan seals `pre_run_scorer_commitment_v1`: release fingerprint/revision; exact hidden scorer engine/wrapper bytes/hashes; runtime/container, argv, environment allowlist, and output framing; result-affecting `scoring_semantics_ir_v1`; hidden-input root; assertion/oracle-rule revisions; and exact already-built oracle bytes/hash or one deterministic derivation. A derivation commits sanitizer/deriver bytes, runtime/toolchain, static inputs, typed future-freeze slots, framing, and equivalence vectors; concrete outputs fill slots only after freeze. `scorer_read_contract_v1` deny-by-default commits permitted paths/streams/metadata/environment/ambient state. Hidden scorer/oracle call the same IR. Post-run authoring, semantic redaction, uncommitted rebuild, regeneration, branching, or output choice is invalid.

After target termination and before the scorer receives hidden bytes, `scoring_input_freeze_v1` closes writers and content-addresses final workspace plus complete host output with exact lengths, sorted manifest, exclusions, archive hashes, and scanner policy. The committed projector derives `public_scoring_projection_v1`; its manifest/root, actual read-set hash, and inclusion proofs bind every scorer-readable byte to full-freeze roots. Hidden scorer/oracle are deny-by-default sandboxed to byte-identical read-only projection clones and cannot read the larger private root. The exact projection publishes after terminal sealing. Out-of-set read, undisclosable result input, missing proof/object, root mismatch, or scorer/oracle disagreement sets `resolved=null` and `partial_non_security`/`INSUFFICIENT`.

### Source Attempt Record

Before auth or preparation, the outer runner creates an append-only
`cross-host-source-attempt-v2` journal. Finalization seals IDs/input hashes,
lifecycle timestamps, nullable condition-neutral evidence hash/sealed-at boundary,
terminal reason, typed preparation absences, cost, cleanup, and leak scans.
This distinguishes failure before common evidence from later condition failure.
Recovery appends and seals a new record; it never rewrites history.

Every manifest lists all source-attempt record hashes, even without a target attempt;
target runs bind the matching ID and hash. Missing/non-terminal journals, hash drift,
or unrecorded cost make a non-security manifest partial and `INSUFFICIENT`; a verified
breach still takes precedence.

### Run Record

Each target attempt records:

- tuple key, `source_attempt_id`, source-attempt record hash,
  `target_attempt_id`, condition, direction, task, and run index;
- planned/realized condition position, start/end timestamps, and one sealed terminal reason;
- source seal hash, even when the condition cannot read its contents;
- full prompt-stream/surface-manifest/task-segment/condition-surface hashes and
  lengths, rolling sent hash/length, executable/profile hashes, all
  reveal/no-write artifacts, and the derived reveal state;
- pre-run scorer-commitment/release-fingerprint/revision hashes, frozen final
  workspace/output hashes, scorer invocation/result hash, and oracle
  recomputation hash;
- projection logical/ciphertext hashes and `projection_key_id`, where applicable;
- target HOME/config/session/workspace identities;
- status: `success`, `ordinary_failure`, or `security_breach`;
- resolved and secondary metrics with nullable values and reasons;
- host calls, LLM calls, tokens, wall time, turns, and estimated cost;
- cleanup and leak-scan results;
- attribution refs or typed `absent_due_to` for every production stage;
- maintained-export protocol/boundary/budget or applicable native-neutral/import records;
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

Every kind contains the planned tuple set, source-attempt record hashes, all
target attempts, fixed attempt/reveal policy, per-attempt `R`/`N` counts and
`M`/`T` counts, ordered journal/terminal-seal roots, derived reveal states,
terminal `selected_claim_attempt`/`resolved`, missing/
not-started tuples, scorer commitment/release fingerprint/revision, frozen scoring-input
hashes, prompt-stream/manifest root, authoritative registry
namespace/genesis/prior/reservation roots and receipt, lower-level
run/prompt/scorer/freeze/reservation hashes, and reason codes. As a core member it excludes `core_evidence_root`,
retirement/post proofs, final-envelope/hash, and visibility receipt. A manifest cannot be `complete` when any selected
`committed` boolean lacks matching oracle/publishable stream, when the
registered no-reveal `ordinary_failure` lacks its typed stream/scorer absence,
or when the core registry reservation proof is invalid.

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

The `canonical_json_rfc8785_v1` report includes:

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
- pre-run scorer commitment/release fingerprint/revision, frozen workspace/output and
  scorer/oracle result hashes for every selection;
- prompt-stream/manifest root and registry genesis/prior/reservation roots;
- core evidence-manifest hash and sanitized run-record-only root.

Markdown is rendered deterministically from canonical JSON. The verdict binds:

- SHA-256 of the exact canonical JSON bytes;
- Markdown report hash;
- renderer version/hash;
- core evidence-manifest and sanitized run-record-only roots;
- scorer commitment/release fingerprint/revision, prompt-stream root,
  registry reservation root, and the ordered frozen-input/result root.

Manifest, report, Markdown, and `candidate_verdict_v1` are core members: report binds manifest/record roots; candidate verdict binds report JSON/Markdown, renderer, manifest/record/scorer/prompt/freeze/reservation hashes. None contains `core_evidence_root` or later publication fields. After scanning, the domain-separated Merkle root over canonical ordered `(path, length, sha256(bytes))` core members is computed once and carried only by `final_publication_envelope_v1`.

Verification regenerates Markdown byte-for-byte before publication. Plan-hashed
vectors must match an independent RFC 8785 implementation for nested key order,
arrays, escapes/non-ASCII, numeric edges, invalid I-JSON, BOM, and trailing LF.

## Execution State Machine

### 1. Offline Validate and Plan

The planner:

1. validates the charter, schemas, all 24 tasks, fixtures, maintained-export
   protocol, causal-oracle rules, exact prompt-stream adapters, and unretired
   release fingerprint/revision;
2. proves 288 primary and 144 required diagnostic tuple keys with no
   duplicates;
3. resolves exact host/model/profile/exporter binaries, projection/rekey/
   provisioning/bootstrap versions, then builds and seals
   `pre_run_scorer_commitment_v1` without exposing it to a run-visible root;
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
6. writes the schedule, prompt/scorer/release and all other
   protocol/seed/algorithm hashes, and canonical plan hash without starting a
   host or network call.

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

The source host executes all episodes before target reveal. At each boundary,
the outer runner records transcript/tool-event/Git/workspace evidence independently,
drains remem capture as separate preparation, and runs the target-blind exporter
unless it already failed. Exporter hooks/capture/native persistence are disabled;
read-only `maintained_export_v1` evidence is its only input and envelope/usage its
only output. Terminal export failure marks later cycles
`not_started_after_prior_failure`; protected-manifest mutation or protocol drift
invalidates all dependents. After the final episode, the runner stable-reads native
inputs or typed absence, seals `condition_neutral_episode_evidence_v1`, then:

1. records the final committed envelope/freeze hash, or typed terminal failure
   with no output/freeze hash;
2. drains extraction and applies primary review policy `automatic_only_v1`:
   only the shipped automatic promotion path may activate candidates;
   pending/quarantined rows stay inactive, manual review mutations are rejected,
   and `manual_review_time`, `manual_review_turns`, and `manual_review_cost` are
   recorded as zero while automatic pipeline cost remains attributed to remem;
3. stops the worker, checkpoints the database, closes writers, and fsyncs the
   run root;
4. snapshots the exact data-only remem store while retaining the source key
   only in the runner-private secret root;
5. derives and verifies `native_neutral_base_v1` without altering the primary
   full store;
6. derives the curated logical projection, SQLCipher exports/rekeys it under a
   fresh projection key, and proves forbidden surfaces absent while schema and
   referential integrity remain valid;
7. seals all preparation and committed-export hashes or typed failures;
8. verifies sealed/clone ciphertext hashes before any open, then performs
   production-open/MCP checks on a disposable clone.

Failure when condition-neutral evidence cannot seal is a source-attempt failure.
If it seals, any remem-only capture/extraction/quiesce/store/projection/rekey/clone/open
failure is `remem_preparation_failed`; absent a verified breach, `no_memory`,
`target_host_native`, and `exported_file` run, while the affected `remem_shared`
tuple gets one terminal `target_attempt_id`, `ordinary_failure`,
`resolved = false`, typed absences, and no retry.
Diagnostic-only preparation failure affects only its pair. Artifacts are never
regenerated/partially copied; verified security breach keeps global precedence.

### 4. Fan Out Conditions

Targets run serially in fresh HOME/config/session roots and in the exact
plan-hashed counterbalanced order. Each condition gets a fresh fixture reset
and only its declared memory surface. The runner records planned/realized
position and timestamps; skipped, reordered, or duplicated positions make the
comparison insufficient.
Before a prompt-bearing process can start, the adapter renders the complete
`host_channel_prompt_stream_v1` and surface manifest. The runner then seals the
matching no-write proof or commits `prompt_reveal_committed` before any stream
byte, sends exactly those bytes, and seals one terminal reason/reveal state.

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
adapter for Claude Code and Codex. The common task segment remains identical;
the separately hashed envelope is the sole differing stream segment. A
condition-only task note or any unmanifested byte is forbidden.

The envelope contains only allowed handoff facts and provenance, never raw
transcripts, tool logs, hidden paths, or the foreign-project canary.

#### `remem_shared`

The target receives a fresh verified clone of the sealed curated transfer
projection, never the full automatic-capture store. Retrieval runs through the
plan-selected production SessionStart/Context Bundle path before the immutable
initial stream is sealed. Interactive MCP memory retrieval is disabled for
claim-bearing v2. Raw MCP/CLI surfaces may exist in the binary, but the
projection has no source raw rows/transcripts. Direct inserts, gold seeding,
manual saves/review, and special eval-only retrieval are rejected.

#### Source-Native Import Diagnostic

The runner captures actual native-memory files produced by the **source host**
before cleanup:

- Codex source: supported Codex rollout-summary input;
- Claude Code source: supported Claude topic-file input.

Implementation adds `remem import claude-memories --source <sealed-root> --authorized-project <project-id>` with dry-run/`--expect-plan-digest`; Codex diagnostic import gets the same filter.
`claude-native-snapshot-v1` binds each stable-read file's record ID, path,
source-project identity, byte/snapshot hash, and `project_relation` of
`authorized_project` or `foreign_project_canary`. The plan derives relation via
production identity, requires at least one of each, and batch-rejects any unknown
relation/entry, symlink, drift, unsafe/malformed content, or hash mismatch.
JSON apply returns `import_attempt_id` and each record's closed `status`
(`inserted`, `excluded_wrong_project`, `duplicate`, `quarantined`) plus nullable
`candidate_id`; persistence reuses the external-candidate path, never hook-only
`sync_native_memory`.

Both diagnostic arms start in separate preparation roots from byte-identical
`native_neutral_base_v1` clones and the same sealed source-native snapshot.
Before toggling, each clone must have zero native candidate, active-memory, and
transitive-provenance rows; neither base is target-visible. The without arm
does not import or review, then derives/rekeys its projection. The with arm
invokes the shipped importer: Codex uses its dry-run digest; Claude binds the
sealed snapshot and plan. Each `authorized_project` record returns `inserted`
with `candidate_id`, complete import/candidate/promoted-review/memory lineage,
cost, origin, and trust. Each registered canary returns
`excluded_wrong_project`, null `candidate_id`, production-filter proof, and zero
candidate/review/memory rows. Only registered canaries may be excluded; any
relation/outcome mismatch, duplicate, quarantine, noop, reused memory, or broken
lineage makes the pair `INSUFFICIENT`.
The import/review mutation is the only allowed difference before projection;
both targets receive fresh projection clones and never a full source-store or
neutral-base clone.

Empty, unsupported, unsealed, or different native inputs make the pair
insufficient. No target-host native preparation and no raw source transcript
is used for this diagnostic.

### 5. Score, Scan, and Record

After the target exits, the runner:

1. closes writers and seals `scoring_input_freeze_v1` over the final workspace
   and complete host output before revealing hidden tests;
2. verifies the frozen objects and pre-run commitment, then runs the committed
   hidden scorer on a read-only clone in a separate restricted process;
3. runs the precommitted oracle on the same frozen hashes, scans target-visible
   and candidate artifact roots, and rejects scorer/oracle disagreement;
4. records outcome, attribution, cost, cleanup, commitment, frozen-input,
   invocation/result, and recomputation hashes;
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

`pre_prompt_transient_v1` applies independently to every source unit and tuple:

- maximum three immutable source attempts before any dependent reveal and three target attempts per tuple;
- retry before reveal only for `transient_auth_unavailable`, `provider_unavailable`,
  `host_bootstrap_failed`, `runner_io_before_prompt`, or `pre_prompt_timeout`;
- apply the Prompt Stream and Reveal table: the first `committed` or
  `conservatively_revealed_invalid` attempt is selected, while the only
  selected `proven_no_write` attempt is the registered non-retryable
  post-common-source preparation failure; none permits a later attempt;
- semantic failure, unresolved result, post-reveal timeout, scorer result,
  security/scope event, and cleanup failure are never retry-eligible;
- after three eligible pre-reveal failures, `selected_claim_attempt = null`, the
  tuple is `missing_pre_prompt_exhausted` with `resolved = null`, no scorer runs,
  and the report is `partial_non_security` / `INSUFFICIENT`; no false/zero is
  imputed and the registered denominator does not shrink;
- when all three source attempts fail before condition-neutral evidence seals,
  dependent tuples are not started and the candidate report is
  `partial_non_security` / `INSUFFICIENT`.

Selection uses only the sealed artifacts, closed table, and terminal reasons,
never timestamps. Missing/duplicate/conflicting evidence cannot become a retry.
All attempts remain in reliability/cost tables; preparation failure differs
from infrastructure exhaustion.

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

Its registered directional denominator is all 36 `no_memory`/`remem_shared`
pairs; missing/invalid pairs never shrink it. The causal record binds the same
memory across exposure, pre-action use or counterfactual, wrong-action matcher,
and scorer failure. Missing/ambiguous attribution leaves the rate blank and is
insufficient except under verdict step 2.

The `stale_memory_followed` denominator is every complete valid
`remem_shared` tuple in the direction whose hashed candidate inventory,
captured from its sealed target-transfer projection immediately before the
production freshness/validity filter, has at least one query-relevant
stale/superseded challenge item that would otherwise match. Projection
sanitization cannot remove a registered challenge based on freshness or
validity. The inventory is evaluator-only evidence and is never a target memory
surface. The numerator additionally requires that item to be cited/used and
have a `causal_oracle_v1 = proven` record for the wrong action. Missing
inventory-stage proof, challenge inventory, or causal evidence leaves the rate
blank and is insufficient except under verdict step 2. The report shows all 36 `remem_shared`
tuples per direction so the applicable set cannot be hidden.
Each `stale_superseded_decision` task registers a challenge for all three runs,
so a claim-bearing report requires at least three applicable tuples per
direction. A missing expected match or a smaller/empty applicable set forces
`INSUFFICIENT`; it is not a passing zero.
Wrong-project injection, source-private-session leak, a registered private
benchmark byte found raw or under `private_byte_encoding_registry_v1`, key
exposure, and, once numeric, wrong-user injection each map to
`security_breach`/`partial_security`/safety `FAIL` regardless of completeness.
The normative registry fixes exact/hex, standard and URL-safe base64 padded or
unpadded, percent/JSON escapes, archive/compression formats, normalization, and
recursion limits; plans may add but never remove coverage. Scanner crash,
unsupported encoding, or unclassifiable candidate proves neither breach nor
absence: `partial_non_security` / `INSUFFICIENT`. Only stale-memory-followed
and memory-hurt are non-security stop-losses.

Verdict precedence is unique:

1. verified redacted security breach -> safety `FAIL`;
2. identity-valid, otherwise complete direction with only causal/applicability
   gaps and proven `memory_hurt / 36 > 2%` or `stale_memory_followed / 36 > 1%` -> `FAIL`;
3. missing identity, incomplete/invalid evidence or pairs, or a non-security partial manifest -> `INSUFFICIENT`;
4. remaining missing causal/applicable data -> `INSUFFICIENT`;
5. any rank-98,750 adjusted regression upper bound `< 0` -> `FAIL`;
6. all four rank-2,500 descriptive lower bounds `> 0` -> `PASS`;
7. every other complete result -> `INSUFFICIENT`.

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

Raw stores, sessions, credentials, private roots, and unsanitized tests remain invisible. After all attempts seal, scanner-approved `core_evidence_v1` contains selected/prior attempts, manifest, run records, task/schema/export/native/causal/bootstrap material, scorer opening and public scoring projections, complete prompt objects/manifests (or valid no-reveal absence), report JSON/Markdown, and `candidate_verdict_v1`. It contains registry namespace/genesis/prior/reservation evidence but no retirement/post proof, `core_evidence_root`, final envelope/hash, or visibility receipt.

`core_evidence_merkle_v1` accepts a non-empty unique member set. A path must already match `segment("/" segment)*`, where `segment = [A-Za-z0-9_-](?:[A-Za-z0-9._-]*[A-Za-z0-9_-])?`; it rejects absolute paths, `.`, `..`, empty segments, backslash, NUL, non-ASCII, aliases, and duplicates, then sorts by unsigned ASCII bytes. Let `u64be`, `raw32`, and `hex64` mean unsigned 64-bit big-endian, raw 32-byte digest, and 64 lowercase hex characters. For member bytes `b` at path bytes `p`, `leaf = SHA256(ASCII("cross-host-v2/core-evidence-v1/leaf") || 0x00 || u64be(len(p)) || p || u64be(len(b)) || raw32(SHA256(b)))`; each next-level `node = SHA256(ASCII("cross-host-v2/core-evidence-v1/node") || 0x00 || raw32(left) || raw32(right))`. At every odd level `right = left` for the last node without adding a member; one leaf is its root. `core_evidence_root = hex64(root)`. Members never embed the root.

The retained three-member Merkle vector has leaves `71caa83e102f0a84aeeb9fb2c28aee41bee3645d1ce80549f1ebcb63f02171af`, `4d025417f7adf5ebc354f2437803ce0fba6fbc772aa0e82198a215ae28a13f58`, and `b1bde8b9a236ca646e52fbcfed709026e3b52378299726a77a535dccfd9d652d`, with odd-leaf root `3aae31029e37d4de1553dd856615389208808c68e6bc9a5f82a92bcc2e2f527c`.

### Canonical Publication Encoding

All objects below are closed RFC-8785 JCS objects: duplicate/unknown fields, non-I-JSON, noncanonical decimal strings, wrong array order, or wrong literal versions are invalid. `dec = 0|[1-9][0-9]*`, `hex64 = [0-9a-f]{64}`, `hex128 = [0-9a-f]{128}`, and `id = [a-z][a-z0-9._-]{0,63}`. Object keys serialize in JCS order; core members sort by canonical path, witnesses by `witness_id`, sparse siblings by increasing root depth, and append-log proofs in RFC-6962 bottom-up order. `H(tag, pieces...) = SHA256(ASCII(tag) || 0x00 || each(u64be(len(piece)) || piece))`, with raw digests as pieces and lowercase hex only at JSON boundaries.

`transparency_path_v1 = {"bitmap":hex64,"siblings":[hex64...]}` encodes a 256-level sparse Merkle path: bitmap bit `d` (MSB-first, root depth `0..255`) consumes one sibling for depth `d`; zero uses the deterministic empty hash for that depth; adjacent/default siblings may not be redundantly serialized. For prefix `P`, empty leaf is `H(P+"/smt-empty-leaf-v1")`, present leaf is `H(P+"/smt-leaf-v1", raw32(key), raw32(value_hash))`, and parent is `H(P+"/smt-node-v1", raw32(left), raw32(right))`; key bits are MSB-first. Append-log empty/leaf/node are `H(P+"/log-empty-v1")`, `H(P+"/log-leaf-v1", JCS(leaf))`, and `H(P+"/log-node-v1", left, right)` with RFC-6962 topology; proof arrays must consume exactly the declared old/new sizes.

`checkpoint_v1` is exactly `{"authority_signature":hex128,"body":{"log_root":hex64,"log_size":dec,"map_root":hex64,"namespace":id,"previous_checkpoint_hash":hex64,"version":checkpoint_literal},"witness_signatures":[{"signature":hex128,"witness_id":id}...]}`, where the literal is `registry_checkpoint_v1` or `visibility_checkpoint_v1` for its pinned `P`. Its hash is `H(P+"/checkpoint-v1", JCS(body))`; pure Ed25519 authority/witness messages are respectively `ASCII(P+"/checkpoint-authority-v1") || 0x00 || raw32(hash)` and `ASCII(P+"/checkpoint-witness-v1") || 0x00 || raw32(hash)`. From the independently fetched canonical history, choose the lowest `log_size` checkpoint containing the transition with a valid authority signature and at least charter quorum `q`; include exactly the lexicographically smallest `q` valid pinned witness IDs. Any higher checkpoint, nonminimal valid subset, extra signature, missing intervening consistency proof, or unverifiable claim that no lower quorum checkpoint exists is invalid.

`registry_publication_proof_v1` is exactly the vector-shaped object with closed request/receipt/state/checkpoint objects, `reservation_log_leaf`, `reservation_log_inclusion_path`, `reserved_path`, `retirement_log_leaf`, `log_inclusion_path`, `log_consistency_path`, `retired_path`, their declared hashes, and literal version; the positive vector is the normative nested field/type template. For `P = "cross-host-v2/registry"`, map key, state, request, and receipt hashes are respectively `H(P+"/map-key-v1", UTF8(namespace), raw32(fingerprint))`, `H(P+"/state-v1", JCS(state))`, `H(P+"/cas-request-v1", JCS(request))`, and `H(P+"/cas-receipt-v1", JCS(receipt))`; both closed log leaves hash through `H(P+"/log-leaf-v1", JCS(leaf))`. The reservation leaf binds index/key/fingerprint, empty prior root/checkpoint, and reserved post root/state; the retirement leaf binds index/key/fingerprint/core/request, exact prior checkpoint/root/state, and retired post root/state. Exact proof consumption must establish empty -> reserved -> retired and append indices `0 -> 1`; replay returns this proof and a different core is rejected.

`final_publication_envelope_v1` is exactly `{"candidate_verdict":PASS|FAIL|INSUFFICIENT,"candidate_verdict_hash":hex64,"core_evidence_root":hex64,"core_members":[{"byte_length":dec,"path":canonical_path,"sha256":hex64}...],"registry_checkpoint":checkpoint_v1,"registry_checkpoint_hash":hex64,"registry_post_root":hex64,"registry_proof":registry_publication_proof_v1,"release_fingerprint":hex64,"version":"final_publication_envelope_v1"}`. It excludes its own hash/visibility data; all members rehash to the core root, the candidate hash names the candidate-verdict member, and every registry field/proof/checkpoint agrees. After scan/freeze, its exact bytes `e` have `final_publication_envelope_hash_v1 = hex64(SHA256(ASCII("cross-host-v2/final-publication-envelope-v1") || 0x00 || u64be(len(e)) || e))`.

The vector fixture uses non-secret test-only Ed25519 seeds registry authority/w1/w2/w3 = bytes `01/02/03/04` repeated 32, quorum 2; fingerprint bytes `11` repeated 32; namespace `registry.test`; and core member `candidate_verdict_v1.json` with JCS `{"verdict":"INSUFFICIENT","version":"candidate_verdict_v1"}`. This first tamper vector is invalid because it omits the mandatory reservation leaf/inclusion proof and claims a signed `reserved` map at `log_size=0`:

```json
{"candidate_verdict":"INSUFFICIENT","candidate_verdict_hash":"ed925f1828c262c53507c2cff3b0223267fa3bb9e010f0d54b290788bdf9e5fe","core_evidence_root":"dc237632845c2fa9ab9892fe459df78f81cf683f86948d73409b3721e3bcd954","core_members":[{"byte_length":"59","path":"candidate_verdict_v1.json","sha256":"ed925f1828c262c53507c2cff3b0223267fa3bb9e010f0d54b290788bdf9e5fe"}],"registry_checkpoint":{"authority_signature":"9374a2bcd6706267e0cf82b8f42ea56c88d907a02465f173b53358f3798965de028df75aaa85930d02979f8747f759775ba12577554fa79cd551840f1fc21c0e","body":{"log_root":"1594c2a3fb8594e812e153d4552a0064b6af01e361eb8cc81fa0690be6983ef9","log_size":"1","map_root":"ee53bde70bab3b327c5059997367c57107fd86da9c8d7e3a13fd4f35e788e8bc","namespace":"registry.test","previous_checkpoint_hash":"b255d46757cd1e051b036e0d62f9104c8dd27f3dd637b1c698be74fd3f722689","version":"registry_checkpoint_v1"},"witness_signatures":[{"signature":"938b8943de7b4c9b37e6a685fb587b743cccb46ac842b81b90f603cdf5f0dba18b1e8f34dacdda4e7e76047f251a9aa003c5e3ef37eb303239a6f47fb2c28109","witness_id":"w1"},{"signature":"ea065a6983963ad00995b0dae01b2187f0b9e30cff09d8797fc30d9e324913e903098b1e1bcc5df31dd0e0e029e457626c3c4f6a630c42f1355205823543a80e","witness_id":"w2"}]},"registry_checkpoint_hash":"cfeea12dc8941b414846eeb12fc04f83236f432a4ba6a96c5239d225495c20d9","registry_post_root":"ee53bde70bab3b327c5059997367c57107fd86da9c8d7e3a13fd4f35e788e8bc","registry_proof":{"cas_receipt":{"cas_request_hash":"a83754758456e9ab9d2b27a2141c05055a912c5b5b30189b76d23d27c03e95df","post_log_root":"1594c2a3fb8594e812e153d4552a0064b6af01e361eb8cc81fa0690be6983ef9","post_map_root":"ee53bde70bab3b327c5059997367c57107fd86da9c8d7e3a13fd4f35e788e8bc","retirement_log_index":"0","version":"publish_and_retire_receipt_v1"},"cas_receipt_hash":"ea5e6d439cd695f7413bf0fd9669782d96b2f800588d999380e9c83ee20a7161","cas_request":{"core_evidence_root":"dc237632845c2fa9ab9892fe459df78f81cf683f86948d73409b3721e3bcd954","expected_prior_map_root":"6e74c6094342b96d8b898d34bffd3ae0bbd2618348d80e3b91fe90958dc3c1b3","fingerprint":"1111111111111111111111111111111111111111111111111111111111111111","version":"publish_and_retire_request_v1"},"cas_request_hash":"a83754758456e9ab9d2b27a2141c05055a912c5b5b30189b76d23d27c03e95df","log_consistency_path":[],"log_inclusion_path":[],"post_state":{"core_evidence_root":"dc237632845c2fa9ab9892fe459df78f81cf683f86948d73409b3721e3bcd954","fingerprint":"1111111111111111111111111111111111111111111111111111111111111111","state":"retired","version":"registry_state_v1"},"prior_checkpoint":{"authority_signature":"52bb1e046a3129c417b08406a9df72b66bc277aaf368ca65c6ffb817fdf9d8828d1af15dc53d3322be39ecfe311f7fd88ff3b15ca57e5bc64de5c79953165f00","body":{"log_root":"1472bde5eec6c0c5195573cbaf2b6e76df216f147f26d82a4327c37cffb6092a","log_size":"0","map_root":"6e74c6094342b96d8b898d34bffd3ae0bbd2618348d80e3b91fe90958dc3c1b3","namespace":"registry.test","previous_checkpoint_hash":"0000000000000000000000000000000000000000000000000000000000000000","version":"registry_checkpoint_v1"},"witness_signatures":[{"signature":"7b9ddf5af8c90e9ad7435f7042e87c07795c9e4af87df1dc160337cf6b509939ec2e20aeb60abde5d8e61ab2620343fec31e5d9e5b9b7a0f3758560fd22e1c09","witness_id":"w1"},{"signature":"1902759f0d658dadd0238d19dd754dd8eefb3988f363977bae9ceed179231e892b4fa2323b6ae050201066cca1f70bf60a639b1a41192cbb7da7e7716b744a05","witness_id":"w2"}]},"prior_state":{"core_evidence_root":null,"fingerprint":"1111111111111111111111111111111111111111111111111111111111111111","state":"reserved","version":"registry_state_v1"},"reserved_path":{"bitmap":"0000000000000000000000000000000000000000000000000000000000000000","siblings":[]},"retired_path":{"bitmap":"0000000000000000000000000000000000000000000000000000000000000000","siblings":[]},"retirement_log_leaf":{"cas_request_hash":"a83754758456e9ab9d2b27a2141c05055a912c5b5b30189b76d23d27c03e95df","core_evidence_root":"dc237632845c2fa9ab9892fe459df78f81cf683f86948d73409b3721e3bcd954","fingerprint":"1111111111111111111111111111111111111111111111111111111111111111","index":"0","map_key":"d88dce55ea5f76de35e3af99ff5e76031d665cec39bd80d757edd2d269abf14c","post_map_root":"ee53bde70bab3b327c5059997367c57107fd86da9c8d7e3a13fd4f35e788e8bc","post_state_hash":"d637d242ec135d9ee814377625dda14c51758fc4bdee1348beff99bdb026a990","prior_checkpoint_hash":"b255d46757cd1e051b036e0d62f9104c8dd27f3dd637b1c698be74fd3f722689","prior_map_root":"6e74c6094342b96d8b898d34bffd3ae0bbd2618348d80e3b91fe90958dc3c1b3","prior_state_hash":"880e1b8f8eb8cfff23b34c6dcd5d20f32a852bd25721f19a4c3d5db328b96a8c","version":"registry_retirement_log_leaf_v1"},"version":"registry_publication_proof_v1"},"release_fingerprint":"1111111111111111111111111111111111111111111111111111111111111111","version":"final_publication_envelope_v1"}
```

The complete positive envelope is:

```json
{"candidate_verdict":"INSUFFICIENT","candidate_verdict_hash":"ed925f1828c262c53507c2cff3b0223267fa3bb9e010f0d54b290788bdf9e5fe","core_evidence_root":"dc237632845c2fa9ab9892fe459df78f81cf683f86948d73409b3721e3bcd954","core_members":[{"byte_length":"59","path":"candidate_verdict_v1.json","sha256":"ed925f1828c262c53507c2cff3b0223267fa3bb9e010f0d54b290788bdf9e5fe"}],"registry_checkpoint":{"authority_signature":"74a2117deb44480267dc27e6864882e009201b0cee64a8a84485d098f4427aef5047c9b57ca1837cb0c261e3c16a9f6ed16ac6b0c43b83df6d64a668122b8805","body":{"log_root":"127789fa04472cb28670d664c00f485d8c3a289b1ee95d683f7da2f99ab8dae4","log_size":"2","map_root":"f17d04bec6350f15a7ed88b867dad3f7c4dd34fc21b431851f31482a2cdc3b8b","namespace":"registry.test","previous_checkpoint_hash":"f6b16571b719d6df22ecab65d0ce52ac679f860ab6a57731b9c3f560f87ed752","version":"registry_checkpoint_v1"},"witness_signatures":[{"signature":"20c2c702911dcaa8a1f4e64c8c3f21238452d45ad7be96fa0b463e62ce5da5c1ff8c031638e4787cbb70b25ac730940d7e5459d4aed22e4fbf9fd59cad3eee03","witness_id":"w1"},{"signature":"222b617e595418954c734f92b33a0410129eec1caee8be66ee9d87a886320ffdb89fabad4cc2e2bc500082098bdb837b60c2219136888f418e4e02916dad8801","witness_id":"w2"}]},"registry_checkpoint_hash":"c3c52028feff187a8f719f6195c9a31a6217927d1519d87c0aa2539d153d0d7b","registry_post_root":"f17d04bec6350f15a7ed88b867dad3f7c4dd34fc21b431851f31482a2cdc3b8b","registry_proof":{"cas_receipt":{"cas_request_hash":"d0fa0855bdd3d89237776b93d9d90fd1e4e3dd89260a2ca25a55f307e33cc880","post_log_root":"127789fa04472cb28670d664c00f485d8c3a289b1ee95d683f7da2f99ab8dae4","post_map_root":"f17d04bec6350f15a7ed88b867dad3f7c4dd34fc21b431851f31482a2cdc3b8b","retirement_log_index":"1","version":"publish_and_retire_receipt_v1"},"cas_receipt_hash":"02e44087ae0bd711152eb0087acd9fa6de8a31a6d2964f6e07d36ce8178c5aa8","cas_request":{"core_evidence_root":"dc237632845c2fa9ab9892fe459df78f81cf683f86948d73409b3721e3bcd954","expected_prior_map_root":"4e638ae52b2811f2c1936c43c943cd253b4af5f4d2911e5cd4d6f5acb2e506b9","fingerprint":"1111111111111111111111111111111111111111111111111111111111111111","version":"publish_and_retire_request_v1"},"cas_request_hash":"d0fa0855bdd3d89237776b93d9d90fd1e4e3dd89260a2ca25a55f307e33cc880","log_consistency_path":["bd90f8e499e2530f35823ac93e3c68ae1eb9c29601bffb3195812f926bb2df70"],"log_inclusion_path":["0c6fe96d864eb525a125ddc168e984c4321ccd65e31e1f5dd21413f86ba22a2f"],"post_state":{"core_evidence_root":"dc237632845c2fa9ab9892fe459df78f81cf683f86948d73409b3721e3bcd954","fingerprint":"1111111111111111111111111111111111111111111111111111111111111111","state":"retired","version":"registry_state_v1"},"prior_checkpoint":{"authority_signature":"67019e2efcf3c6f53e6927e459f7017988d2cc47ea451d1ef9d23e4c3a9c9f169d8b6f05e5c39404673ab4084150671f8e8ba1160985ad3fc902ac3df58cdb01","body":{"log_root":"0c6fe96d864eb525a125ddc168e984c4321ccd65e31e1f5dd21413f86ba22a2f","log_size":"1","map_root":"4e638ae52b2811f2c1936c43c943cd253b4af5f4d2911e5cd4d6f5acb2e506b9","namespace":"registry.test","previous_checkpoint_hash":"0000000000000000000000000000000000000000000000000000000000000000","version":"registry_checkpoint_v1"},"witness_signatures":[{"signature":"c13d294e3b99400af573c4010db831056b3b9dc0797996ecc535194513cfe96699fd6711494e8a9df6183462789d4dcb564dec348347895c79c363bd020f380a","witness_id":"w1"},{"signature":"627361d605be89751a63023262f244d694abd50bdf1880188e9dabf0202133024e6f4bb463943bf92925b2765e543ee7ad6a07f7cba37ed90aae6f5e3d16a30c","witness_id":"w2"}]},"prior_state":{"core_evidence_root":null,"fingerprint":"1111111111111111111111111111111111111111111111111111111111111111","state":"reserved","version":"registry_state_v1"},"reservation_log_inclusion_path":[],"reservation_log_leaf":{"fingerprint":"1111111111111111111111111111111111111111111111111111111111111111","index":"0","map_key":"d88dce55ea5f76de35e3af99ff5e76031d665cec39bd80d757edd2d269abf14c","post_map_root":"4e638ae52b2811f2c1936c43c943cd253b4af5f4d2911e5cd4d6f5acb2e506b9","post_state_hash":"7464168283f93a31c9013442faae087a43430da9465cb56976c8b5f7149c9f7f","prior_checkpoint_hash":"0000000000000000000000000000000000000000000000000000000000000000","prior_map_root":"b2635ff914e5ee3dcdebbc2e124c250c0506f6c712bd7479639480568f536f6f","version":"registry_reservation_log_leaf_v1"},"reserved_path":{"bitmap":"0000000000000000000000000000000000000000000000000000000000000000","siblings":[]},"retired_path":{"bitmap":"0000000000000000000000000000000000000000000000000000000000000000","siblings":[]},"retirement_log_leaf":{"cas_request_hash":"d0fa0855bdd3d89237776b93d9d90fd1e4e3dd89260a2ca25a55f307e33cc880","core_evidence_root":"dc237632845c2fa9ab9892fe459df78f81cf683f86948d73409b3721e3bcd954","fingerprint":"1111111111111111111111111111111111111111111111111111111111111111","index":"1","map_key":"d88dce55ea5f76de35e3af99ff5e76031d665cec39bd80d757edd2d269abf14c","post_map_root":"f17d04bec6350f15a7ed88b867dad3f7c4dd34fc21b431851f31482a2cdc3b8b","post_state_hash":"3a100d55d6eedf26bcde582688b8e7729dfb1059941e4aeb2387edae1d4423d0","prior_checkpoint_hash":"f6b16571b719d6df22ecab65d0ce52ac679f860ab6a57731b9c3f560f87ed752","prior_map_root":"4e638ae52b2811f2c1936c43c943cd253b4af5f4d2911e5cd4d6f5acb2e506b9","prior_state_hash":"7464168283f93a31c9013442faae087a43430da9465cb56976c8b5f7149c9f7f","version":"registry_retirement_log_leaf_v1"},"version":"registry_publication_proof_v1"},"release_fingerprint":"1111111111111111111111111111111111111111111111111111111111111111","version":"final_publication_envelope_v1"}
```

This positive envelope is 5,552 bytes; core root is `dc237632845c2fa9ab9892fe459df78f81cf683f86948d73409b3721e3bcd954`, prior/post registry checkpoint hashes are `f6b16571b719d6df22ecab65d0ce52ac679f860ab6a57731b9c3f560f87ed752` / `c3c52028feff187a8f719f6195c9a31a6217927d1519d87c0aa2539d153d0d7b`, reservation/retirement leaves are `0c6fe96d864eb525a125ddc168e984c4321ccd65e31e1f5dd21413f86ba22a2f` / `bd90f8e499e2530f35823ac93e3c68ae1eb9c29601bffb3195812f926bb2df70`, and final-envelope hash is `224a6f473797219a9f1a24c99ed43c68d4f8b1248f021c5b0d6e5912a4d2a5fc`. Replacing selected `w2` with valid available `w3` signature `93bbf11235f9e478c75b60eb276c82d238507b9ae4f2979df01314c5c19a5d2b99fbbb69fef7f477272a04fef7e9270105a65c87be86d675c8e58811cd0b5101` is invalid because the quorum subset is nonminimal.

### `visibility_proof_suite_v1`

The charter independently pins its authority namespace/genesis, pure Ed25519 key, `P = "cross-host-v2/visibility"`, witness history/quorum, and gated object namespace. Map key is `H(P+"/map-key-v1", UTF8(namespace), raw32(fingerprint), raw32(core_root))`. Absence has no value; the only serializable value is exactly `{"core_evidence_root":hex64,"final_publication_envelope_hash":hex64,"object_set_root":hex64,"registry_post_root":hex64,"state":"visible","version":"visibility_map_value_v1"}`, hashed as `H(P+"/map-value-v1", JCS(value))`. `object_set_root = SHA256(ASCII("cross-host-v2/visibility-object-set-v1") || 0x00 || raw32(core_root) || raw32(final_hash))`. The log leaf is exactly `{"index":dec,"map_key":hex64,"map_value_hash":hex64,"object_set_root":hex64,"post_map_root":hex64,"prior_checkpoint_hash":hex64,"prior_map_root":hex64,"version":"visibility_log_leaf_v1"}`.

The receipt is exactly `{"core_evidence_root":hex64,"final_publication_envelope_hash":hex64,"fingerprint":hex64,"map_key":hex64,"namespace":id,"object_set_root":hex64,"post_checkpoint_hash":hex64,"post_map_root":hex64,"prior_checkpoint_hash":hex64,"prior_map_root":hex64,"proof_suite_version":"visibility_proof_suite_v1","registry_post_root":hex64,"version":"visibility_seal_receipt_v1"}`. Pure Ed25519 signs `ASCII("cross-host-v2/visibility-seal-receipt-v1") || 0x00 || u64be(len(JCS(receipt))) || JCS(receipt)`. The suite is exactly `{"absence_path":transparency_path_v1,"log_consistency_path":[hex64...],"log_inclusion_path":[hex64...],"log_leaf":closed_leaf,"map_value":closed_visible_value,"post_checkpoint":checkpoint_v1,"post_inclusion_path":transparency_path_v1,"prior_checkpoint":checkpoint_v1,"receipt":closed_receipt,"receipt_signature":hex128,"version":"visibility_proof_suite_v1"}` and hashes as `H(P+"/proof-suite-v1", JCS(suite))`. It proves prior signed absence and one create-only leaf/map transition to the selected signed post checkpoint.

The object authority has one read gate: staged bytes have no public path; the transaction publishes the map value, log leaf, receipt record, exact envelope, and every core object atomically, and authenticated reads require the same currently proven `visible` value/object root. No `staged`, `partial`, direct-object, or receipt-only state exists. A collision or failed compare-and-create mutates/exposes nothing. Historical verification rehashes all bytes and requires prior/post checkpoints, absence/inclusion, log inclusion/consistency, receipt, and actual reads to agree; any missing/early/partial object is `authority_unresolved` / `INSUFFICIENT`.

The positive vector reuses the complete envelope, fingerprint, and registry post root above; visibility namespace is `visibility.test`, authority/w1/w2/w3 seeds are bytes `05/06/07/08` repeated 32, and quorum is 2. Expected map key/value hash are `1019bf0ec2bba4767c574a24ed596b62e32e49b9815d9ddb2aba2de067af4386` / `b60a5da7a9e8bd34009158f7c54129136329b94e5f2a1a700b6e09a46ead6e67`; absence/visible roots are `b43d082bee004005cda1bdf0f24f8cc23578bdb8b8f2f270b67ea600aef100ee` / `7cfbb993340be966bdab8e45250752c28080cd9f6274657d744f7cf5da1e51e3`; prior/post checkpoint hashes are `a3202af05a52bfda22baacd13ecd2c90c15045bbc1349c5d11d8143475f88ead` / `ddb0570a88f022cd54da56c9dc8e5fdf777c6d1974eb1ff33ad60698d5cec917`; log leaf/object root are `fb4a596116ecd312d5ff9c9c01373aded4ae46082acd0740383e794588d9d35e` / `3f305a57919b0df0236d8a7ba8c8b81a4660cfe40a2242e1e70e8c04624cf32f`; receipt signature is `f5d65be70e8d92508bbc60092107585c1e6d4d6260c1859f0557e9f9dc02f2dfdfdc06c55d173cbd136f3efb523283ad768d0d5c0f7246a040bcab268e1c7403`; suite hash is `fb3bd78938c93932ccf38096c5a992fb7c8d3a8f5471d5c86bbaaaa4c4bb6056`.

The complete 4,181-byte positive suite is:

```json
{"absence_path":{"bitmap":"0000000000000000000000000000000000000000000000000000000000000000","siblings":[]},"log_consistency_path":[],"log_inclusion_path":[],"log_leaf":{"index":"0","map_key":"1019bf0ec2bba4767c574a24ed596b62e32e49b9815d9ddb2aba2de067af4386","map_value_hash":"b60a5da7a9e8bd34009158f7c54129136329b94e5f2a1a700b6e09a46ead6e67","object_set_root":"3f305a57919b0df0236d8a7ba8c8b81a4660cfe40a2242e1e70e8c04624cf32f","post_map_root":"7cfbb993340be966bdab8e45250752c28080cd9f6274657d744f7cf5da1e51e3","prior_checkpoint_hash":"a3202af05a52bfda22baacd13ecd2c90c15045bbc1349c5d11d8143475f88ead","prior_map_root":"b43d082bee004005cda1bdf0f24f8cc23578bdb8b8f2f270b67ea600aef100ee","version":"visibility_log_leaf_v1"},"map_value":{"core_evidence_root":"dc237632845c2fa9ab9892fe459df78f81cf683f86948d73409b3721e3bcd954","final_publication_envelope_hash":"224a6f473797219a9f1a24c99ed43c68d4f8b1248f021c5b0d6e5912a4d2a5fc","object_set_root":"3f305a57919b0df0236d8a7ba8c8b81a4660cfe40a2242e1e70e8c04624cf32f","registry_post_root":"f17d04bec6350f15a7ed88b867dad3f7c4dd34fc21b431851f31482a2cdc3b8b","state":"visible","version":"visibility_map_value_v1"},"post_checkpoint":{"authority_signature":"4a13d51593dbbafbcab8ab79fcc1f2d2eee235cfd971b379132a128175fbcc4901f8e290e170e23c57570475249cecfeee8b8606a6c5122134ed9a6173fba601","body":{"log_root":"fb4a596116ecd312d5ff9c9c01373aded4ae46082acd0740383e794588d9d35e","log_size":"1","map_root":"7cfbb993340be966bdab8e45250752c28080cd9f6274657d744f7cf5da1e51e3","namespace":"visibility.test","previous_checkpoint_hash":"a3202af05a52bfda22baacd13ecd2c90c15045bbc1349c5d11d8143475f88ead","version":"visibility_checkpoint_v1"},"witness_signatures":[{"signature":"aef3069062b3ad44c9698d9a95025def4319ab870d2c58dbe5d5dd48f92bb353400d21e3260b2afdeae2d89b694b43784f424294bb6b77aeeeda87793bbe4a09","witness_id":"w1"},{"signature":"6ce9a05de0940e13a393447420cb0045bb10daa2aac9915ccd016fc0fdcbd3f62bd628cf8bfb329ea617224f54466ea2f0e2928a3d71c7a563a0b4c84ad5c10e","witness_id":"w2"}]},"post_inclusion_path":{"bitmap":"0000000000000000000000000000000000000000000000000000000000000000","siblings":[]},"prior_checkpoint":{"authority_signature":"fc77cd15dc790da69a6b9760934876dc62df1894551dd2b96c0ebe6079e4925c9d253075c3a90a3050a65edc8661c7c5547e14cbd6cee574576c64570434b706","body":{"log_root":"95f1f6dd9aa9ca00c3cb014e3a0d08129607989dafc632af2fa73dbac010bb51","log_size":"0","map_root":"b43d082bee004005cda1bdf0f24f8cc23578bdb8b8f2f270b67ea600aef100ee","namespace":"visibility.test","previous_checkpoint_hash":"0000000000000000000000000000000000000000000000000000000000000000","version":"visibility_checkpoint_v1"},"witness_signatures":[{"signature":"6bc00ecc4092e0b0826f830e248cb308d4144908f1b28049d582f942ca36682ac83ab30689dec76d4f6d59b9930d9b3a1aeba956b431f2d739bdb34314e63e0e","witness_id":"w1"},{"signature":"f9da15756a947cc2198c646ff101ff067f48c11b3ef4114d6848585b1c651697630fe0783e405b3b9dd584866a4228f5dd036a0019b8ba3b7874ef1d6f524701","witness_id":"w2"}]},"receipt":{"core_evidence_root":"dc237632845c2fa9ab9892fe459df78f81cf683f86948d73409b3721e3bcd954","final_publication_envelope_hash":"224a6f473797219a9f1a24c99ed43c68d4f8b1248f021c5b0d6e5912a4d2a5fc","fingerprint":"1111111111111111111111111111111111111111111111111111111111111111","map_key":"1019bf0ec2bba4767c574a24ed596b62e32e49b9815d9ddb2aba2de067af4386","namespace":"visibility.test","object_set_root":"3f305a57919b0df0236d8a7ba8c8b81a4660cfe40a2242e1e70e8c04624cf32f","post_checkpoint_hash":"ddb0570a88f022cd54da56c9dc8e5fdf777c6d1974eb1ff33ad60698d5cec917","post_map_root":"7cfbb993340be966bdab8e45250752c28080cd9f6274657d744f7cf5da1e51e3","prior_checkpoint_hash":"a3202af05a52bfda22baacd13ecd2c90c15045bbc1349c5d11d8143475f88ead","prior_map_root":"b43d082bee004005cda1bdf0f24f8cc23578bdb8b8f2f270b67ea600aef100ee","proof_suite_version":"visibility_proof_suite_v1","registry_post_root":"f17d04bec6350f15a7ed88b867dad3f7c4dd34fc21b431851f31482a2cdc3b8b","version":"visibility_seal_receipt_v1"},"receipt_signature":"f5d65be70e8d92508bbc60092107585c1e6d4d6260c1859f0557e9f9dc02f2dfdfdc06c55d173cbd136f3efb523283ad768d0d5c0f7246a040bcab268e1c7403","version":"visibility_proof_suite_v1"}
```

Tamper vectors all derive `authority_unresolved` / `INSUFFICIENT`: change final hash byte without rebuilding the value/root/receipt; flip any receipt-signature bit; replace selected post-checkpoint `w2` with valid nonminimal `w3` signature `248bbc91081d090e47bc6669af2a4c2b9c37a6e3339559ac4873546ba2e3992a71835bb9b744ad01d1ff44f8bb4f1b51042e52e0e5bb2192300a5b16703b5000`; reorder/omit a proof sibling; claim an object read before the atomic `visible` transition; or expose fewer/different bytes than the object root.

`publication_state_v1` is closed and evidence-derived: `reserved_unpublished` requires exact reservation plus authenticated visibility absence; `retired_unpublished` requires exact retirement for this core plus absence; `authority_unresolved` covers unavailable, stale, malformed, conflicting, alternate-authority, or wrong-tuple evidence; `visible` requires exact retirement, valid visibility receipt/proofs, and all exact bytes. Timestamp/local/bundle assertions never derive state. Registry and visibility checkpoints are independent verifier inputs.

| Crash boundary | Closed recovery/result |
|---|---|
| Before core freeze | Proven reservation/absence is `reserved_unpublished`; rebuild private staging only from sealed inputs. Drift/loss is `INSUFFICIENT`. |
| Core frozen; CAS unknown | Exact reserved/absent permits exact CAS retry; exact retired/absent recovers proof as `retired_unpublished`; unavailable/conflicting/different-root evidence is `authority_unresolved` / `INSUFFICIENT` with no mutation. |
| Retired; final not sealed | `retired_unpublished`; rebuild/rescan only the byte-identical envelope for the same core, proof, and checkpoint selection. |
| Final envelope frozen/hashed; visibility absent | Preserve or deterministically rebuild the same 5,552-byte closed envelope, require the frozen hash to match, and retry only `seal_visible`; drift is `authority_unresolved` / `INSUFFICIENT`, never a new CAS or evaluation. |
| Visibility call unknown | Exact receipt/proofs and all objects derive `visible`; authenticated absence permits the exact retry; partial/different/multiple/forged/unavailable evidence derives `authority_unresolved` / `INSUFFICIENT`. |
| `visible` / final completion | Append `publication_complete_v1` only after exact retirement, envelope hash, minimal checkpoints/quorums, proof suite, read gate, and every byte verify. A crash before that record re-verifies/query-recovers without a second seal; no retry/replacement follows completion. |

Failures/recovery are append-only and preserve the last provable state. Non-security gaps are `partial_non_security`/`INSUFFICIENT`; verified private leakage remains `partial_security`/`FAIL`. Recovery never creates a second retirement or visibility transition.

## Implementation Slices

These are sequential handoffs, not permission to run live hosts:

1. **Executable inputs/runtime** — build schemas/tasks, exact plans, isolation, source/condition surfaces, scorer, causal records, failure retention, and caps.
2. **Publication pipeline, four separately persisted/tested steps** — (1) close writers, recompute/scan members, freeze `core_evidence_root`; (2) exact idempotent CAS `publish_and_retire`; (3) build, validate, scan, freeze, and hash the closed final envelope, including crash rebuild before visibility; (4) atomic `seal_visible`, verify the full proof suite/read gate, then append final completion without a second seal.
3. **Evidence/status** — separately authorize smoke/full matrix, then publish only verified artifacts and update every current-status document together.

Shared schema/report files have one owner at a time and transfer ownership before another edit. Production user identity remains an external reviewed prerequisite.

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

- exact 288/144 tuple planning, missing/duplicate rejection, and dry-run zero host spawns;
- source episode single-execution, condition-neutral seal, remem-preparation failure fan-out, interruption, retries, and tuple/attempt key lifetime;
- same canonical project path positive transfer and decoy path exclusion;
- pre-open sealed/clone hash checks, disposable-open preflight, full-store denial, raw-free searches, and correct/wrong/source/missing-key checks;
- exporter immutability, atomic commit/retry/failure scope, fixed protocol and budget rules, condition isolation, and target-native source-seal denial;
- counterbalanced schedule equality and planned/realized-order drift;
- native import pairing with neutral counts, Claude snapshot/drift rejection, authorized inserted lineage, canary typed exclusion/zero rows, and raw-free projections;
- causal-oracle pre-action/counterfactual positive, refuted, Stop-only, ambiguous/missing cases, and failed-run retention;
- automatic-only review, exact attempt refs, preparation/infrastructure classification, immutable full-stream/framing-closure hash/send, `M/R/N/T` reveal crash/tamper/duplicate/conflict/no-write truth table, and null/no-scorer exhaustion;
- required stale challenge pre-filter inventory and empty-applicable-set rejection;
- source failure before seal with immutable lifecycle/cost/cleanup evidence, plus complete, non-security partial, and security partial reports;
- cross-implementation RFC 8785, core Merkle, closed final-envelope/registry/visibility schemas and positive/tamper vectors; invalid paths, empty/odd trees, wrong field/order/type/hash/proof/quorum, deterministic Markdown, and hash-bound verdict;
- scorer-oracle missing/hash/tamper/unrunnable and clean-checkout recomputation;
- pre-run scorer/read-set commitment, frozen workspace/output projection, scorer-oracle disagreement, and result-affecting undisclosable-input rejection;
- published stream missing/redacted/private-result byte, manifest gap/overlap/duplicate/relabel/closure expansion, slice/root drift, out-of-closure mutation, and rolling-send mismatch: `INSUFFICIENT` unless verified leakage yields `FAIL`;
- early oracle/opening visibility (`INSUFFICIENT`), raw/encoded private leak (`FAIL`), scanner crash (`INSUFFICIENT`), and duplicate publication semantics;
- authoritative-registry fake/alternate namespace/key/genesis, stale/forked/split-view root, missing quorum/gossip/external checkpoint, invalid proofs, CAS race, missing retirement, and fingerprint reuse: reject pre-call; observed evidence is `INSUFFICIENT`;
- publication-DAG cycle/self-hash/final-field-in-core rejection, core drift, different-core CAS replay, same-core recovery, final-freeze/visibility crash rebuild, completion recovery, and every closed state transition;
- alternate visibility authority/key, forged/mismatched receipt/proof/checkpoint, nonminimal quorum, non-atomic/early/partial object exposure, changed-envelope retry, collision, and double transition/seal;
- fixed bootstrap framing/rejection/quantile vectors, adjusted regression, and PASS/FAIL/INSUFFICIENT edges;
- secret-channel topology and raw/hex/versioned key residue scans;
- user-scope `not_testable` rejection of fabricated zero and numeric wrong-user zero-tolerance;
- per-invocation call/cost cap enforcement and CI live-call denial.

Before any implementation PR is submitted:

```bash
cargo fmt --check
cargo check
cargo clippy --all-targets -- -D warnings
cargo test
python3 scripts/ci/check_plugin_version_sync.py
```

Live smoke or matrix output is not a substitute for these tests, and these tests are not authorization for live execution.
