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

Fixed vectors (byte strings are hex; paths are ASCII):

| Vector | Input | Expected lowercase SHA-256 hex |
|---|---|---|
| leaf 1 | path `a.txt`, bytes empty | `71caa83e102f0a84aeeb9fb2c28aee41bee3645d1ce80549f1ebcb63f02171af` |
| leaf 2 | path `dir/b.bin`, bytes `00ff` | `4d025417f7adf5ebc354f2437803ce0fba6fbc772aa0e82198a215ae28a13f58` |
| leaf 3 | path `z.txt`, UTF-8 bytes of `remem` | `b1bde8b9a236ca646e52fbcfed709026e3b52378299726a77a535dccfd9d652d` |
| three-leaf root | the ordered leaves above; duplicate leaf 3 at level one | `3aae31029e37d4de1553dd856615389208808c68e6bc9a5f82a92bcc2e2f527c` |
| final-hash framing (not a schema object) | RFC-8785 bytes `7b2261223a317d` (`{"a":1}`, length 7) | `9abf4bd745be0f13e3a17928a74a3b5d57292e26c24810677b81ee447a879c04` |
| visibility object set | raw core-root and final-envelope digests from the two rows above | `200f9238e30ce0ea0b03770fb81dfb2e66da4c64a0ddd8d4e2f594e3308a1661` |

`release_revision_registry_v1` pins namespace/genesis, log/map key, witness quorum/gossip/freshness, and proof suite in the charter. Plan/preflight bind a fresh independently witnessed prior root, genesis consistency, `unused` proof, and reservation; verification independently obtains a matching-or-later checkpoint and validates genesis -> prior -> reservation -> retirement -> post -> checkpoint.

Atomic `publish_and_retire(fingerprint, core_evidence_root)` changes that registry `reserved -> retired`, exposes no bytes, and returns post-root/retirement/inclusion/consistency/state/non-reuse proofs; exact replay returns the committed proof and a different core is rejected. Deterministic `final_publication_envelope_v1` binds the core list/root, candidate-verdict hash/value, CAS request/receipt/proofs/post-root, and the lowest append-log-index canonical checkpoint at/after post-root that has the charter-pinned quorum plus its history proof, excluding its own hash and visibility receipt. Its RFC-8785 bytes `e` have `final_publication_envelope_hash_v1 = hex64(SHA256(ASCII("cross-host-v2/final-publication-envelope-v1") || 0x00 || u64be(len(e)) || e))`.

The charter separately pins `publication_visibility_authority_v1` namespace/genesis, pure Ed25519 RFC-8032 receipt key, append-log/map key, `visibility_proof_suite_v1`, witness quorum/gossip/freshness, and object namespace. For raw core/final digests `c/f`, `visibility_object_set_root_v1 = hex64(SHA256(ASCII("cross-host-v2/visibility-object-set-v1") || 0x00 || c || f))`. `seal_visible(fingerprint, core_root, final_hash)` verifies the private staged envelope/core set, then one transaction changes the authenticated key from absent to visible and exposes exactly those bytes. Canonical `visibility_seal_receipt_v1` binds authority identity, key tuple, core/final/registry-post/object-set roots, visibility prior/post roots, proof-suite version, and the lowest-index quorum-witnessed checkpoint at/after the visibility post-root; pure Ed25519 signs `ASCII("cross-host-v2/visibility-seal-receipt-v1") || 0x00 || u64be(len(JCS(receipt_without_signature))) || JCS(receipt_without_signature)`, and its raw 64-byte signature is serialized as 128 lowercase hex characters. Verifier requires that signature, an independently obtained matching-or-later witnessed checkpoint, append inclusion/consistency, absent-to-visible state and atomic object-set proofs, plus every rehashed object. Alternate authority/key, forged/mismatched receipt, partial/different objects, collision, or missing proof is invalid.

`publication_state_v1` is closed and evidence-derived: `reserved_unpublished` requires exact reservation plus authenticated visibility absence; `retired_unpublished` requires exact retirement for this core plus absence; `authority_unresolved` covers unavailable, stale, malformed, conflicting, alternate-authority, or wrong-tuple evidence; `visible` requires exact retirement, valid visibility receipt/proofs, and all exact bytes. Timestamp/local/bundle assertions never derive state. Registry and visibility checkpoints are independent verifier inputs.

| Crash boundary | Closed recovery/result |
|---|---|
| Before core freeze | Proven reservation/absence is `reserved_unpublished`; rebuild private staging only from sealed inputs. Drift/loss is `INSUFFICIENT`. |
| Core frozen; CAS unknown | Exact reserved/absent permits exact CAS retry; exact retired/absent recovers proof as `retired_unpublished`; unavailable/conflicting/different-root evidence is `authority_unresolved` / `INSUFFICIENT` with no mutation. |
| Retired; final not sealed | `retired_unpublished`; rebuild/rescan only the byte-identical envelope for the same core, proof, and checkpoint selection. |
| Visibility call unknown | Exact receipt/proofs and all objects derive `visible`; authenticated absence permits the exact retry; partial/different/multiple/forged/unavailable evidence derives `authority_unresolved` / `INSUFFICIENT`. |
| `visible` | No retry/replacement. Correction requires a new unused fingerprint; old remains retired and visible. |

