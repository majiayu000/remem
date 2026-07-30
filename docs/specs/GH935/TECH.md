# Cross-Host Continuity Benchmark — Technical Contract

Status: Current contract; v1 infrastructure shipped, completion unimplemented; Issue: #935

## Existing Implementation Facts

- `eval/cross-host/benchmark-charter.json` is `cross-host-v1` / `infrastructure_only_no_runs`.
- `eval/cross-host/tasks/` contains 24 `skeleton_todo` JSON tasks.
- `cross-host-task.schema.json`, `cross-host-run.schema.json`, `schema_validate.py`, `scan_artifacts.py`, and `run_dry.py` are offline; no Rust runner/report exists.
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

`partial_evidence` and `complete_evidence` describe artifact availability, not claim success. Verdict is separately `PASS`, `FAIL`, or `INSUFFICIENT`.

## Planned Layout

The implementation should extend existing surfaces instead of adding a second benchmark framework:

| Area | Planned ownership |
|---|---|
| Charter, task fixtures, schemas, offline validators | `eval/cross-host/` |
| Shared process/HOME/workspace isolation | reusable primitives extracted from `src/eval/coding_bench/` |
| Cross-host plan, source seal, conditions, runner, score, artifacts | `src/eval/cross_host/` plus `src/eval/cross_host.rs` |
| CLI wiring | existing `bench` command modules under `src/cli/` |
| Sanitized release evidence and reports | versioned paths under `eval/cross-host/evidence/` and `eval/cross-host/reports/` |
| Current-status documentation | `eval/cross-host/README.md`, this spec pair, the spec index, and public-memory benchmark contracts |

## Contract Objects

All claim-bearing JSON uses snake_case, closed enums, and `canonical_json_rfc8785_v1`: parsed I-JSON is serialized exactly by RFC 8785 JCS. JCS fixes recursive raw-property sorting by unsigned UTF-16 code units, ECMAScript primitive serialization, preserved array order, no Unicode normalization, and zero token whitespace.
Output is UTF-8 with no BOM or trailing newline. Schemas reject unknown/duplicate keys, lone surrogates, NaN/Infinity, and numbers outside finite IEEE-754 binary64. Precision-sensitive integers outside `[-(2^53-1), 2^53-1]` use schema-typed strings matching `0|[1-9][0-9]*|-[1-9][0-9]*`. The plan binds the algorithm/version.

### Executable Task

An executable task adds to the v1 skeleton:

- immutable fixture repository revision and canonical workspace recipe;
- two or more chronological source episodes;
- source-host expected evidence and allowed/forbidden paths;
- hidden tests and non-empty score commands;
- target-blind gold facts;
- foreign-project decoy fixture and canary;
- for `stale_superseded_decision`, at least one deterministic stale/superseded challenge per run with its production state and expected query-relevant pre-filter match assertion;
- target-invisible `causal_oracle_v1` rules covering every scorer-recognized wrong-action class,
  each binding a unique memory/content/state hash, artifact matcher, and one plan-hashed proof method plus its closed outcome table;
- `pre_run_scorer_commitment_v1`, which commits all result-affecting scorer/oracle semantics and bytes or their sole deterministic derivation before any live call;
- source-native snapshot policy for the diagnostic arm;
- per-host executable/profile requirements;
- `status: "ready"` and `todo: []`.

The task validator rejects empty values, aliases, direction/host mismatch, target-visible hidden paths, and any canary copied into prompts or gold facts.

### Source Seal

One source seal exists per `(direction, task_id, run_index, source_attempt_id)` and contains:

- task/fixture, executable, profile, model, schema, and migration hashes;
- `condition_neutral_episode_evidence_v1`, binding source host/session/event IDs, transcript/tool-event/Git/workspace hashes, and the source-native manifest or typed native absence;
- canonical project path and the exact `project_from_cwd` value;
- terminal extraction/review state and queue counts;
- a sorted data-only manifest for the quiesced full source `REMEM_DATA_DIR`, with typed exclusions for `.key`, environment-provided keys, WAL/SHM, sockets, and other runtime-only secret/process files;
- a deterministic data-only full-store archive hash, length, and creation policy, or `remem_preparation_failed(stage, reason)` typed absence; exclusions record type, never value;
- target-transfer projection logical/ciphertext hashes, sorted manifest, non-secret `projection_key_id`, policy versions, forbidden-surface proof, or matching typed absence;
- `native_neutral_base_v1` hash, policy, zero native candidate/active/provenance counts, referential-integrity proof, or typed diagnostic-preparation absence;
- source-host native file manifest/hash, or typed absence;
- `maintained_export_v1` hash plus every cycle input/state/usage/budget; a commit carries prior/output/freeze hashes, while failure carries a typed reason and no output/freeze hash;
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

Before auth or preparation, the outer runner creates an append-only `cross-host-source-attempt-v2` journal. Finalization seals IDs/input hashes, lifecycle timestamps, nullable condition-neutral evidence hash/sealed-at boundary, terminal reason, typed preparation absences, cost, cleanup, and leak scans. This distinguishes failure before common evidence from later condition failure. Recovery appends and seals a new record; it never rewrites history.

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

`checkpoint_certificate_v1` is exactly `{"authority_signature":hex128,"body":{"certificate_epoch":dec,"log_root":hex64,"log_size":dec,"map_root":hex64,"namespace":id,"previous_checkpoint_hash":hex64,"transition_index":dec|null,"transition_leaf_hash":hex64|null,"version":certificate_literal,"witness_set_id":id},"witness_signatures":[{"signature":hex128,"witness_id":id}...]}`, where the literal is `registry_checkpoint_certificate_v1` or `visibility_checkpoint_certificate_v1`. Its hash is `H(P+"/checkpoint-certificate-v1", JCS(body))`; pure Ed25519 authority/witness messages use `P+"/checkpoint-certificate-authority-v1"` and `P+"/checkpoint-certificate-witness-v1"` followed by `0x00 || raw32(hash)`. Except the charter-pinned size-zero genesis certificate, the authority CAS atomically assigns a never-reused epoch, exact transition index/leaf, `log_size = transition_index + 1`, roots, and one charter-pinned `witness_set_id`; that set names exactly `q` ordered witness IDs and every one must sign. The certificate is create-only, its hash is in the transaction receipt, and exact replay returns it. Extra/alternate/late signatures, a later checkpoint, another epoch/set, or any body drift is invalid and can never change frozen envelope bytes.

For `P = "cross-host-v2/registry"`, map key, state, request, receipt, and both log-leaf hashes are respectively `H(P+"/map-key-v1", UTF8(namespace), raw32(fingerprint))`, `H(P+"/state-v1", JCS(state))`, `H(P+"/cas-request-v1", JCS(request))`, `H(P+"/cas-receipt-v1", JCS(receipt))`, and `H(P+"/log-leaf-v1", JCS(leaf))`. The closed `registry_publication_proof_v1` fields are: pre-reservation certificate/absence path; reservation leaf, checkpoint certificate, post-state path, log inclusion, and prior-to-reservation consistency; CAS-prior certificate, reserved-state path, reservation inclusion at CAS, and reservation-to-CAS consistency; request/hash, receipt/hash, retirement leaf, retired-state path, post certificate, retirement inclusion, and CAS-to-post consistency; the two closed states and literal version. A pre-reservation map may be non-empty and the log may contain unrelated leaves: `0 <= reservation_index < retirement_index`; pre-reservation, reservation, CAS-prior, and post certificate sizes are exactly `reservation_index`, `reservation_index + 1`, `retirement_index`, and `retirement_index + 1`. Every path consumes exactly its declared index/sizes and proves immediate absence -> reserved -> retired across all intervening history. Indices `0/1`, empty maps, and empty paths are fixture values only, never the generic contract.

`final_publication_envelope_v1` is exactly `{"candidate_verdict":PASS|FAIL|INSUFFICIENT,"candidate_verdict_hash":hex64,"core_evidence_root":hex64,"core_members":[{"byte_length":dec,"path":canonical_path,"sha256":hex64}...],"registry_checkpoint":checkpoint_certificate_v1,"registry_checkpoint_hash":hex64,"registry_post_root":hex64,"registry_proof":registry_publication_proof_v1,"release_fingerprint":hex64,"version":"final_publication_envelope_v1"}`. It excludes its own hash/visibility data; all members rehash to the core root, the candidate hash names the full binding object, and every registry field/proof/certificate agrees. After scan/freeze, exact bytes `e` have `final_publication_envelope_hash_v1 = hex64(SHA256(ASCII("cross-host-v2/final-publication-envelope-v1") || 0x00 || u64be(len(e)) || e))`.

The retained `0/1` one-member examples are deleted: they were useful only as negative framing toys and are not evidence for this contract. A production implementation must pass the closed generator below; no summary-only member list, empty-map registry proof, or fixture-only index may satisfy publication.

#### Closed registry objects and general history

The serialized registry state is exactly `{"core_evidence_root":hex64|null,"fingerprint":hex64,"state":"reserved"|"retired","version":"registry_state_v1"}`; `reserved` requires a null core and `retired` a non-null core. Unrelated append leaves are exactly `{"index":dec,"map_key":hex64,"post_map_root":hex64,"prior_map_root":hex64,"transition_id":hex64,"version":"registry_unrelated_log_leaf_v1"}`. The reservation leaf is exactly `{"fingerprint":hex64,"index":dec,"map_key":hex64,"post_map_root":hex64,"post_state_hash":hex64,"prior_map_root":hex64,"version":"registry_reservation_log_leaf_v1"}`.

The CAS request is exactly `{"core_evidence_root":hex64,"expected_prior_map_root":hex64,"fingerprint":hex64,"version":"publish_and_retire_request_v1"}`; its receipt is exactly `{"cas_request_hash":hex64,"checkpoint_certificate_hash":hex64,"post_log_root":hex64,"post_map_root":hex64,"retirement_log_index":dec,"version":"publish_and_retire_receipt_v1"}`. The retirement leaf is exactly `{"cas_request_hash":hex64,"core_evidence_root":hex64,"fingerprint":hex64,"index":dec,"map_key":hex64,"post_map_root":hex64,"post_state_hash":hex64,"prior_checkpoint_hash":hex64,"prior_map_root":hex64,"prior_state_hash":hex64,"version":"registry_retirement_log_leaf_v1"}`. Unknown fields, numeric JSON indices, noncanonical decimal strings, and mismatched derived hashes are rejected.