Failures/recovery are append-only and preserve the last provable state. Non-security gaps are `partial_non_security`/`INSUFFICIENT`; verified private leakage remains `partial_security`/`FAIL`. Recovery never creates a second retirement or visibility transition.

## Implementation Slices

These are sequential handoffs, not permission to run live hosts:

1. **Executable contracts/fixtures** — bump charter/schemas, build 24 deterministic tasks, and validate export/causal protocols, exact 288/144 plans, canonical JSON, and partial manifests.
2. **Isolation/source/conditions** — implement same-path isolation, source sealing, neutral evidence, failure fan-out, caps, all primary surfaces, and native-import lineage/canary rules.
3. **Score and core freeze** — close writers; build/recompute/scan prompt, scorer, causal, manifest, report, candidate-verdict, exact hash vectors, canonical member list, then freeze `core_evidence_root`.
4. **Registry retirement** — call only exact idempotent `publish_and_retire(fingerprint, core_evidence_root)` and persist independently verifiable transition proofs; no evidence becomes visible.
5. **Final envelope and visibility** — build/rescan/hash the byte-identical final envelope, then perform the independent authority's atomic create-only seal; implement all `publication_state_v1`, crash/retry/collision/partial-visibility paths before exposing a claim.
6. **Evidence/status** — separately authorize the six-surface smoke, optionally authorize the matrix only after review, then publish verified artifacts and update every current-status document together.

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

- exact 288/144 tuple planning, missing/duplicate rejection, and dry-run zero
  host spawns;
- source episode single-execution, condition-neutral seal, remem-preparation
  failure fan-out, interruption, retries, and tuple/attempt key lifetime;
- same canonical project path positive transfer and decoy path exclusion;
- pre-open sealed/clone hash checks, disposable-open preflight, full-store
  denial, raw-free searches, and correct/wrong/source/missing-key checks;
- exporter immutability, atomic commit/retry/failure scope, fixed protocol and
  budget rules, condition isolation, and target-native source-seal denial;
- counterbalanced schedule equality and planned/realized-order drift;
- native import pairing with neutral counts, Claude snapshot/drift rejection,
  authorized inserted lineage, canary typed exclusion/zero rows, and raw-free projections;
- causal-oracle pre-action/counterfactual positive, refuted, Stop-only, ambiguous/missing cases, and failed-run retention;
- automatic-only review, exact attempt refs, preparation/infrastructure
  classification, immutable full-stream/framing-closure hash/send, `M/R/N/T`
  reveal crash/tamper/duplicate/conflict/no-write truth table, and null/no-scorer exhaustion;
- required stale challenge pre-filter inventory and empty-applicable-set
  rejection;
- source failure before seal with immutable lifecycle/cost/cleanup evidence, plus
  complete, non-security partial, and security partial reports;
- cross-implementation RFC 8785, `core_evidence_merkle_v1`, and final-envelope fixed vectors; invalid/aliased/duplicate paths, empty trees, odd levels, deterministic Markdown, and hash-bound verdict;
- scorer-oracle missing/hash/tamper/unrunnable and clean-checkout recomputation;
- pre-run scorer/read-set commitment, frozen workspace/output projection,
  scorer-oracle disagreement, and result-affecting undisclosable-input rejection;
- published stream missing/redacted/private-result byte, canonical-manifest
  gap/overlap/duplicate/relabel/closure expansion, slice/root rehash drift,
  out-of-closure paired mutation, and rolling-send/object mismatch, all
  `INSUFFICIENT` unless verified private leakage yields `FAIL`;
- early oracle/opening visibility (`INSUFFICIENT`), raw/encoded private leak
  (`FAIL`), scanner crash (`INSUFFICIENT`), and duplicate publication semantics;
- authoritative-registry fake empty/local/alternate namespace or key,
  wrong genesis, stale/forked/same-key split-view root, missing witness quorum/
  gossip/bundle-external checkpoint, invalid non-membership/inclusion/
  consistency/state proof, CAS race, missing retirement, and fingerprint reuse:
  reject pre-call; observed candidate evidence is `INSUFFICIENT`;
- publication-DAG cycle/self-hash/final-field-in-core rejection, core drift,
  different-core CAS replay, exact same-core recovery, and every closed
  `publication_state_v1` crash/retry transition before/after freeze/CAS/seal;
- alternate visibility authority/namespace/key, forged or mismatched Ed25519
  receipt/proof/checkpoint, non-atomic object exposure, changed-envelope retry,
  create-only collision, partial/multiple visibility, and double transition/seal;
- fixed bootstrap framing/rejection/quantile vectors, adjusted regression, and
  PASS/FAIL/INSUFFICIENT edges;
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

Live smoke or matrix output is not a substitute for these tests, and these
tests are not authorization for live execution.