`registry_publication_proof_v1` is the closed JCS object with exactly: `cas_prior_checkpoint`, `cas_prior_state_path`, `cas_receipt`, `cas_receipt_hash`, `cas_request`, `cas_request_hash`, `cas_to_post_consistency_path`, `genesis_checkpoint`, `genesis_to_pre_consistency_path`, `namespace`, `post_checkpoint`, `pre_reservation_absence_path`, `pre_reservation_checkpoint`, `pre_to_reservation_consistency_path`, `reservation_at_cas_inclusion_path`, `reservation_checkpoint`, `reservation_log_inclusion_path`, `reservation_log_leaf`, `reservation_state`, `reservation_state_path`, `reservation_to_cas_consistency_path`, `retired_state`, `retired_state_path`, `retirement_log_inclusion_path`, `retirement_log_leaf`, and literal `version`. Checkpoints and states use the exact schemas above; paths use `transparency_path_v1` or ordered `hex64[]`. The verifier consumes every bit/sibling/proof node, recomputes all map/log roots, checks all authority/witness signatures, and enforces `pre_size=reservation_index`, `reservation_size=reservation_index+1`, `cas_size=retirement_index`, and `post_size=retirement_index+1`.

`registry_general_history_vector_v1` is exactly `{"checkpoints":[checkpoint_certificate_v1...],"map_keys":[hex64...],"proof":registry_publication_proof_v1,"transition_leaves":[closed_registry_leaf...],"version":"registry_general_history_vector_v1"}`; map keys sort lexicographically and transition leaves sort by canonical decimal index. Its normative instance has a signed charter genesis followed by four signed transition checkpoints: sizes `4`, `5`, `8`, and `9`; reservation index `4`; retirement index `8`; four non-target map entries before reservation; three unrelated appends between reservation and retirement; non-empty sparse paths; and RFC-6962 inclusion/consistency paths for `4->5`, `5->8`, and `8->9`. The generator emits every transition leaf, map key, checkpoint, proof path, and signature, so an independent implementation can recompute rather than trust listed roots. The `0/1` case, empty prior map, and empty proof arrays remain separately named toys only.

#### Candidate-specific final-envelope freeze

`final_envelope_freeze_v1` is exactly `{"candidate_id":hex64,"candidate_verdict_hash":hex64,"core_evidence_root":hex64,"envelope_byte_length":dec,"envelope_raw_sha256":hex64,"final_publication_envelope_hash":hex64,"freeze_id":hex64,"previous_freeze_hash":hex64,"registry_checkpoint_certificate_hash":hex64,"registry_post_root":hex64,"registry_proof_hash":hex64,"release_fingerprint":hex64,"version":"final_envelope_freeze_v1"}`. `candidate_id=H("cross-host-v2/publication-candidate-id-v1",raw32(fingerprint),raw32(core_root),raw32(candidate_verdict_hash),raw32(registry_checkpoint_certificate_hash))`; `freeze_id=H("cross-host-v2/final-envelope-freeze-id-v1",raw32(candidate_id),raw32(final_hash))`; its record hash is `H("cross-host-v2/final-envelope-freeze-v1",JCS(record))`.

The freeze ledger is hash-chained and create-only by `candidate_id`; its canonical bytes, unique candidate index, and singleton head commit in one `synchronous=FULL` SQLite transaction against the charter-pinned genesis/head. Before CAS, no guessed final hash exists. If CAS committed but its response was lost, recovery obtains the exact create-only receipt/proofs/certificate from the authority, verifies them against the immutable candidate, deterministically rebuilds and scans the envelope, and create-or-reads one freeze. Existing exact bytes win; same-candidate drift, multiple proofs/certificates, a stale previous freeze head, or unverifiable authority state is `authority_unresolved` / `INSUFFICIENT`. Visibility accepts only bytes rehashed against this freeze.

#### Closed visibility objects

The charter independently pins its authority namespace/genesis, pure Ed25519 key, `P = "cross-host-v2/visibility"`, witness history/quorum, and gated object namespace. Map key is `H(P+"/map-key-v1",UTF8(namespace),raw32(fingerprint),raw32(core_root))`. Absence has no value; the only value is exactly `{"core_evidence_root":hex64,"final_publication_envelope_hash":hex64,"object_set_root":hex64,"registry_post_root":hex64,"state":"visible","version":"visibility_map_value_v1"}`, hashed with `H(P+"/map-value-v1",JCS(value))`. `object_set_root=SHA256(ASCII("cross-host-v2/visibility-object-set-v1")||0x00||raw32(core_root)||raw32(final_hash))`. The log leaf is exactly `{"index":dec,"map_key":hex64,"map_value_hash":hex64,"object_set_root":hex64,"post_map_root":hex64,"prior_checkpoint_hash":hex64,"prior_map_root":hex64,"version":"visibility_log_leaf_v1"}`.

The receipt is exactly `{"core_evidence_root":hex64,"final_publication_envelope_hash":hex64,"fingerprint":hex64,"map_key":hex64,"namespace":id,"object_set_root":hex64,"post_checkpoint_hash":hex64,"post_map_root":hex64,"prior_checkpoint_hash":hex64,"prior_map_root":hex64,"proof_suite_version":"visibility_proof_suite_v1","registry_post_root":hex64,"version":"visibility_seal_receipt_v1"}`. Ed25519 signs `ASCII("cross-host-v2/visibility-seal-receipt-v1")||0x00||u64be(len(JCS(receipt)))||JCS(receipt)`. The suite is exactly `{"absence_path":transparency_path_v1,"log_consistency_path":[hex64...],"log_inclusion_path":[hex64...],"log_leaf":closed_leaf,"map_value":closed_value,"post_checkpoint":checkpoint_certificate_v1,"post_inclusion_path":transparency_path_v1,"prior_checkpoint":checkpoint_certificate_v1,"receipt":closed_receipt,"receipt_signature":hex128,"version":"visibility_proof_suite_v1"}` and hashes as `H(P+"/proof-suite-v1",JCS(suite))`.

The previous seed-`05` receipt has 988 JCS bytes and SHA-256 `fe56b08b33bf736d023ec354f2fc65faaae422a9f70477164f611f7a85ecd551`. Its correct signature is `437b433d295c00f5fe51cf5dde4abc93569cfede5e0474e951234a1129f2e2066fbff64c28cbe5eb7fdf6d9c8a5a4876d6217436823dd6d523e07be9c9d0e308`. The old `54b33f...`, its `0a0c05...` suite oracle, and the `e87fed...` completion-record oracle are invalid and MUST be rejected; receipt-only completion ID `4eee59...` and read root `0c7604...` do not rescue that suite. The replacement vector below freezes every downstream digest.

#### Executable production publication vector

`production_publication_vector_v2` is normative executable JavaScript for Node 20+. It uses only built-ins, writes to the explicit output directory, and emits 18 complete core members plus the envelope, registry history, visibility suite, freeze, completion, and oracle files. Its `exactKeys` tables plus nested validators are the closed executable vector schemas (`required` equals the listed keys and `additionalProperties=false`); implemented v2 JSON Schema must accept these bytes and reject every one-field deletion/addition. The generator also checks JCS identity, every content binding, sparse proof, and Ed25519 signature. Its exact source SHA-256 is `29ff8edea16eaee42b3eecdb4e7bb3e6bf2d00db08daa829f792f2db1726258c`. Copy the following block byte-for-byte to `/tmp/gh935-vector.mjs`, run `node --check /tmp/gh935-vector.mjs && node /tmp/gh935-vector.mjs /tmp/gh935-vector-out`, then scan `/tmp/gh935-vector-out`.

```js
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
 const ZERO = "0".repeat(64); const dec = n => String(n); const u64 = n => {   const b = Buffer.alloc(8);   b.writeBigUInt64BE(BigInt(n));   return b; }; const canon = x => {   if (Array.isArray(x)) return `[${x.map(canon).join(",")}]`;   if (x && typeof x === "object") {     return `{${Object.keys(x).sort().map(k => `${JSON.stringify(k)}:${canon(x[k])}`).join(",")}}`;   }   return JSON.stringify(x); }; const J = x => Buffer.from(canon(x)); const sha = b => crypto.createHash("sha256").update(b).digest(); const hex = b => Buffer.from(b).toString("hex"); const raw = s => Buffer.from(s, "hex"); const H = (tag, ...pieces) => sha(Buffer.concat([   Buffer.from(`${tag}\0`),   ...pieces.flatMap(p => [u64(p.length), p]), ])); const L = s => hex(sha(Buffer.from(`gh935-production-vector-v2/${s}`))); const eq = (a, b, label) => {   if (a !== b) throw new Error(`${label}: ${a} != ${b}`); }; const closed = (o, keys, label) => {   eq(Object.keys(o).sort().join(","), [...keys].sort().join(","), `${label} keys`); }; const keyFromSeed = byte => crypto.createPrivateKey({   key: Buffer.concat([Buffer.from("302e020100300506032b657004220420", "hex"), Buffer.alloc(32, byte)]),   format: "der",   type: "pkcs8", }); const signHash = (key, tag, digest) => hex(crypto.sign(   null,   Buffer.concat([Buffer.from(`${tag}\0`), raw(digest)]),   key, )); const signReceipt = (key, receipt) => {   const bytes = J(receipt);   return hex(crypto.sign(null, Buffer.concat([     Buffer.from("cross-host-v2/visibility-seal-receipt-v1\0"),     u64(bytes.length),     bytes,   ]), key)); }; const verifyHashSig = (key, tag, digest, sig) => {   const pub = crypto.createPublicKey(key);   if (!crypto.verify(null, Buffer.concat([Buffer.from(`${tag}\0`), raw(digest)]), pub, raw(sig))) {     throw new Error(`bad signature: ${tag}`);   } }; const verifyReceiptSig = (key, receipt, signature) => {   const bytes = J(receipt);   const message = Buffer.concat([     Buffer.from("cross-host-v2/visibility-seal-receipt-v1\0"),     u64(bytes.length),     bytes,   ]);   if (!crypto.verify(null, message, crypto.createPublicKey(key), raw(signature))) {     throw new Error("bad visibility receipt signature");   } };  const coreLeaf = (p, b) => sha(Buffer.concat([   Buffer.from("cross-host-v2/core-evidence-v1/leaf\0"),   u64(Buffer.byteLength(p)),   Buffer.from(p),   u64(b.length),   sha(b), ])); const coreRoot = members => {   let level = [...members].sort(([a], [b]) => Buffer.from(a).compare(Buffer.from(b)))     .map(([p, b]) => coreLeaf(p, b));   if (!level.length) throw new Error("empty core");   while (level.length > 1) {     const next = [];     for (let i = 0; i < level.length; i += 2) {       next.push(sha(Buffer.concat([         Buffer.from("cross-host-v2/core-evidence-v1/node\0"),         level[i],         level[i + 1] || level[i],       ])));     }     level = next;   }   return hex(level[0]); };  const smt = P => {   const empty = Array(257);   empty[256] = H(`${P}/smt-empty-leaf-v1`);   for (let d = 255; d >= 0; d--) empty[d] = H(`${P}/smt-node-v1`, empty[d + 1], empty[d + 1]);   const levels = entries => {     const out = Array.from({ length: 257 }, () => new Map());     for (const [k, valueHash] of entries) {       out[256].set(BigInt(`0x${k}`).toString(), H(`${P}/smt-leaf-v1`, raw(k), raw(valueHash)));     }     for (let d = 255; d >= 0; d--) {       const parents = new Set([...out[d + 1].keys()].map(k => (BigInt(k) >> 1n).toString()));       for (const p of parents) {         const n = BigInt(p);         out[d].set(p, H(           `${P}/smt-node-v1`,           out[d + 1].get((n << 1n).toString()) || empty[d + 1],           out[d + 1].get(((n << 1n) | 1n).toString()) || empty[d + 1],         ));       }     }     return out;   };   const root = entries => hex(levels(entries)[0].get("0") || empty[0]);   const proof = (entries, key) => {     const ls = levels(entries);     const k = BigInt(`0x${key}`);     const bitmap = Buffer.alloc(32);     const siblings = [];     for (let d = 0; d < 256; d++) {       const child = k >> BigInt(255 - d);       const sibling = ls[d + 1].get((child ^ 1n).toString()) || empty[d + 1];       if (!sibling.equals(empty[d + 1])) {         bitmap[Math.floor(d / 8)] |= 1 << (7 - (d % 8));         siblings.push(hex(sibling));       }     }     return { bitmap: hex(bitmap), siblings };   };   const verify = (rootHex, key, valueHash, p) => {     const byDepth = new Map();     let cursor = 0;     const bits = raw(p.bitmap);     for (let d = 0; d < 256; d++) {       if (bits[Math.floor(d / 8)] & (1 << (7 - (d % 8)))) byDepth.set(d, raw(p.siblings[cursor++]));     }     eq(cursor, p.siblings.length, "SMT sibling count");     let node = valueHash === null ? empty[256] : H(`${P}/smt-leaf-v1`, raw(key), raw(valueHash));     const k = BigInt(`0x${key}`);     for (let d = 255; d >= 0; d--) {       const sibling = byDepth.get(d) || empty[d + 1];       const bit = Number((k >> BigInt(255 - d)) & 1n);       node = bit ? H(`${P}/smt-node-v1`, sibling, node) : H(`${P}/smt-node-v1`, node, sibling);     }     eq(hex(node), rootHex, "SMT proof");   };   return { proof, root, verify }; };  const logFns = P => {   const leaf = o => H(`${P}/log-leaf-v1`, J(o));   const rootHashes = hs => {     if (!hs.length) return H(`${P}/log-empty-v1`);     if (hs.length === 1) return hs[0];     let k = 1;     while ((k << 1) < hs.length) k <<= 1;     return H(`${P}/log-node-v1`, rootHashes(hs.slice(0, k)), rootHashes(hs.slice(k)));   };   const root = os => hex(rootHashes(os.map(leaf)));   const inclusion = (i, hs) => {     if (hs.length === 1) return [];     let k = 1;     while ((k << 1) < hs.length) k <<= 1;     if (i < k) return [...inclusion(i, hs.slice(0, k)), hex(rootHashes(hs.slice(k)))];     return [...inclusion(i - k, hs.slice(k)), hex(rootHashes(hs.slice(0, k)))];   };   const consistencySub = (m, hs, complete) => {     if (m === hs.length) return complete ? [] : [hex(rootHashes(hs))];     if (m === 0) return [];     let k = 1;     while ((k << 1) < hs.length) k <<= 1;     if (m <= k) return [...consistencySub(m, hs.slice(0, k), complete), hex(rootHashes(hs.slice(k)))];     return [...consistencySub(m - k, hs.slice(k), false), hex(rootHashes(hs.slice(0, k)))];   };   return {     consistency: (m, os) => consistencySub(m, os.map(leaf), true),     inclusion: (i, os) => inclusion(i, os.map(leaf)),     leafHash: o => hex(leaf(o)),     root,   }; };  const flip = (key, depth) => {   const b = raw(key);   b[Math.floor(depth / 8)] ^= 1 << (7 - (depth % 8));   return hex(b); }; const makeCertificate = (P, namespace, bodyFields, previousHash, seeds) => {   const body = {     certificate_epoch: dec(bodyFields.epoch),     log_root: bodyFields.logRoot,     log_size: dec(bodyFields.logSize),     map_root: bodyFields.mapRoot,     namespace,     previous_checkpoint_hash: previousHash,     transition_index: bodyFields.transitionIndex === null ? null : dec(bodyFields.transitionIndex),     transition_leaf_hash: bodyFields.transitionLeafHash,     version: bodyFields.version,     witness_set_id: "epoch1",   };   const digest = hex(H(`${P}/checkpoint-certificate-v1`, J(body)));   const authority = keyFromSeed(seeds[0]);   const w1 = keyFromSeed(seeds[1]);   const w2 = keyFromSeed(seeds[2]);   const cert = {     authority_signature: signHash(authority, `${P}/checkpoint-certificate-authority-v1`, digest),     body,     witness_signatures: [{       signature: signHash(w1, `${P}/checkpoint-certificate-witness-v1`, digest),       witness_id: "w1",     }, {       signature: signHash(w2, `${P}/checkpoint-certificate-witness-v1`, digest),       witness_id: "w2",     }],   };   verifyHashSig(authority, `${P}/checkpoint-certificate-authority-v1`, digest, cert.authority_signature);   verifyHashSig(w1, `${P}/checkpoint-certificate-witness-v1`, digest, cert.witness_signatures[0].signature);   verifyHashSig(w2, `${P}/checkpoint-certificate-witness-v1`, digest, cert.witness_signatures[1].signature);   return { cert, hash: digest }; };  function buildMembers() {   const prompt = Buffer.from("system: benchmark\nmemory: none\ntask: vector\n");   const promptParts = [     ["system", "common", Buffer.from("system: benchmark\n")],     ["condition_surface", "no_memory", Buffer.from("memory: none\n")],     ["task", "common", Buffer.from("task: vector\n")],   ];   let offset = 0;   const promptManifest = {     segments: promptParts.map(([role, owner, bytes]) => {       const item = {         byte_length: dec(bytes.length),         channel: "initial",         offset: dec(offset),         owner,         role,         sha256: hex(sha(bytes)),       };       offset += bytes.length;       return item;     }),     stream_byte_length: dec(prompt.length),     stream_sha256: hex(sha(prompt)),     version: "prompt_surface_manifest_v1",   };   const schemaContract = {     canonical_json: "canonical_json_rfc8785_v1",     closed_objects: [       "canonical_plan_v2", "candidate_verdict_v1", "causal_oracle_v1",       "cross_host_report_v2", "cross_host_run_v2", "cross_host_source_attempt_v2",       "cross_host_task_v2", "evidence_manifest_v2", "maintained_export_v1",       "native_import_v1", "pre_run_scorer_commitment_v1",       "prompt_surface_manifest_v1", "public_scoring_projection_v1",       "scoring_input_freeze_v1",     ],     schema_revision: L("schema-revision"),     validator_revision: L("closed-validator"),     version: "schema_bundle_v2",   };   const task = {     allowed_paths: ["src/vector.rs"],     category: "architecture_decision",     causal_oracles: [{       artifact_matcher_hash: L("matcher"),       memory_selector_hash: L("memory-selector"),       proof_method: "pre_action_use_v1",       rule_id: "wrong-vector-action",       wrong_action_class: "wrong_architecture_action",     }],     direction: "claude_to_codex",     fixture: {       canonical_workspace_recipe_hash: L("workspace-recipe"),       git_revision: L("fixture-revision"),       repository_hash: L("fixture-repository"),     },     foreign_project_decoy: {       canary_hash: L("foreign-canary"),       canonical_path_hash: L("decoy-path"),       fixture_hash: L("decoy-fixture"),       project_identity: L("decoy-project"),     },     forbidden_paths: ["hidden/vector_test.rs"],     gold_fact_hashes: [L("gold-fact")],     hidden_tests: [{       path: "hidden/vector_test.rs",       sha256: L("hidden-test"),     }],     host_requirements: [{       executable_hash: L("claude-executable"),       host: "claude_code",       profile_hash: L("claude-profile"),     }, {       executable_hash: L("codex-executable"),       host: "codex",       profile_hash: L("codex-profile"),     }],     native_snapshot_policy_hash: L("native-policy"),     pre_run_scorer_commitment_hash: L("scorer-commitment-slot"),     score_commands: [["cargo", "test", "--test", "vector_hidden"]],     source_episodes: [{       episode_index: "0",       expected_evidence_hash: L("episode-0"),     }, {       episode_index: "1",       expected_evidence_hash: L("episode-1"),     }],     stale_challenges: [{       expected_pre_filter_match: true,       state: "superseded",       state_hash: L("stale-state"),     }],     status: "ready",     source_host: "claude_code",     target_host: "codex",     task_id: "publication-vector",     todo: [],     version: "cross_host_task_v2",   };   const plan = {     conditions: ["no_memory", "target_host_native", "exported_file", "remem_shared"],     diagnostic_conditions: ["remem_without_host_native_import", "remem_with_host_native_import"],     diagnostic_tuple_count: "144",     plan_hash: L("plan"),     primary_tuple_count: "288",     runs_per_task_condition: "3",     schedule_hash: L("schedule"),     task_set_hash: L("task-set"),     version: "canonical_plan_v2",   };   const sourceAttempt = {     cleanup: {       leak_scan_hash: L("source-leak-scan"),       secret_destruction_status: "passed",       status: "passed",     },     cleanup_status: "passed",     condition_neutral_evidence_hash: L("neutral-evidence"),     cost_ledger_hash: L("source-cost"),     direction: "claude_to_codex",     ended_at: "2030-01-01T00:00:01Z",     input_hashes: {       executable_hash: L("claude-executable"),       fixture_hash: L("fixture-revision"),       plan_hash: plan.plan_hash,       profile_hash: L("claude-profile"),       task_hash: hex(sha(J(task))),     },     preparation_absences: [{       reason: "vector_no_live_run",       stage: "target_projection",     }],     run_index: "0",     scanner_status: "passed",     sealed_at: "2030-01-01T00:00:00Z",     source_attempt_id: "source-vector-1",     source_seal_hash: L("source-seal"),     started_at: "2030-01-01T00:00:00Z",     task_id: "publication-vector",     terminal_reason: "vector_no_live_run",     version: "cross_host_source_attempt_v2",   };   const scorer = {     argv: ["vector-scorer", "--offline"],     assertion_revision: L("assertion-revision"),     deriver_hash: L("oracle-deriver"),     engine_hash: L("scorer-engine"),     environment_allowlist: ["LANG", "PATH"],     hidden_input_root: L("hidden-input"),     oracle_derivation: {       mode: "deterministic_derivation_v1",       output_slot_hash: L("oracle-output-slot"),       static_input_root: L("oracle-static-input"),     },     oracle_hash: L("oracle"),     output_framing_hash: L("scorer-output-framing"),     read_contract_hash: L("read-contract"),     release_fingerprint: "11".repeat(32),     release_revision: "vector-revision",     runtime_hash: L("scorer-runtime"),     sanitizer_hash: L("oracle-sanitizer"),     scoring_ir_hash: L("scoring-ir"),     toolchain_hash: L("scorer-toolchain"),     version: "pre_run_scorer_commitment_v1",     wrapper_hash: L("scorer-wrapper"),   };   const freeze = {     complete_host_output_manifest_root: L("host-output-manifest"),     exclusions_hash: L("freeze-exclusions"),     host_output_byte_length: "0",     host_output_hash: hex(sha(Buffer.alloc(0))),     manifest_root: L("freeze-manifest"),     scanner_policy_hash: L("freeze-scanner"),     writers_closed: true,     workspace_archive_hash: L("workspace-archive"),     workspace_byte_length: "4096",     workspace_manifest_root: L("workspace-manifest"),     workspace_root: L("workspace-root"),     version: "scoring_input_freeze_v1",   };   const scoringProjection = {     actual_read_set_hash: L("read-set"),     byte_length: "512",     freeze_root: freeze.workspace_root,     inclusion_proof_root: L("inclusion-proofs"),     manifest_root: L("projection-manifest"),     oracle_result_hash: L("oracle-result"),     projection_root: L("scoring-projection"),     scorer_result_hash: L("scorer-result"),     version: "public_scoring_projection_v1",   };   const maintainedExport = {     boundary_state: "not_applicable",     budget: {       cumulative_cost_microusd: "0",       host_calls: "0",       llm_calls: "0",       output_bytes: "0",       tokens: "0",       turns: "0",       wall_time_ms: "0",     },     budget_status: "within_cap",     cycle_records: [],     cycle_count: "0",     evidence_projection_hash: L("export-evidence-projection"),     exporter_hash: L("exporter"),     final_envelope_hash: null,     protocol_hash: L("export-protocol"),     reason: "condition_not_exported_file",     version: "maintained_export_v1",   };   const nativeImport = {     authorized_inserted_count: "0",     foreign_canary_excluded_count: "0",     import_attempt_id: null,     lineage_root: null,     neutral_base_hash: L("neutral-base"),     neutral_counts: {       active_memory_count: "0",       candidate_count: "0",       provenance_count: "0",     },     reason: "condition_not_native_diagnostic",     status: "not_applicable",     version: "native_import_v1",   };   const causal = {     first_wrong_action_event_id: null,     intervention_hash: null,     memory_tuple: null,     method_hash: L("pre-action-method"),     reason: "no_memory_surface",     replay_hashes: [],     rule_id: "wrong-vector-action",     status: "not_proven",     use_event_id: null,     version: "causal_oracle_v1",   };   const bootstrap = {     algorithm_hash: L("bootstrap-algorithm"),     cluster_count: "12",     confidence_level: "0.95",     draw_count: "100000",     interval: null,     quantile_rank_lower: "2500",     quantile_rank_upper: "97500",     regression_rank_upper: "98750",     seed: L("bootstrap-seed"),     status: "not_run",     version: "bootstrap_evidence_v1",   };   const members = new Map();   const addJson = (p, o) => members.set(p, J(o));   addJson("inputs/schema_bundle_v2.json", schemaContract);   addJson("inputs/task_v2.json", task);   addJson("plans/canonical_plan_v2.json", plan);   addJson("attempts/source_attempt_v2.json", sourceAttempt);   members.set("prompts/host_channel_prompt_stream_v1.bin", prompt);   addJson("prompts/prompt_surface_manifest_v1.json", promptManifest);   addJson("scoring/pre_run_scorer_commitment_v1.json", scorer);   addJson("scoring/scoring_input_freeze_v1.json", freeze);   addJson("scoring/public_scoring_projection_v1.json", scoringProjection);   addJson("evidence/maintained_export_v1.json", maintainedExport);   addJson("evidence/native_import_v1.json", nativeImport);   addJson("evidence/causal_oracle_v1.json", causal);   addJson("evidence/bootstrap_evidence_v1.json", bootstrap);   const D = p => hex(sha(members.get(p)));   const run = {     attempt_evidence: {       journal_root: L("target-journal"),       m_count: "0",       n_count: "1",       no_write_artifact_hash: L("no-write-artifact"),       r_count: "0",       reveal_state: "proven_no_write",       selected_claim_attempt: "target-vector-1",       t_count: "1",       terminal_seal_hash: L("target-terminal-seal"),     },     attribution: {       absent_due_to: "condition_no_memory",       production_stage_refs: [],     },     causal_oracle_hashes: [D("evidence/causal_oracle_v1.json")],     cleanup: {       leak_scan_hash: L("target-leak-scan"),       secret_destruction_status: "passed",       status: "passed",     },     condition: "no_memory",     condition_records: {       maintained_export_hash: D("evidence/maintained_export_v1.json"),       native_import_hash: D("evidence/native_import_v1.json"),     },     cost: {       estimated_cost_microusd: "0",       host_calls: "0",       llm_calls: "0",       tokens: "0",       turns: "0",       wall_time_ms: "0",     },     direction: "claude_to_codex",     ended_at: "2030-01-01T00:00:02Z",     executable_hash: L("codex-executable"),     plan_hash: D("plans/canonical_plan_v2.json"),     planned_position: "0",     profile_hash: L("codex-profile"),     projection: {       absent_due_to: "condition_no_memory",       ciphertext_hash: null,       logical_hash: null,       projection_key_id: null,     },     prompt: {       adapter_version: "vector_adapter_v1",       condition_surface_hash: hex(sha(Buffer.from("memory: none\n"))),       condition_surface_length: "13",       manifest_hash: D("prompts/prompt_surface_manifest_v1.json"),       rolling_sent_hash: hex(sha(Buffer.alloc(0))),       rolling_sent_length: "0",       stream_hash: D("prompts/host_channel_prompt_stream_v1.bin"),       stream_length: "44",       task_segment_hash: hex(sha(Buffer.from("task: vector\n"))),     },     realized_position: null,     resolved: false,     run_index: "0",     scoring: {       freeze_hash: D("scoring/scoring_input_freeze_v1.json"),       oracle_result_hash: L("oracle-result"),       release_fingerprint: scorer.release_fingerprint,       release_revision: scorer.release_revision,       scorer_commitment_hash: D("scoring/pre_run_scorer_commitment_v1.json"),       scorer_result_hash: L("scorer-result"),       scoring_projection_hash: D("scoring/public_scoring_projection_v1.json"),     },     secondary_metrics: {       tokens: null,       turns: null,       wall_time_ms: null,     },     source_attempt_hash: D("attempts/source_attempt_v2.json"),     source_attempt_id: "source-vector-1",     source_seal_hash: sourceAttempt.source_seal_hash,     started_at: "2030-01-01T00:00:02Z",     status: "ordinary_failure",     target_attempt_id: "target-vector-1",     target_identity: {       config_hash: L("target-config"),       home_hash: L("target-home"),       session_hash: L("target-session"),       workspace_hash: L("target-workspace"),     },     task_hash: D("inputs/task_v2.json"),     task_id: "publication-vector",     terminal_reason: "vector_no_live_run",     tuple_key: "claude_to_codex/publication-vector/0/no_memory",     version: "cross_host_run_v2",   };   addJson("runs/run_v2.json", run);   const manifest = {     attempt_evidence: [{       journal_root: run.attempt_evidence.journal_root,       m_count: "0",       n_count: "1",       r_count: "0",       resolved: false,       reveal_state: "proven_no_write",       selected_claim_attempt: "target-vector-1",       t_count: "1",       target_attempt_id: "target-vector-1",       terminal_seal_hash: run.attempt_evidence.terminal_seal_hash,     }],     attempt_policy_hash: L("attempt-policy"),     bootstrap_hash: D("evidence/bootstrap_evidence_v1.json"),     causal_hash: D("evidence/causal_oracle_v1.json"),     component_roots: {       freeze: D("scoring/scoring_input_freeze_v1.json"),       prompt: D("prompts/host_channel_prompt_stream_v1.bin"),       reservation: L("reservation-root"),       run: D("runs/run_v2.json"),       scorer: D("scoring/pre_run_scorer_commitment_v1.json"),     },     export_hash: D("evidence/maintained_export_v1.json"),     failed_tuple_count: "1",     freeze_root: freeze.workspace_root,     kind: "partial_non_security",     missing_tuple_count: "432",     missing_tuple_set_hash: L("missing-tuples"),     native_hash: D("evidence/native_import_v1.json"),     not_started_tuple_count: "431",     not_started_tuple_set_hash: L("not-started-tuples"),     plan_hash: D("plans/canonical_plan_v2.json"),     planned_tuple_count: "432",     planned_tuple_set_hash: L("planned-tuples"),     prompt_manifest_root: D("prompts/prompt_surface_manifest_v1.json"),     prompt_root: D("prompts/host_channel_prompt_stream_v1.bin"),     reason_codes: ["vector_no_live_run"],     recorded_tuple_count: "1",     registry: {       genesis_root: L("registry-genesis-root"),       namespace: "registry.test",       prior_root: L("registry-prior-root"),       reservation_receipt_hash: L("reservation-receipt"),       reservation_root: L("reservation-root"),     },     registry_reservation_root: L("reservation-root"),     release_fingerprint: scorer.release_fingerprint,     release_revision: scorer.release_revision,     run_hashes: [D("runs/run_v2.json")],     sanitized_run_record_root: D("runs/run_v2.json"),     scorer_commitment_hash: D("scoring/pre_run_scorer_commitment_v1.json"),     selected_tuple_count: "1",     source_attempt_hashes: [D("attempts/source_attempt_v2.json")],     version: "evidence_manifest_v2",   };   addJson("evidence/evidence_manifest_v2.json", manifest);   const report = {     bootstrap_hash: D("evidence/bootstrap_evidence_v1.json"),     causal_input_root: D("evidence/causal_oracle_v1.json"),     diagnostic_tables: [],     direction_tables: [{       direction: "claude_to_codex",       failed: "1",       missing: "215",       planned: "216",       recorded: "1",       selected: "1",     }, {       direction: "codex_to_claude",       failed: "0",       missing: "216",       planned: "216",       recorded: "0",       selected: "0",     }],     evidence_manifest_hash: D("evidence/evidence_manifest_v2.json"),     export_cost_root: D("evidence/maintained_export_v1.json"),     failed_tuple_count: "1",     frozen_result_root: L("frozen-result-root"),     metric_rows: [{       applicability: "not_applicable_partial",       denominator: null,       metric_id: "resolved_rate",       missing_reason: "vector_no_live_run",       numerator: null,     }],     missing_tuple_count: "432",     native_input_root: D("evidence/native_import_v1.json"),     not_started_tuple_count: "431",     planned_tuple_count: "432",     prompt_root: D("prompts/host_channel_prompt_stream_v1.bin"),     recorded_tuple_count: "1",     registry_roots: {       genesis: manifest.registry.genesis_root,       prior: manifest.registry.prior_root,       reservation: manifest.registry.reservation_root,     },     release_fingerprint: scorer.release_fingerprint,     release_revision: scorer.release_revision,     run_record_root: D("runs/run_v2.json"),     scorer_commitment_hash: D("scoring/pre_run_scorer_commitment_v1.json"),     selected_tuple_count: "1",     selection_input_root: L("selection-input-root"),     stop_loss_rows: [],     verdict: "INSUFFICIENT",     version: "cross_host_report_v2",   };   addJson("reports/report_v2.json", report);   const markdown = Buffer.from(     `# Cross-host vector\n\nVerdict: INSUFFICIENT\nReport SHA-256: ${D("reports/report_v2.json")}\n`,   );   members.set("reports/report_v2.md", markdown);   const candidate = {     evidence_manifest_hash: D("evidence/evidence_manifest_v2.json"),     freeze_root: freeze.workspace_root,     markdown_hash: D("reports/report_v2.md"),     prompt_root: D("prompts/host_channel_prompt_stream_v1.bin"),     registry_reservation_root: manifest.registry_reservation_root,     renderer_hash: L("renderer"),     renderer_version: "vector_renderer_v1",     report_hash: D("reports/report_v2.json"),     run_record_root: D("runs/run_v2.json"),     scorer_commitment_hash: D("scoring/pre_run_scorer_commitment_v1.json"),     verdict: "INSUFFICIENT",     version: "candidate_verdict_v1",   };   addJson("candidate_verdict_v1.json", candidate);   const exactKeys = {     "inputs/schema_bundle_v2.json": [       "canonical_json", "closed_objects", "schema_revision", "validator_revision", "version",     ],     "inputs/task_v2.json": [       "allowed_paths", "category", "causal_oracles", "direction", "fixture",       "foreign_project_decoy", "forbidden_paths", "gold_fact_hashes", "hidden_tests",       "host_requirements", "native_snapshot_policy_hash", "pre_run_scorer_commitment_hash",       "score_commands", "source_episodes", "source_host", "stale_challenges", "status",       "target_host", "task_id", "todo", "version",     ],     "plans/canonical_plan_v2.json": [       "conditions", "diagnostic_conditions", "diagnostic_tuple_count", "plan_hash",       "primary_tuple_count", "runs_per_task_condition", "schedule_hash", "task_set_hash",       "version",     ],     "attempts/source_attempt_v2.json": [       "cleanup", "cleanup_status", "condition_neutral_evidence_hash", "cost_ledger_hash",       "direction", "ended_at", "input_hashes", "preparation_absences", "run_index",       "scanner_status", "sealed_at", "source_attempt_id", "source_seal_hash", "started_at",       "task_id", "terminal_reason", "version",     ],     "prompts/prompt_surface_manifest_v1.json": [       "segments", "stream_byte_length", "stream_sha256", "version",     ],     "scoring/pre_run_scorer_commitment_v1.json": [       "argv", "assertion_revision", "deriver_hash", "engine_hash", "environment_allowlist",       "hidden_input_root", "oracle_derivation", "oracle_hash", "output_framing_hash",       "read_contract_hash", "release_fingerprint", "release_revision", "runtime_hash",       "sanitizer_hash", "scoring_ir_hash", "toolchain_hash", "version", "wrapper_hash",     ],     "scoring/scoring_input_freeze_v1.json": [       "complete_host_output_manifest_root", "exclusions_hash", "host_output_byte_length",       "host_output_hash", "manifest_root", "scanner_policy_hash", "workspace_archive_hash",       "workspace_byte_length", "workspace_manifest_root", "workspace_root", "writers_closed",       "version",     ],     "scoring/public_scoring_projection_v1.json": [       "actual_read_set_hash", "byte_length", "freeze_root", "inclusion_proof_root",       "manifest_root", "oracle_result_hash", "projection_root", "scorer_result_hash", "version",     ],     "evidence/maintained_export_v1.json": [       "boundary_state", "budget", "budget_status", "cycle_count", "cycle_records",       "evidence_projection_hash", "exporter_hash", "final_envelope_hash", "protocol_hash",       "reason", "version",     ],     "evidence/native_import_v1.json": [       "authorized_inserted_count", "foreign_canary_excluded_count", "import_attempt_id",       "lineage_root", "neutral_base_hash", "neutral_counts", "reason", "status", "version",     ],     "evidence/causal_oracle_v1.json": [       "first_wrong_action_event_id", "intervention_hash", "memory_tuple", "method_hash",       "reason", "replay_hashes", "rule_id", "status", "use_event_id", "version",     ],     "evidence/bootstrap_evidence_v1.json": [       "algorithm_hash", "cluster_count", "confidence_level", "draw_count", "interval",       "quantile_rank_lower", "quantile_rank_upper", "regression_rank_upper", "seed",       "status", "version",     ],     "runs/run_v2.json": [       "attempt_evidence", "attribution", "causal_oracle_hashes", "cleanup", "condition",       "condition_records", "cost", "direction", "ended_at", "executable_hash", "plan_hash",       "planned_position", "profile_hash", "projection", "prompt", "realized_position",       "resolved", "run_index", "scoring", "secondary_metrics", "source_attempt_hash",       "source_attempt_id", "source_seal_hash", "started_at", "status", "target_attempt_id",       "target_identity", "task_hash", "task_id", "terminal_reason", "tuple_key", "version",     ],     "evidence/evidence_manifest_v2.json": [       "attempt_evidence", "attempt_policy_hash", "bootstrap_hash", "causal_hash",       "component_roots", "export_hash", "failed_tuple_count", "freeze_root", "kind",       "missing_tuple_count", "missing_tuple_set_hash", "native_hash",       "not_started_tuple_count", "not_started_tuple_set_hash", "plan_hash",       "planned_tuple_count", "planned_tuple_set_hash", "prompt_manifest_root", "prompt_root",       "reason_codes", "recorded_tuple_count", "registry", "registry_reservation_root",       "release_fingerprint", "release_revision", "run_hashes", "sanitized_run_record_root",       "scorer_commitment_hash", "selected_tuple_count", "source_attempt_hashes", "version",     ],     "reports/report_v2.json": [       "bootstrap_hash", "causal_input_root", "diagnostic_tables", "direction_tables",       "evidence_manifest_hash", "export_cost_root", "failed_tuple_count",       "frozen_result_root", "metric_rows", "missing_tuple_count", "native_input_root",       "not_started_tuple_count", "planned_tuple_count", "prompt_root", "recorded_tuple_count",       "registry_roots", "release_fingerprint", "release_revision", "run_record_root",       "scorer_commitment_hash", "selected_tuple_count", "selection_input_root",       "stop_loss_rows", "verdict", "version",     ],     "candidate_verdict_v1.json": [       "evidence_manifest_hash", "freeze_root", "markdown_hash", "prompt_root",       "registry_reservation_root", "renderer_hash", "renderer_version", "report_hash",       "run_record_root", "scorer_commitment_hash", "verdict", "version",     ],   };   for (const [p, keys] of Object.entries(exactKeys)) {     const parsed = JSON.parse(members.get(p));     closed(parsed, keys, p);     eq(J(parsed).equals(members.get(p)), true, `${p} JCS`);   }   closed(task.fixture, ["canonical_workspace_recipe_hash", "git_revision", "repository_hash"], "task fixture");   closed(task.foreign_project_decoy, ["canary_hash", "canonical_path_hash", "fixture_hash", "project_identity"], "task decoy");   closed(task.causal_oracles[0], ["artifact_matcher_hash", "memory_selector_hash", "proof_method", "rule_id", "wrong_action_class"], "task oracle");   for (const requirement of task.host_requirements) {     closed(requirement, ["executable_hash", "host", "profile_hash"], "task host requirement");   }   closed(task.hidden_tests[0], ["path", "sha256"], "task hidden test");   closed(task.source_episodes[0], ["episode_index", "expected_evidence_hash"], "task episode");   closed(task.stale_challenges[0], ["expected_pre_filter_match", "state", "state_hash"], "task stale challenge");   closed(sourceAttempt.cleanup, ["leak_scan_hash", "secret_destruction_status", "status"], "source cleanup");   closed(sourceAttempt.input_hashes, ["executable_hash", "fixture_hash", "plan_hash", "profile_hash", "task_hash"], "source inputs");   closed(scorer.oracle_derivation, ["mode", "output_slot_hash", "static_input_root"], "oracle derivation");   closed(maintainedExport.budget, ["cumulative_cost_microusd", "host_calls", "llm_calls", "output_bytes", "tokens", "turns", "wall_time_ms"], "export budget");   closed(nativeImport.neutral_counts, ["active_memory_count", "candidate_count", "provenance_count"], "native neutral counts");   closed(run.attempt_evidence, ["journal_root", "m_count", "n_count", "no_write_artifact_hash", "r_count", "reveal_state", "selected_claim_attempt", "t_count", "terminal_seal_hash"], "run attempt evidence");   closed(run.attribution, ["absent_due_to", "production_stage_refs"], "run attribution");   closed(run.cleanup, ["leak_scan_hash", "secret_destruction_status", "status"], "run cleanup");   closed(run.condition_records, ["maintained_export_hash", "native_import_hash"], "run condition records");   closed(run.cost, ["estimated_cost_microusd", "host_calls", "llm_calls", "tokens", "turns", "wall_time_ms"], "run cost");   closed(run.projection, ["absent_due_to", "ciphertext_hash", "logical_hash", "projection_key_id"], "run projection");   closed(run.prompt, ["adapter_version", "condition_surface_hash", "condition_surface_length", "manifest_hash", "rolling_sent_hash", "rolling_sent_length", "stream_hash", "stream_length", "task_segment_hash"], "run prompt");   closed(run.scoring, ["freeze_hash", "oracle_result_hash", "release_fingerprint", "release_revision", "scorer_commitment_hash", "scorer_result_hash", "scoring_projection_hash"], "run scoring");   closed(run.secondary_metrics, ["tokens", "turns", "wall_time_ms"], "run metrics");   closed(run.target_identity, ["config_hash", "home_hash", "session_hash", "workspace_hash"], "target identity");   closed(manifest.component_roots, ["freeze", "prompt", "reservation", "run", "scorer"], "manifest components");   closed(manifest.registry, ["genesis_root", "namespace", "prior_root", "reservation_receipt_hash", "reservation_root"], "manifest registry");   closed(report.registry_roots, ["genesis", "prior", "reservation"], "report registry");   for (const segment of promptManifest.segments) {     closed(segment, ["byte_length", "channel", "offset", "owner", "role", "sha256"], "prompt segment");   }   eq(task.status, "ready", "task ready");   eq(task.todo.length, 0, "task todo");   eq(task.source_episodes.length >= 2, true, "task episodes");   eq(task.score_commands.length > 0 && task.hidden_tests.length > 0, true, "task scorer inputs");   eq(promptManifest.stream_sha256, D("prompts/host_channel_prompt_stream_v1.bin"), "prompt hash");   eq(promptManifest.segments.reduce((n, x) => n + Number(x.byte_length), 0), prompt.length, "prompt partition");   eq(report.evidence_manifest_hash, D("evidence/evidence_manifest_v2.json"), "report manifest");   eq(candidate.report_hash, D("reports/report_v2.json"), "candidate report");   eq(candidate.markdown_hash, D("reports/report_v2.md"), "candidate markdown");   return {     candidate,     core: coreRoot([...members]),     D,     members,     schemaContract,   }; }  const OUT = process.argv[2] || "/tmp/gh935-production-vector-v2"; if (!/^\/(?:private\/)?tmp\/gh935-[A-Za-z0-9._-]+$/.test(OUT) || fs.existsSync(OUT)) {   throw new Error("output must be a new /tmp/gh935-* directory"); } const write = (p, b) => {   const dst = path.join(OUT, p);   fs.mkdirSync(path.dirname(dst), { recursive: true });   fs.writeFileSync(dst, b); }; const { candidate, core, D, members } = buildMembers();  const PREG = "cross-host-v2/registry"; const NSREG = "registry.test"; const fingerprint = "11".repeat(32); const regMapKey = hex(H(`${PREG}/map-key-v1`, Buffer.from(NSREG), raw(fingerprint))); const regSmt = smt(PREG); const regLog = logFns(PREG); let regEntries = new Map(); const regLeaves = []; const unrelatedKeys = [0, 7, 31, 79, 143, 211, 247].map(d => flip(regMapKey, d)); for (let i = 0; i < 4; i++) {   const priorMapRoot = regSmt.root(regEntries);   regEntries.set(unrelatedKeys[i], L(`registry-unrelated-value-${i}`));   regLeaves.push({     index: dec(i),     map_key: unrelatedKeys[i],     post_map_root: regSmt.root(regEntries),     prior_map_root: priorMapRoot,     transition_id: L(`registry-unrelated-transition-${i}`),     version: "registry_unrelated_log_leaf_v1",   }); } const reservedState = {   core_evidence_root: null,   fingerprint,   state: "reserved",   version: "registry_state_v1", }; const reservedHash = hex(H(`${PREG}/state-v1`, J(reservedState))); const preEntries = new Map(regEntries); const preMapRoot = regSmt.root(preEntries); regEntries.set(regMapKey, reservedHash); const reservationMapRoot = regSmt.root(regEntries); const reservationLeaf = {   fingerprint,   index: "4",   map_key: regMapKey,   post_map_root: reservationMapRoot,   post_state_hash: reservedHash,   prior_map_root: preMapRoot,   version: "registry_reservation_log_leaf_v1", }; regLeaves.push(reservationLeaf); const reservationEntries = new Map(regEntries); for (let i = 4; i < 7; i++) {   const priorMapRoot = regSmt.root(regEntries);   regEntries.set(unrelatedKeys[i], L(`registry-unrelated-value-${i}`));   regLeaves.push({     index: dec(i + 1),     map_key: unrelatedKeys[i],     post_map_root: regSmt.root(regEntries),     prior_map_root: priorMapRoot,     transition_id: L(`registry-unrelated-transition-${i}`),     version: "registry_unrelated_log_leaf_v1",   }); } const casEntries = new Map(regEntries); const casMapRoot = regSmt.root(casEntries); const retiredState = {   core_evidence_root: core,   fingerprint,   state: "retired",   version: "registry_state_v1", }; const retiredHash = hex(H(`${PREG}/state-v1`, J(retiredState))); const request = {   core_evidence_root: core,   expected_prior_map_root: casMapRoot,   fingerprint,   version: "publish_and_retire_request_v1", }; const requestHash = hex(H(`${PREG}/cas-request-v1`, J(request))); regEntries.set(regMapKey, retiredHash); const postMapRoot = regSmt.root(regEntries); const regCert = (epoch, leaves, mapRoot, transitionIndex, previousHash) => makeCertificate(   PREG,   NSREG,   {     epoch,     logRoot: regLog.root(leaves),     logSize: leaves.length,     mapRoot,     transitionIndex,     transitionLeafHash: transitionIndex === null ? null : regLog.leafHash(leaves[transitionIndex]),     version: "registry_checkpoint_certificate_v1",   },   previousHash,   [1, 2, 3], ); const genesis = regCert(0, [], regSmt.root(new Map()), null, ZERO); const preCert = regCert(1, regLeaves.slice(0, 4), preMapRoot, 3, genesis.hash); const reservationCert = regCert(   2,   regLeaves.slice(0, 5),   reservationMapRoot,   4,   preCert.hash, ); const casCert = regCert(3, regLeaves.slice(0, 8), casMapRoot, 7, reservationCert.hash); const retirementLeaf = {   cas_request_hash: requestHash,   core_evidence_root: core,   fingerprint,   index: "8",   map_key: regMapKey,   post_map_root: postMapRoot,   post_state_hash: retiredHash,   prior_checkpoint_hash: casCert.hash,   prior_map_root: casMapRoot,   prior_state_hash: reservedHash,   version: "registry_retirement_log_leaf_v1", }; regLeaves.push(retirementLeaf); const postCert = regCert(4, regLeaves, postMapRoot, 8, casCert.hash); const receipt = {   cas_request_hash: requestHash,   checkpoint_certificate_hash: postCert.hash,   post_log_root: postCert.cert.body.log_root,   post_map_root: postMapRoot,   retirement_log_index: "8",   version: "publish_and_retire_receipt_v1", }; const receiptHash = hex(H(`${PREG}/cas-receipt-v1`, J(receipt))); const registryProof = {   cas_prior_checkpoint: casCert.cert,   cas_prior_state_path: regSmt.proof(casEntries, regMapKey),   cas_receipt: receipt,   cas_receipt_hash: receiptHash,   cas_request: request,   cas_request_hash: requestHash,   cas_to_post_consistency_path: regLog.consistency(8, regLeaves),   genesis_checkpoint: genesis.cert,   genesis_to_pre_consistency_path: regLog.consistency(0, regLeaves.slice(0, 4)),   namespace: NSREG,   post_checkpoint: postCert.cert,   pre_reservation_absence_path: regSmt.proof(preEntries, regMapKey),   pre_reservation_checkpoint: preCert.cert,   pre_to_reservation_consistency_path: regLog.consistency(4, regLeaves.slice(0, 5)),   reservation_at_cas_inclusion_path: regLog.inclusion(4, regLeaves.slice(0, 8)),   reservation_checkpoint: reservationCert.cert,   reservation_log_inclusion_path: regLog.inclusion(4, regLeaves.slice(0, 5)),   reservation_log_leaf: reservationLeaf,   reservation_state: reservedState,   reservation_state_path: regSmt.proof(reservationEntries, regMapKey),   reservation_to_cas_consistency_path: regLog.consistency(5, regLeaves.slice(0, 8)),   retired_state: retiredState,   retired_state_path: regSmt.proof(regEntries, regMapKey),   retirement_log_inclusion_path: regLog.inclusion(8, regLeaves),   retirement_log_leaf: retirementLeaf,   version: "registry_publication_proof_v1", }; regSmt.verify(preMapRoot, regMapKey, null, registryProof.pre_reservation_absence_path); regSmt.verify(reservationMapRoot, regMapKey, reservedHash, registryProof.reservation_state_path); regSmt.verify(casMapRoot, regMapKey, reservedHash, registryProof.cas_prior_state_path); regSmt.verify(postMapRoot, regMapKey, retiredHash, registryProof.retired_state_path);  const memberList = [...members]   .sort(([a], [b]) => Buffer.from(a).compare(Buffer.from(b)))   .map(([p, b]) => ({ byte_length: dec(b.length), path: p, sha256: hex(sha(b)) })); const envelope = {   candidate_verdict: candidate.verdict,   candidate_verdict_hash: D("candidate_verdict_v1.json"),   core_evidence_root: core,   core_members: memberList,   registry_checkpoint: postCert.cert,   registry_checkpoint_hash: postCert.hash,   registry_post_root: postMapRoot,   registry_proof: registryProof,   release_fingerprint: fingerprint,   version: "final_publication_envelope_v1", }; const envelopeBytes = J(envelope); const finalHash = hex(sha(Buffer.concat([   Buffer.from("cross-host-v2/final-publication-envelope-v1\0"),   u64(envelopeBytes.length),   envelopeBytes, ]))); const candidateId = hex(H(   "cross-host-v2/publication-candidate-id-v1",   raw(fingerprint),   raw(core),   raw(D("candidate_verdict_v1.json")),   raw(postCert.hash), )); const freezeId = hex(H(   "cross-host-v2/final-envelope-freeze-id-v1",   raw(candidateId),   raw(finalHash), )); const envelopeFreeze = {   candidate_id: candidateId,   candidate_verdict_hash: D("candidate_verdict_v1.json"),   core_evidence_root: core,   envelope_byte_length: dec(envelopeBytes.length),   envelope_raw_sha256: hex(sha(envelopeBytes)),   final_publication_envelope_hash: finalHash,   freeze_id: freezeId,   previous_freeze_hash: L("previous-freeze"),   registry_checkpoint_certificate_hash: postCert.hash,   registry_post_root: postMapRoot,   registry_proof_hash: hex(sha(J(registryProof))),   release_fingerprint: fingerprint,   version: "final_envelope_freeze_v1", }; const envelopeFreezeHash = hex(H(   "cross-host-v2/final-envelope-freeze-v1",   J(envelopeFreeze), ));  const PVIS = "cross-host-v2/visibility"; const NSVIS = "visibility.test"; const visMapKey = hex(H(   `${PVIS}/map-key-v1`,   Buffer.from(NSVIS),   raw(fingerprint),   raw(core), )); const objectSetRoot = hex(sha(Buffer.concat([   Buffer.from("cross-host-v2/visibility-object-set-v1\0"),   raw(core),   raw(finalHash), ]))); const visValue = {   core_evidence_root: core,   final_publication_envelope_hash: finalHash,   object_set_root: objectSetRoot,   registry_post_root: postMapRoot,   state: "visible",   version: "visibility_map_value_v1", }; const visValueHash = hex(H(`${PVIS}/map-value-v1`, J(visValue))); const visSmt = smt(PVIS); const visLog = logFns(PVIS); let visEntries = new Map(); const visUnrelated = [0, 19].map(d => flip(visMapKey, d)); const visLeaves = []; for (let i = 0; i < 2; i++) {   const priorMapRoot = visSmt.root(visEntries);   visEntries.set(visUnrelated[i], L(`visibility-unrelated-value-${i}`));   visLeaves.push({     index: dec(i),     map_key: visUnrelated[i],     post_map_root: visSmt.root(visEntries),     prior_map_root: priorMapRoot,     transition_id: L(`visibility-unrelated-transition-${i}`),     version: "visibility_unrelated_log_leaf_v1",   }); } const visPriorEntries = new Map(visEntries); const visPriorRoot = visSmt.root(visPriorEntries); const visCert = (epoch, leaves, mapRoot, transitionIndex, previousHash) => makeCertificate(   PVIS,   NSVIS,   {     epoch,     logRoot: visLog.root(leaves),     logSize: leaves.length,     mapRoot,     transitionIndex,     transitionLeafHash: transitionIndex === null ? null : visLog.leafHash(leaves[transitionIndex]),     version: "visibility_checkpoint_certificate_v1",   },   previousHash,   [5, 6, 7], ); const visGenesis = visCert(0, [], visSmt.root(new Map()), null, ZERO); const visPriorCert = visCert(1, visLeaves, visPriorRoot, 1, visGenesis.hash); visEntries.set(visMapKey, visValueHash); const visPostRoot = visSmt.root(visEntries); const visLeaf = {   index: "2",   map_key: visMapKey,   map_value_hash: visValueHash,   object_set_root: objectSetRoot,   post_map_root: visPostRoot,   prior_checkpoint_hash: visPriorCert.hash,   prior_map_root: visPriorRoot,   version: "visibility_log_leaf_v1", }; visLeaves.push(visLeaf); const visPostCert = visCert(2, visLeaves, visPostRoot, 2, visPriorCert.hash); const visReceipt = {   core_evidence_root: core,   final_publication_envelope_hash: finalHash,   fingerprint,   map_key: visMapKey,   namespace: NSVIS,   object_set_root: objectSetRoot,   post_checkpoint_hash: visPostCert.hash,   post_map_root: visPostRoot,   prior_checkpoint_hash: visPriorCert.hash,   prior_map_root: visPriorRoot,   proof_suite_version: "visibility_proof_suite_v1",   registry_post_root: postMapRoot,   version: "visibility_seal_receipt_v1", }; const visReceiptSignature = signReceipt(keyFromSeed(5), visReceipt); verifyReceiptSig(keyFromSeed(5), visReceipt, visReceiptSignature); const visibilitySuite = {   absence_path: visSmt.proof(visPriorEntries, visMapKey),   log_consistency_path: visLog.consistency(2, visLeaves),   log_inclusion_path: visLog.inclusion(2, visLeaves),   log_leaf: visLeaf,   map_value: visValue,   post_checkpoint: visPostCert.cert,   post_inclusion_path: visSmt.proof(visEntries, visMapKey),   prior_checkpoint: visPriorCert.cert,   receipt: visReceipt,   receipt_signature: visReceiptSignature,   version: "visibility_proof_suite_v1", }; visSmt.verify(visPriorRoot, visMapKey, null, visibilitySuite.absence_path); visSmt.verify(visPostRoot, visMapKey, visValueHash, visibilitySuite.post_inclusion_path); const visibilitySuiteHash = hex(H(`${PVIS}/proof-suite-v1`, J(visibilitySuite))); const visibilityReceiptHash = hex(sha(J(visReceipt))); const completionId = hex(H(   "cross-host-v2/publication-completion-id-v1",   raw(fingerprint),   raw(finalHash),   raw(visibilityReceiptHash), )); const readRoot = hex(H(   "cross-host-v2/read-verification-root-v1",   raw(objectSetRoot),   raw(finalHash), )); const completion = {   candidate_verdict_hash: D("candidate_verdict_v1.json"),   completion_id: completionId,   core_evidence_root: core,   final_envelope_freeze_hash: envelopeFreezeHash,   final_publication_envelope_hash: finalHash,   object_set_root: objectSetRoot,   previous_completion_hash: L("previous-completion"),   read_verification_root: readRoot,   registry_checkpoint_certificate_hash: postCert.hash,   registry_post_root: postMapRoot,   release_fingerprint: fingerprint,   version: "publication_complete_v1",   visibility_checkpoint_certificate_hash: visPostCert.hash,   visibility_proof_suite_hash: visibilitySuiteHash,   visibility_receipt_hash: visibilityReceiptHash, }; const completionHash = hex(H("cross-host-v2/publication-complete-v1", J(completion)));  for (const [p, b] of members) write(`core/${p}`, b); write("publication/final_publication_envelope_v1.json", envelopeBytes); write("publication/final_envelope_freeze_v1.json", J(envelopeFreeze)); write("publication/registry_general_history_vector_v1.json", J({   checkpoints: [genesis.cert, preCert.cert, reservationCert.cert, casCert.cert, postCert.cert],   map_keys: [...regEntries.keys()].sort(),   proof: registryProof,   transition_leaves: regLeaves,   version: "registry_general_history_vector_v1", })); write("publication/visibility_proof_suite_v1.json", J(visibilitySuite)); write("publication/publication_complete_v1.json", J(completion)); const frozen = {   candidate_id: candidateId,   completion_byte_length: dec(J(completion).length),   completion_hash: completionHash,   completion_id: completionId,   core_evidence_root: core,   core_members: memberList,   final_envelope_byte_length: dec(envelopeBytes.length),   final_envelope_freeze_byte_length: dec(J(envelopeFreeze).length),   final_envelope_freeze_hash: envelopeFreezeHash,   final_envelope_framed_hash: finalHash,   final_envelope_raw_sha256: hex(sha(envelopeBytes)),   registry_checkpoint_hashes: [preCert.hash, reservationCert.hash, casCert.hash, postCert.hash],   registry_indices: { reservation: "4", retirement: "8" },   registry_map_roots: {     cas_prior: casMapRoot,     post: postMapRoot,     pre_reservation: preMapRoot,     reservation: reservationMapRoot,   },   registry_proof_hash: hex(sha(J(registryProof))),   registry_proof_path_lengths: {     cas_to_post_consistency: registryProof.cas_to_post_consistency_path.length,     pre_absence_siblings: registryProof.pre_reservation_absence_path.siblings.length,     pre_to_reservation_consistency: registryProof.pre_to_reservation_consistency_path.length,     reservation_at_cas_inclusion: registryProof.reservation_at_cas_inclusion_path.length,     reservation_inclusion: registryProof.reservation_log_inclusion_path.length,     reservation_state_siblings: registryProof.reservation_state_path.siblings.length,     reservation_to_cas_consistency: registryProof.reservation_to_cas_consistency_path.length,     retired_state_siblings: registryProof.retired_state_path.siblings.length,     retirement_inclusion: registryProof.retirement_log_inclusion_path.length,   },   visibility_proof_suite_byte_length: dec(J(visibilitySuite).length),   visibility_proof_suite_hash: visibilitySuiteHash,   visibility_receipt_hash: visibilityReceiptHash,   visibility_receipt_signature: visReceiptSignature,   version: "production_publication_vector_v2_oracles", }; write("oracles.json", J(frozen)); console.log(canon(frozen));
```

The 18 frozen core `(path=length:digest)` oracles are: `attempts/source_attempt_v2.json=1286:e62e048e08a584b16ec607f5dba9fa369b6175a35679a50abbb02000c96b5eec`; `candidate_verdict_v1.json=872:55caaa561d832825df27733235e785292ea5859229c55cbbbb912ba826878f53`; `evidence/bootstrap_evidence_v1.json=388:f4be7859adce34fd7d2a17667c3d540c6a91935c1102fe7343876695feff5431`; `evidence/causal_oracle_v1.json=313:de4109b5bc0e2d8dd2eeea828880727d0195245d6d6269da6847c7ef753f6488`; `evidence/evidence_manifest_v2.json=3044:e48b4b78ecc2010eef890a8dd4978d39c1ea41b04b10799ab7846e670443c2cb`; `evidence/maintained_export_v1.json=598:090713881091de69d5936a04eca4eebf7df11c0f2ef2e6addc0de380700c060d`; `evidence/native_import_v1.json=389:30a4d316d76561e55aae9fc986de01b9f347cf1941f13aad9b949024d25242bc`; `inputs/schema_bundle_v2.json=606:3728821f068f0d4440ad5863961112442374037a652469e41b1f6966b0a92034`; `inputs/task_v2.json=2518:1f4863811640cb66d2bf01130119abf07196b35e80d2d2015d815e38d92ebe34`; `plans/canonical_plan_v2.json=537:15dbb23d94cf2a0af850cd4a50210282798f49684964ef8e9aa99b94db49b197`; `prompts/host_channel_prompt_stream_v1.bin=44:571c3971965db09c78452cf6a650e773af708091e9f2651e4aacd30f56dfeb50`; `prompts/prompt_surface_manifest_v1.json=665:aba5fd23f5128e69c6be37e4c10edc82255e3ed2ee66b84b9e81ec42a61ebe93`; `reports/report_v2.json=1915:7a5b00990800b52b162041895483a2e84284765cd9787eb93608d6c6c45604aa`; `reports/report_v2.md=124:7ba9d4e690a073cd566e5f832296276b9f7ccd0229675fbec09c5cceafadd934`; `runs/run_v2.json=3702:87ed3cb0b2c56d5eaa4082c9de12f1878e2127ad3a480a1abb7db79fd7a4a2bc`; `scoring/pre_run_scorer_commitment_v1.json=1490:f4cac094683bba35682650f050be3b29df157169b56494a1c5f88c7acce4a72b`; `scoring/public_scoring_projection_v1.json=667:49def8bab6fda3ba71ddd45b71c1bb26e1a4e169ee612ef38a2b8dd0598d8a2b`; `scoring/scoring_input_freeze_v1.json=836:e873153427cc451bd7e04b5478fad7438c688b90bc1749016468f4a90c6e2fb6`.

Frozen graph oracles are: candidate ID `c64885ee5bef7c7111eacc627303688488d2416137e7adfbd661bbeacbb7b1fe`; core root `4a4229afc7130137450a5cd2736e3f817db6db6c4e640d1956d15fafb3448970`; registry checkpoint hashes `8458a1baa345f2c61fed2fdb3f27691a282b9fdd4e0e2d4e4094ee1f79b15e7c`, `93c316304dd7ee02d4b95ad12b9b8ec2b68471d7fcbe7e21ae6452b0c1969077`, `3c95d740b0f08e7edd6fef53c071347df1c9d0dce2f690ecd915cb0944133ce3`, `71bded4efba2d3d9377823270ef7681ea24fc6d68357b290981f555bcf781ab8`; map roots pre/reserved/CAS/post `234d0a55d8a1c14828d0d6c54b61a1c13ee88f92a0ef93b10e82b61aff928d17`, `23d4c6b17c200884af189275e8f51e4fe43642d66b7de13c5584cff5d726fd20`, `fefb37e0b55a7451b32b1f8d8a5a67fa7a331cf599ba1586c2a70b6804aea269`, `1541691ff729021d61114d3c2b61c96263c00e401dbde260e63c05b6bb54cf97`; proof digest `f4d4f73aefabd18a1ea985dee826bee4b3176fd5711f05b8e9394833457b049c`; envelope `14911` bytes, raw `bc8f87808a0c8e32ccc7bcf38214bd1a46c5c56322c4b9034ece1930729c1b5a`, framed `06746bebb97513a19938a4bf9f82c552645effffafcc964e347b49453d6679fd`; freeze `1062` bytes/hash `b3d1ef506a84b04edd882fdc99f8084ae63816355f3f10f0331cdac8937fd0e8`; visibility suite `4929` bytes/hash `c878d09facba0eca7659854e755f27580432a134c059c622eeb5244129bc6802`, receipt digest `99812fd947ef8139910f10093c5b8fa532b7efa60815b741fa1875cb8fbe6dda`, signature `1a8870467f2be5f096a811b360e41e60791f7ecc01dd58e340db9e36ff9eb94e042f1aee71815753ad4ef0f4b603f2241cdf68ba24180c8bba0b948cee82fd08`; completion ID `fddeb8cb3fc14789d805a666f9de9314082a2766dcb53dabdac07a575a510e94`, `1349` bytes, record hash `39e8ec327c54300e6d38c0d0e268c179f6b39ac536aa9af8a2cdf0dcc968d2b3`. `oracles.json` emitted by the exact generator is the machine oracle.

The generator's output was independently recomputed by a Python-stdlib/OpenSSL implementation (source digest `18471dbdb78a4494e3f9f7d76f79445d02fb05c5b0784ea09c55b1348e354390`): it reconstructs all 18 JCS/member hashes and core Merkle levels, all four map states, RFC-6962 inclusion/consistency paths, five registry and two visibility certificate signatures, final/framed/freeze/suite/completion hashes, and the receipt signature without importing generator code. Both implementations must produce exact `oracles.json`; any difference invalidates the vector.

#### Completion transaction and crash closure

`publication_complete_v1` is exactly `{"candidate_verdict_hash":hex64,"completion_id":hex64,"core_evidence_root":hex64,"final_envelope_freeze_hash":hex64,"final_publication_envelope_hash":hex64,"object_set_root":hex64,"previous_completion_hash":hex64,"read_verification_root":hex64,"registry_checkpoint_certificate_hash":hex64,"registry_post_root":hex64,"release_fingerprint":hex64,"version":"publication_complete_v1","visibility_checkpoint_certificate_hash":hex64,"visibility_proof_suite_hash":hex64,"visibility_receipt_hash":hex64}`. The ID/read-root/record-hash framing remains as declared above; completion additionally rehashes and requires the exact freeze.

The completion ledger is a durable SQLite transaction, not three independently observable file updates: `BEGIN IMMEDIATE`; compare the singleton head with `previous_completion_hash`; `INSERT` the canonical record into a table whose primary key is `completion_id`; update the singleton head to the record hash; `COMMIT` with `synchronous=FULL`. Exact pre-existing bytes return without a write; duplicate ID with drift, stale head, constraint error, or commit error fails closed. Recovery opens the ledger, lets SQLite roll back an incomplete transaction, queries the ID, and either returns the exact committed record or repeats the same transaction; it never calls either authority.

Crash tests inject before `BEGIN`, after record insertion but before head update, after head update but before `COMMIT`, immediately before `COMMIT`, and after durable `COMMIT` before acknowledgment. A separate connection must see the old head and no ID at every pre-commit point; kill/reopen must roll all of them back. After commit it must see the new head and exactly one matching ID together. The former “after record fsync / after head update / after ID update” states are not externally observable contracts and must not be asserted.

Tamper vectors derive `authority_unresolved` / `INSUFFICIENT`: change any member, proof, checkpoint, envelope, freeze, receipt, completion, or signature; add a later checkpoint/signature; alter an unrelated intermediate leaf; reorder or omit a non-default sibling/proof node; expose bytes before visible commit; make a pre-commit completion mutation externally visible; or expose fewer or different bytes than the object root.

`publication_state_v1` is closed and evidence-derived: `reserved_unpublished` requires exact reservation plus authenticated visibility absence; `retired_unpublished` requires exact retirement for this core plus absence; `authority_unresolved` covers unavailable, stale, malformed, conflicting, alternate-authority, or wrong-tuple evidence; `visible` requires exact retirement, valid visibility receipt/proofs, and all exact bytes. Timestamp/local/bundle assertions never derive state. Registry and visibility checkpoints are independent verifier inputs.

| Crash boundary | Closed recovery/result |
|---|---|
| Before core freeze | Proven reservation/absence is `reserved_unpublished`; rebuild private staging only from sealed inputs. Drift/loss is `INSUFFICIENT`. |
| Core frozen; CAS unknown | Exact reserved/absent permits exact CAS retry; exact retired/absent recovers proof as `retired_unpublished`; unavailable/conflicting/different-root evidence is `authority_unresolved` / `INSUFFICIENT` with no mutation. |
| Retired; final freeze absent | `retired_unpublished`; query the exact registry result, rebuild/rescan from the immutable candidate, then create-or-read its one hash-chained `final_envelope_freeze_v1`. Competing bytes or proof drift is `authority_unresolved` / `INSUFFICIENT`. |
| Final-envelope freeze committed; visibility absent | Rehash the exact bytes against that candidate's freeze and retry only `seal_visible`; drift is `authority_unresolved` / `INSUFFICIENT`, never a new CAS or evaluation. |
| Visibility call unknown | Exact receipt/proofs and all objects derive `visible`; authenticated absence permits the exact retry; partial/different/multiple/forged/unavailable evidence derives `authority_unresolved` / `INSUFFICIENT`. |
| `visible` / final completion | Commit the closed completion record, unique ID, and head in one SQLite transaction only after exact retirement, envelope freeze, fixed certificates, proof suite, state-machine read oracle, and every byte verify. Recovery first queries `completion_id`; exact replay returns one record/hash and never seals again. |

Failures/recovery are append-only and preserve the last provable state. Non-security gaps are `partial_non_security`/`INSUFFICIENT`; verified private leakage remains `partial_security`/`FAIL`. Recovery never creates a second retirement or visibility transition.

## Implementation Slices

These are sequential handoffs, not permission to run live hosts:

1. **Executable inputs/runtime** — build schemas/tasks, exact plans, isolation, source/condition surfaces, scorer, causal records, failure retention, and caps.
2. **Publication pipeline, four separately persisted/tested steps** — (1) close writers, recompute/scan members, freeze `core_evidence_root`; (2) exact idempotent CAS `publish_and_retire` with its immutable certificate; (3) build, validate, scan, and create-or-read the candidate-specific hash-chained envelope freeze; (4) atomic `seal_visible`, verify the full proof suite/read state machine, then commit record + unique ID + head in one completion-ledger transaction without a second seal.
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
- cross-implementation RFC 8785, core Merkle, closed final-envelope/registry/visibility/completion schemas and production-shape/tamper vectors; invalid paths, empty/odd trees, wrong field/order/type/hash/proof/fixed certificate, deterministic Markdown, and hash-bound verdict;
- scorer-oracle missing/hash/tamper/unrunnable and clean-checkout recomputation;
- pre-run scorer/read-set commitment, frozen workspace/output projection, scorer-oracle disagreement, and result-affecting undisclosable-input rejection;
- published stream missing/redacted/private-result byte, manifest gap/overlap/duplicate/relabel/closure expansion, slice/root drift, out-of-closure mutation, and rolling-send mismatch: `INSUFFICIENT` unless verified leakage yields `FAIL`;
- early oracle/opening visibility (`INSUFFICIENT`), raw/encoded private leak (`FAIL`), scanner crash (`INSUFFICIENT`), and duplicate publication semantics;
- authoritative-registry fake/alternate namespace/key/genesis, stale/forked/split-view root, missing fixed certificate/quorum/gossip, arbitrary prior/index/intervening history, invalid proofs, CAS race, missing retirement, and fingerprint reuse: reject pre-call; observed evidence is `INSUFFICIENT`;
- publication-DAG cycle/self-hash/final-field-in-core rejection, incomplete core graph, core drift, different-core CAS replay, same-core recovery, candidate-specific freeze create/read/drift, completion previous-head/ID drift, pre-commit invisibility/rollback and post-commit all-visible crash points, and every closed state transition;
- alternate visibility authority/key, forged/mismatched receipt/proof/certificate, late signer/checkpoint substitution, non-empty sibling reorder/omission, early-read API/state oracle, non-atomic/partial exposure, changed-envelope retry, collision, and double transition/seal;
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
