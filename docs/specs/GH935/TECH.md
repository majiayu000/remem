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
| `M=0,R=0,N=1,T=1`, registered post-common-source non-retryable preparation failure (`terminal_reason="post_common_source_preparation_failed"`) | `proven_no_write`; false `ordinary_failure` | current attempt; `complete` remains possible; never retry |
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

`final_envelope_freeze_v1` is exactly `{"candidate_id":hex64,"candidate_verdict_hash":hex64,"core_evidence_root":hex64,"envelope_byte_length":dec,"envelope_raw_sha256":hex64,"final_publication_envelope_hash":hex64,"freeze_id":hex64,"previous_freeze_checkpoint_hash":hex64,"previous_freeze_hash":hex64,"registry_checkpoint_certificate_hash":hex64,"registry_post_root":hex64,"registry_proof_hash":hex64,"release_fingerprint":hex64,"version":"final_envelope_freeze_v1"}`. `candidate_id=H("cross-host-v2/publication-candidate-id-v1",raw32(fingerprint),raw32(core_root),raw32(candidate_verdict_hash),raw32(registry_checkpoint_certificate_hash))`; `freeze_id=H("cross-host-v2/final-envelope-freeze-id-v1",raw32(candidate_id),raw32(final_hash))`; its record hash is `H("cross-host-v2/final-envelope-freeze-v1",JCS(record))`.

`freeze_ledger_checkpoint_v1` is the closed `{"authority_signature":hex128,"body":{"head_hash":hex64,"namespace":id,"previous_checkpoint_hash":hex64,"record_count":dec,"version":"freeze_ledger_checkpoint_v1"}}`; its hash is `H("cross-host-v2/freeze-ledger/checkpoint-v1",JCS(body))`, and the charter-pinned seed-independent Ed25519 authority signs `"cross-host-v2/freeze-ledger/checkpoint-authority-v1" || 0x00 || raw32(hash)`. With `synchronous=FULL`, create is exactly `BEGIN IMMEDIATE` -> select and authenticate the prior checkpoint -> verify its signature/head -> select the singleton and require both selected prior hashes -> insert canonical record -> insert unique candidate index -> insert signed post-checkpoint -> CAS the singleton with both prior hashes in `WHERE` -> require `changes()==1` else `ROLLBACK` -> `COMMIT`; all new rows and the head are one visibility unit. The vector supplies full-hash create, exact-replay, candidate-byte drift, candidate-hash drift, stale-checkpoint, and stale-head operations plus authenticated replay rows. Every crash point before commit—including after either authenticated read, each insert, CAS, or the `changes()` assertion—reopens only the signed prior state with no new record/index/checkpoint; commit-before-ack exposes all new values. Lost-CAS recovery verifies the immutable authority result, rebuilds/scans once, and create-or-reads this freeze; visibility accepts only bytes rehashed against it.

#### Closed visibility objects

The charter independently pins its authority namespace/genesis, pure Ed25519 key, `P = "cross-host-v2/visibility"`, witness history/quorum, and gated object namespace. Map key is `H(P+"/map-key-v1",UTF8(namespace),raw32(fingerprint),raw32(core_root))`. Absence has no value; the only value is exactly `{"core_evidence_root":hex64,"final_publication_envelope_hash":hex64,"object_set_root":hex64,"registry_post_root":hex64,"state":"visible","version":"visibility_map_value_v1"}`, hashed with `H(P+"/map-value-v1",JCS(value))`. `object_set_root=SHA256(ASCII("cross-host-v2/visibility-object-set-v1")||0x00||raw32(core_root)||raw32(final_hash))`. The log leaf is exactly `{"index":dec,"map_key":hex64,"map_value_hash":hex64,"object_set_root":hex64,"post_map_root":hex64,"prior_checkpoint_hash":hex64,"prior_map_root":hex64,"version":"visibility_log_leaf_v1"}`.

The receipt is exactly `{"core_evidence_root":hex64,"final_publication_envelope_hash":hex64,"fingerprint":hex64,"map_key":hex64,"namespace":id,"object_set_root":hex64,"post_checkpoint_hash":hex64,"post_map_root":hex64,"prior_checkpoint_hash":hex64,"prior_map_root":hex64,"proof_suite_version":"visibility_proof_suite_v1","registry_post_root":hex64,"version":"visibility_seal_receipt_v1"}`. Ed25519 signs `ASCII("cross-host-v2/visibility-seal-receipt-v1")||0x00||u64be(len(JCS(receipt)))||JCS(receipt)`. The suite is exactly `{"absence_path":transparency_path_v1,"log_consistency_path":[hex64...],"log_inclusion_path":[hex64...],"log_leaf":closed_leaf,"map_value":closed_value,"post_checkpoint":checkpoint_certificate_v1,"post_inclusion_path":transparency_path_v1,"prior_checkpoint":checkpoint_certificate_v1,"receipt":closed_receipt,"receipt_signature":hex128,"version":"visibility_proof_suite_v1"}` and hashes as `H(P+"/proof-suite-v1",JCS(suite))`.

The previous seed-`05` receipt has 988 JCS bytes and SHA-256 `fe56b08b33bf736d023ec354f2fc65faaae422a9f70477164f611f7a85ecd551`. Its correct signature is `437b433d295c00f5fe51cf5dde4abc93569cfede5e0474e951234a1129f2e2066fbff64c28cbe5eb7fdf6d9c8a5a4876d6217436823dd6d523e07be9c9d0e308`; the invalid legacy signature is exactly `54b33f10115b693c4219fd7362c6cc2caca5014a7b51e66d1498d77bb428f5e4ef8c0772ef70939ac31b12d79d9f79c48abdd0c6a3ee503d9128cd707d71e304`. The generator emits the complete otherwise-valid tainted suite and completion record, binds their frozen hashes, and asserts seed-`05` signature rejection before accepting completion; receipt-only IDs/read roots cannot rescue them.

#### Executable production publication vector

`production_publication_vector_v2` is normative executable JavaScript for Node 20+. It uses only built-ins, writes 27 files (18 core plus nine publication/oracle files), and pins the exact path/SHA-256 set for all 26 non-oracle files inside the authenticated oracle. It embeds a complete five-file workspace with runnable scorer engine/wrapper, exact fixture/IR/oracle bytes, command, every consumed-file Merkle sibling, sanitizer/deriver/runtime/toolchain bytes, and recomputes every binding. Its fixed `exactKeys`, recursive object closure, no-JSON-number/decimal/range/enum/relationship checks, certificate verification, and final frozen-instance byte equality make `required` exact and `additionalProperties=false`; positive exact bytes pass and all 17 named nested/binding/selection/count mutations fail. The fenced payload includes exactly one trailing LF before the closing fence and has SHA-256 `ac336fac0914ba6c2fb685d3133907e0a8263d8d0ded7b05afb0b08fdd55fdf3`; its inflated full source has no trailing LF and SHA-256 `f3923bdc423ae1f5e8fdde72e4ef3645eeb2d7e8921ef30512810bffd477eea0`. Extract without adding/removing bytes, run `node --check /tmp/gh935-vector.mjs && node /tmp/gh935-vector.mjs /tmp/gh935-vector-out`, then scan the output.

```js
import zlib from"node:zlib";await import("data:text/javascript;base64,"+zlib.inflateSync(Buffer.from("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","base64")).toString("base64"))
```

The 18 frozen core `(path=length:digest)` oracles are: `attempts/source_attempt_v2.json=1324:0b3fdef1b9aeb708bbe33759f4693248f1c2b5ca0351bdb29833e22e19bf7014`; `candidate_verdict_v1.json=872:03112a4dcd53e86ddfe42b8f3a9282692204cbae50a7bc7fe78edb22eb663286`; `evidence/bootstrap_evidence_v1.json=388:f4be7859adce34fd7d2a17667c3d540c6a91935c1102fe7343876695feff5431`; `evidence/causal_oracle_v1.json=313:de4109b5bc0e2d8dd2eeea828880727d0195245d6d6269da6847c7ef753f6488`; `evidence/evidence_manifest_v2.json=3063:542e6e3a58c618386437ec29281da74359e228e3ec14ea7279be97389542db17`; `evidence/maintained_export_v1.json=598:090713881091de69d5936a04eca4eebf7df11c0f2ef2e6addc0de380700c060d`; `evidence/native_import_v1.json=389:30a4d316d76561e55aae9fc986de01b9f347cf1941f13aad9b949024d25242bc`; `inputs/schema_bundle_v2.json=1177:549754a0f9e295bd9baa7809e0863e3f08e07758b001c6de0da2725dcae7d10f`; `inputs/task_v2.json=3004:e51e0b42adf2a78fc29509685031aa277458b7917cbccaaf11834528dabadb25`; `plans/canonical_plan_v2.json=537:15dbb23d94cf2a0af850cd4a50210282798f49684964ef8e9aa99b94db49b197`; `prompts/host_channel_prompt_stream_v1.bin=44:571c3971965db09c78452cf6a650e773af708091e9f2651e4aacd30f56dfeb50`; `prompts/prompt_surface_manifest_v1.json=665:aba5fd23f5128e69c6be37e4c10edc82255e3ed2ee66b84b9e81ec42a61ebe93`; `reports/report_v2.json=1934:83dba76a1e0cce6659abccf21862915c92e4bfa416b709a363339084fca41f96`; `reports/report_v2.md=124:3e64168a35c40b69f619c1911a7d3b0cb26dc7d423b611ce5aa7b7bb1f252d36`; `runs/run_v2.json=3721:201325787bca82b4106420e8ee13e9175eb5956fde1c5e5d415929d22e0b815a`; `scoring/pre_run_scorer_commitment_v1.json=3600:ac4ddf835ad17868c77171779cc20f147673221220cdbac6216bf9fb5c1e2db7`; `scoring/public_scoring_projection_v1.json=4406:d67bf254b4f15ce80b67d839e69fe9da3c9cc9e98ac030d07df3d2420a5244e9`; `scoring/scoring_input_freeze_v1.json=4064:156798445dfd349fce7c64a029b879097d7e70a1ccc46a835f7da96a8723dd01`.

Frozen graph oracles are: candidate ID `9ff394258f5ce71a619c530f5c4b425dbfc1ff85e4bdd48983f2f2dfa289e7c6`; core root `2c53f21c162f9c66cef4f0418f5f8eab0838be756c328273523543c8695310bd`; registry checkpoint hashes `8458a1baa345f2c61fed2fdb3f27691a282b9fdd4e0e2d4e4094ee1f79b15e7c`, `93c316304dd7ee02d4b95ad12b9b8ec2b68471d7fcbe7e21ae6452b0c1969077`, `3c95d740b0f08e7edd6fef53c071347df1c9d0dce2f690ecd915cb0944133ce3`, `262e21c2447b4f37e7f716c662d51e5f5868ac8d825d0df50205d1dd759257cb`; map roots pre/reserved/CAS/post `234d0a55d8a1c14828d0d6c54b61a1c13ee88f92a0ef93b10e82b61aff928d17`, `23d4c6b17c200884af189275e8f51e4fe43642d66b7de13c5584cff5d726fd20`, `fefb37e0b55a7451b32b1f8d8a5a67fa7a331cf599ba1586c2a70b6804aea269`, `eb48d0e5936a0c6b43f2f12a575474053004ce0bcb47196c5421ccc4b8c7cc0b`; proof digest `b0f69c72c465a487fc4231d8ec03eae3b46d4658a48c86b05125107cacff76e4`; envelope `14914` bytes, raw `201e2e75cecba3570e9bf08cc674f3f4450be1826ff04b169fd8062170d0b660`, framed `1bc1a9d1ae9b8938f0539b019ce74cb81af8864e2d68347eb54c1bd60ba0f2f6`; freeze `1163` bytes/hash `e310f6078d10d32c954ce911e5632ecf21f50d1f7e48a82f3d6e4d7c0c7125b3`; freeze-ledger prior/post/vector `8eaec7644915e4e6b6a3e09572610456f903c1d77eb434c3bad9965ded34deb1`, `f206e1289c66e96f6999f7675aac4c2eebaef49589aded4f8c5c23462aa0743e`, `762a09d8ed207a4c438e478d1ff43d3d0b17054ab64df82452eae917230c9798`; visibility suite `4929` bytes/hash `a4278f3276d4159c50b5487abe705530cf21e5295ddcd8fadab1517c00af1bd0`, receipt `e987f6a8d285c6c6258bfa73d9e07899d9d8386201e70f312d10dc9691521a0a`, signature `4b5faa969185d1ad5a5a2037c2caa9a0ffa13476c22ac63038a87365878d5485a30c6e104a33fc274f41266dae6f57bc134bc6b51609f70cd0ccb27bc4a2000e`; completion ID `b558766d9300f8c387ef58ce781799c6f1cf2e649ea95b0a79359723880daa20`, `1349` bytes/hash `0cdfdff92c63cf28787d06172f4faa14a075034a346c3bc45120a4edd56bf826`; tainted suite/completion `f95780c339edca9fa6c821d14ab0fec1134c5cf4f16f86dd92fb84f064d99f25`, `12729fc0ef1d566c6466d19b1efb7627b4c82783906d1306398ca42bce23fb58`. The exact JCS `oracles.json` SHA-256 is `738218b2b799a992df5d265bac764d07b2c76b5cfaba4bd61871c172d7634f5d`.

The independent verifier below uses only Python stdlib and OpenSSL, imports no generator code, first requires the exact 27-file path set and each authenticated digest, then recomputes all 18 JCS/member hashes, archive/recipe/file closure, every workspace Merkle level and proof sibling from exact bytes, and reconstructs and actually executes the scorer with exact status/stderr/stdout. It also recomputes core, registry, final/freeze-ledger/visibility/completion hashes and signatures, concrete crash/replay/drift operations, all 17 schema-mutation oracles, and rejection of the complete tainted seed-`05` suite/record. Its fenced payload includes exactly one trailing LF and has SHA-256 `266e6fb3389b01eab9a01e25302a625a865b709eac48a10d9f6dfbbcc1ef28a2`; the inflated verifier source includes one trailing LF and has SHA-256 `b70c4cb6bf81ae859cd56a8b15e3a12b3a8c0a5dd9a591da9a5264e5fae54f26`, and hard-pins the `oracles.json` digest above. Extract byte-for-byte to `/tmp/gh935-verify.py` and run `python3 /tmp/gh935-verify.py /tmp/gh935-vector-out`; any difference or accepted taint invalidates the vector.

```python
import base64,zlib;exec(zlib.decompress(base64.b85decode(b'c$~#OYjY#Vjo<kzCUcd9v?+?D-YUzfjjdev&ac{aa&^b!v4)(XG`r-Go*8Oa-qL?RJo<s|=^>?!bGKDC$$kK60F6eY(XD^^-SMs}kLP)DoD~nz_OZU-6epd0z1@^`G*7DR{8azCPpbP>KG#3rRhvS8-zK%Js&?~jxmjdYWlA2+*E(Bom-#B|bl&{<<ImBJ?mYMjKD*Fxkd${1lhL%-`LEkIKSG0e7)M`3K**18UVZc3?c3SgzrGruony_#$>n%-IUmn2uC9`+t8uzKOULJD^JH;xewq$1=41FWKU*x5d2%{W&qtRRqs8cAoL-!toI=~U)9IwyGJ4m2NLIV7cioBLUzydrvY-(T(%pJnacw`!ifUJ8v!q(&`OROFRRv{LwoS^U-jvl%H}1#%D87z+Pz+<8z0V%2o1aSnJjja0Ce6A%g|NC$x@l4;E=I-T0o1|(xxpaK@3N{kJ<H@nh}ijKomGQnxmn+5A5{a@V|RWU>X8?<Djw9E8TId80{Qd&F4kS|yLEEckG|OE8HiK$UzW)_OM(4RXT}wM8O`JW9>&~aQ%2MUknop^yNd2#-VkA{tmsmU9)h56l1StechDp)?z6>v=(eo;QJLS}!^bL_XDdZqzKpOg`rQrJTD)bFS6TF`s<N`qH^psPZp!X5{zSc>uA@)b?+H78qMlEN3yTd1z1(c-?mAn~vr;o+l|5uD;KO8UM-58ck5F(pbX0;Y)x8Ee=zi7h^(;-RELj2o9HlA4fAe@zZmR0&epA&)594E`;3#{@Q($*?^e{Tcj+)#0-<Ou5K!uS^ujjRLYFi+X>iLyMX-#=gj0AWwNVeOoNV`ZKSADpLsU9IjYS4?mjz-sRCuE?5sKJeAaioCZ_#9!f3Ckoe((L0jP~8>v@N+ap9WuY7uO#&uOnC>M%Odv|>I1_8RVb6<F6$2a0$M*B+YVEy^^9RS9pGHnTDaQW&6c}jfy!HTw`I1>KPpwCYT&su%>}%B)f@&C9|N2tYz#~TYo#q8V9}6J(`RUkz##C4fC#)PHoO?fNxcsl82tJXvGpVr<b#49-P}m6kg%u$aKcnp+f`nJ%79?Lin?71<tQ5U2J^h0LE#;&C0x?vg@eeW6}5;M<5JV$$mE&`nf4Agq;}V`p~q?#K-H9*1U3=kCB>qzs94;4+S9tCLFdI{wW~l^x}<~sC?v{f31RYEaNo?SuNX)WQwz<8*&wFL9u_kTram7%s6ZEWFwk}P7U&wehHf~_Zq8=et@2+p4XqEW#d^ES>dfUdwm}vPmbEY&TWc6>Z~{~{#wt(@|EW?WEMML1kz$Z9nvH@#pC?TRdYdf!5qW1X5~+g!$smkz&c)Z@UaaS7!V~RU+F9;yCw7depaXu0QlozsmQrt?yVp&<KRn$9`~|EBc~!$g_-HXO?==F$G<Tt;HV1L_BXnV{`C`;h+LK+^vxltAmygQs_m$zoaexENUkDUQp&t<?O8aSTKo8)DO_5E(%^#m#*op~w!BePpmJ~p!mK42;^^E(KX50GSF1O>evwn0m!vDl<W6;zz2;8gzvL$&2%@Rfp6K0)k0Tw#vG`va7WwB~e<?~fu+*Q!#(-V~}b|tun@O#)!{kdyH7^K2}d>ma)qd!Dlp2Zh16}aN}(PhuGEgHKWv>;Sh#?uMY<T*8pQ1I7BOgK5<T*~fD0}!}2F>un38dY=$PP`Hos0Gk~8xW%)qw}8j->t4hc@?2z*wLVDpsUpmRu-7+jW!LrTBip`uQ2!ec1J}xdoR=`HAeK})o2HIS>2_+ZTg~g2Kpcw(br!`2wY1Lc0zwpycWZ8r|ur5Rb=$3qbo|*A*YBIm_5+&n}O*&{;UevM43V{^+^H9AZiD81ABw2T4`%hWm!tw&VIDcsw%l7Ja<I`W>_ztA9CQEju;Gn&M<_Kl#k!$WwwAV_am%NNfo8~r<IB+@&gQ}!j6Tt(e^lwVkjD<S!s6@+Z`}q;Wy*b!8TiqmdWF4lcbHNTnuY#5ewb9^V=B?xGj;O4<&3M)kfXj?#Cy?aW)(eN5kRCaCkO6AD)co=Vuqg;pun`{|#f>pHt6?;X8OdUqmNj529(5h?Ho0l4hp?&K@f^(ar>c2Ia1BwRaLLUBGb37RH93M|lC0ZjE2Ry?vufVp>D+JMen5tErejnfX)K0Vj>5kc(Sh)hYD45k!4^`}ZII_T6`lW?5P8ZT8F&Ha436kasnGJ>uCAt0B^eap24p9%H`=><HLy+G0Z%9sUMk;A|mFaOZaYe_$3*eIDd&fuWaWzDVlKI18c-gK1f+9pm{X#d#^}VNr#$m~B3X#e6$YL)va&X*m)d^;Q?V_w-H)fp{j#iWcdBZKoZ{Gms>^`hEj5gZH|P$QXR$Tw%I}|K<$Pz?hji6)P(2x`c5g<`phZzvkPn0|Sg^0n)G#U`1A}os7V-wpOxj_?iW-yr01Wh5BU-z<)TTN!%E-9k=q%xie^$im9<bg-k)6M|N~6tbB}aF~I2!%DdHO-i^PAZGey#lf0VYXIHR8!JJTinXQsKf5>L_#s`^Hv+btJaq9s1RqG^G2C>(HHEOX^_7PWZ-JQ#;0S1<lGTDchQZE230g>A3Sa3gbW8pCGVJ-MID<XwMF)G%?ARQFXMk`UdfIHd8Ei6E=D&SCvr7o%09=0d1$6&WjaY}z$5{HgIttWrPMfZ=>C+LfErQyU=(%v;nNx`2`hnZr0Jbm(s(V$IxwulJ^!KlSd!|rvGMZHT_b`U&Y8iJJayXFA1BMNCBNU@M!>?l-;w8*MZ=c$7`V{rQ<4Lyt{W1%}F9fc2tqvPx&K#0Z{N2JckM1%lh)G9tx5oWf2K$*uc6=S{@{+Xm{Vqi=)kbbm`Kbhhut2tTmkgdQ(xT<5cSxDk|nHPXT%0_pdJ&Z#74g)H)?APbRHt@LL0kE@GmfmHh3!PZ<0u&4bj3YW{RSU*WuX7KZZ}v0~4W-QPpl$g$yUU8K1eB_)!qGaYha4R6e3jSFL1G)vXk#|3cId6O!B*iI==luTQo=fp(+asxV0i$=Y@@zi9=a#4LBlfvVDDHu7G>L@o{iE1Xh+u3M^S}27lM5(9-;A1*!AuToJeZN+&D$*duC7{XHY9#v#D5bq8N%+(zY&#Aeu?dYgtG86J$zU*v2ZWLk2w5x-p9+uh?iZ;^|aa8!ksKsTd+TA9X2?l`Mx;i(aF@w(5O*=%lxpx^KfynFaTq;gZiaZqnap1zuppfR=yz=B;6ykr9s6#RyF?+F^v_861aOJt-x_Anrjy5piDVpNdhdBQb;{4GCN_eL7}O`8*<fs@g5?=mn7nX9*F?)-a%WVoQxk>8P4Qw6U?!YAE;J;1CSi+*&*E{Hy0tWC89Y$uwe;c==H@NkF=5P!V*0$psCI${N5-SMYsju3CHSHwv!cK?jVTl-V!4yrctnc4^|g*zW2IOjh-N_Au5ynh0DctJUTM2mohig_}fl&hKEM;qSW&XjhEF5)MFMfSP3Wkvwn_U<MnJfoG1($vbE`OS8p>KtaVkPr<@iOm~|VxFX<ksF1sabI)yItTnYok<Zld)>*-Y1v_h1k!-8`O%0wJNa2wgFJNuanC^;Mg{G=Rd%ezU1fp6-G1X8-QltdB+U!cOh1oW*U}NHtrmB!4YFQ`X4=wJ2kuU+7OHfwt=v$qXcUf&3lN|C?f~z;_2CHCWMS*`6GUtytsWGGl5>!~#@31UENJ+8VlH~a7%u=%06lqQxT4cK#Jqly-eTFTN*~+ESEOG>&6)C7pLPH`A3rG;SPT4MqF_~A0i0D8`(9s1mJ}JPx5oA^wu#;goSgr?;3^aw<!3q-iT^rIu+|J51%||fK6%QuNcp$SnMq0zOMp2?!h;X2+%;!7JBWLay{s<vCr!p)WS5Cn2O_?g;vHrA3!Gg*@W{X{&%vYJ=$##_#&W{48J8Wt+9GI)*(>Lc0&0*ULMoba}j_1F6K+3GztRAw|6S0sX3??t`SjITW1f1D*20UBH8K*hx6Z4rWpcPAK0MAGlkSauB*5ErD<<UT&r%Ck=9Ko52QHIq@E6lN(d|-0&C78JvDH@eI<}Qlo;J1PElx$6jQo-i*UT-%A+COyli21NH#3VG`ahc>e33)v1nwqtS{X+rnJ%}2s*LhWefc;)>ag|vt%6$9e6=2D=5ItDUDhW?0x=>=J@E#<$*nvWNZpGpcSJiVdO@@kgIH@vkG-;a%^?pu*GJ8CjJX!g`w$cIyLs5Y@0?f<GEzBLKxXf^?WAsbgEMaIW7(q?*6w8?|1HEl66cY{*EdnwK%u~kKVn;L7EHI_?8CLRE&E>U4)12nXT>*wLU(CqDIn&x$MpI`en-!*puk@W3I$ywT%8xNKl+<ozNn-m2KR-Ccv`hu1@b0Ku)%~yz`<fWiypXo(S^t?T_Dof3Qz2i)P-ZSms(Q1Xt-#Y)j3wPl^F<w0GmhyNu#KPtpo&gml_MOAabcKE?DO>|6#-0f!-+5`(1j&W(E<noj~YwW9+*+-#7!?_(?pK3)c}_03J)f`vDy^c2r?7%PLiUxy!a_7fQ_R$7)5_Ng<Sx!GPSA3SeDM{^ozH;{9Puysz$Igpk5QvX)5>ZoX9<xpPw?Su4+hm894@6IZIhE)OJx}e3gvM{d<N$MGzheMBw*r3fMBO6hdcT-`K}9$#!T{2}<qDu)B`@Lto$*F4}HPN6R|6Et>W^Y};D;9CF2>Kg^Dn&JA}GT5GG+(&ZVh0-_|l+J-f3d~u{SX<ND}SM9ic<D`Qe3KH77P>{9(Q8(uK?!{*Tm~}Z1kw!c2nTBWkNocH1FoKD2v8QBnF=C}QxhOPK4Z6Bp$4*z@(L(@RTsV*)fKaJ}-0$!bqJe7wkpiHhbH^ZpR>a)rcpq5mtNdQ<0=62Of!#-{XuHm2t+G4ltQtC<jW4g0<5Z7bVQReg{)PZLVls~EXY71r8aVwMPVRJK*N(+1$=7<aS>80f_^DWMBHdi0;b5+@y{%Z+YLOCWimh$S7ql&2(6)XF4cP<keNc$@@RvWj@en0V{~MYA!9;lgHwGpEC1%7uH_i}&3fF2+*KwGKF-dKICP9xxCmOmfgI)<G(7NqHE|Z3+(cNfZU#ixE$ODjI=%**d0b_MlJ1_KZ;j}qsAz0A%NVWCgvFSZXl``W#O&t|to0*l#aR7)rxQI<mEN*J!a@sPf6}DMnwtHM0adF(iRb`QHpNEbmWY+W9hfVpu+9r6AKtK%ogAK(FBsP^1SCXVfP;d`vPUvuonHdqCF;s#PD!AI$fyM}1$3~eL(1Ez2ip;+2a(&MRWy$eSE9@M0$Ec}#$u%brDi=W|N?<kuk37vv*d&x3;JM2Svez6%qHnvJm(FCZqA7ID01Fswpx8qOn(KV6!?I={SZZgT7GU~SZ#JvNeZtv6A4<HEQzEC?Wt0R@n9gaC@3wJYT#~Tt7G+MvwTv*)gKJgZidBlFT-*aAf(Tuo!&e#I)ziFYeOTdCt*|Y4{?X{I&S_jyCIU<2q{gBkmV!!9RGTIGoN=l!xKaVD&9YQ87|S9qL%mjENZ!5%aakA2=$qc{T1;-a#>J%(mm)T$InTxBG?%l~Dfd*zVpI7rmI?jH>1lTFdQK{~=$_7?nti$Y*zargQ~0S(E$Qfeo>iMhqkcgB)UMBNS#AGMG395-&G1tY1aDmp6jeMiy{5t-G-n5$qXnSq-i8HyqJx?F@}M`8$HfLUQ$C%#Br$}(z69x%x-*B;T+jz7W4%hvHgi-ZR}frfx$iW^<YW5XZ!R<r-<@8D_<rP(-2}pCT$V#>b3K`M_Hji<DMWr@O$Xf~3+|J1W*%7HNCI}5dlE4hODd%eDHV}vgNjyX&T<;zo@La6MUyV|KyK{Kxjx&@!1=!`?WMXAY7hP;L-iF>zGrHCVVd-!?qaf|Cn(vwr-f`f!lt8K9CLonp#E6%;jQ$Tgs-G$gI(Vn(Ac$50LdRwJJWt7d=%@0{9_=U)ml3YQ)``{0i8h{v)eIPzpI*Xl}j=;-pLE#THm9EXyKe7g4b_e09<)5VXC!jGqr9`0Cb1$(vl(Loa{!zfg<pT!WZI_h;vPv<99YG(7oq<NGm&F8@-#*)#PY271yD+Wlpz&Y$X-*PdzDYR=IbJb?=bEcUlRdy2adD(|RA8+Nk@`VMsPlzWp9FLoxTDr_xo#xUs_zdmq|U7i!v%+MPC&{3J}e%&GDrSwCiCFYl>jIs;2YwH#pTLX-i3o;+WP!kWe^Xl{BM4=71R%BQ*ysBE>>x*q3i?bca|>9MGQ0i=yZ&9#-k+0ulQ#-9Is3l)Cj>qA6^vv5f!;=ltdm&^*|#Ygj@(VME9y##eC6^(xM0uZ>BjyuB^?6cNQ52{0wfvQgg8Z_vAgg=Z!DRn-aKyDqj*O~T0B3e0luF;iWhPPRo>uoxCfxy&_G&MDcQ~9=nxz<Q!#$Hm}nbKyB{GLsMRpTIS?!~^!A{H>k-DcI94bF|G=YPb7I-M~MwFGhE^qKM{X9<N(9<@~_kNc_XlHTc$7+v4sGGPRls~i`oBD~0Dw-Dl?gqF*H&$IUid&^xw#hc$pW2Z)434A6Fz96VEg`NgcXpWC>6r$#}LEi#PWi}R=FAlrmUupDIo;%GjbNH`TBo6Ah*4Meet$G{#8>u)#D>ipJG7^}Q)oyL4CEp@dPJ(NDbr95!69MW;y9cj06;W%Ea%)5`NKkOLqw5UdogA0{!t^8GoV~NEb9tvwME;uC_kwR;N*-9T35I0T%!U9QpASk0u&?CsSAi)2yi7(4L~Kv0a_`)$-3c{u6Vl{;y|lk=cI*ZUD4V{x!EQ5N+fWH_YLBgXf)1}~E3UVgILf>3C#iw)wYNVpXRZU#FIeX_(={aMAO|!t6*F^Q#2wa24dX};Bfv)_o(EEg4X*vT4r$nvy82bbIt|+$d^0SA_7*+etf;L+L?Iz?4L|2DuvP4oG-TghVC~n>)DgU%xCVxs4uJn)6{zzfQD>VjvSR@()M2nqN*L#0{T^C(`3JuihUPTKz}I_mEbsT0cW~|Agi{4+ZEL%p`ivPTE+>9SZ=7L?D!M(&M~T*6G_H>R^!le;r{23WYU`z^qQ^1d)Bgo6GD90E1P0Y{umfZ0gWid_A{VxH*kE|Ep`5W{W;n!%wo&Z)%~1ijHfI(h4Q6WN0&INzEikSeD#`_0!(jwAj(iVy!crZ-%Arz()?3aG)ic{Tk#ZG5&t6?PQa0pMZ0~VaeD<}aN9$tksnCG}?AiE)7uLJv!4FchB~Uk-B7GT3X4rVE8PcH=`4t&r>4FhcLW;`aw7*rhkj*A*-fxj9v!SaM{V14?FVn;hWd8R}!lauwy2Z4r4cLBFYuPGN5ZxwzYM#mD5`D$hS_8+l+lKPoY%T3pZKq&TSU9^+C4Kl3>;UYUI`z5noeYx+Nv&2JeAmEzu*~-Eby$t?_6KXrB%ou{ju^euR_d(NO64tLtS4~c+!DZ-&cXSBZO*tKolMzL!$P$860eSFH?iPVYH{5pBtDf!ITcciB=Nk(XIkhrqvmaH*t1Vys-O0wvwn2hkFKW9t{q$ITk){b1%_)oqr@=bH;JZQ=IZ1c@lZh45`u!#F$~_G2xhYt&}>jIz2G5gy>T*J#Un&By-hbX$QO#H-Ba9cGBsyJ%IVS9XT}676cRR|S|rs=P@Gv5mxSW%=<7=#1;PZ-D(&alurD=ignN1P_0>MoQ6wUS<U_VkLj~9(;Mh&jK4eKve0n*)%Lcxrj=p*Iwu!7(S9M-8Fo$6lgfk|!@SuklL`6?=VJ;811BGO2)h#o+NG9wPy<=00mmdP$Qu#IJG8tohDLn4efQ8w((M`ZS-~c#RciIt8XG!Bk)hbiL(u?d>qt|&@Vm3Nzz!!6L{Nw<dHWUtj`%xPMpNR$alA^uOz+u|H2m?-I54e?QKMD!VK(_vzlB~`9Xi8A5td3=8(`jy{;-&4@<<j10jyr*+%69EXwkb*)xneCh?V!Qdo%f>)7{*H=k<n`d1TJNkZ*g-f`>0Zyd-P3QxK$;vw+=iha>kB@e_7SSKElgBM+yetR28>W9c=VWuv5ao1BALys7NWh5F4Tv_sw-nA#P@kku%>&H%3E99gPMl#SyhJ@2wi>-aS?eDNKvJ&2Q+TpYAnQ<e7`cyM5dOvvFq2uuUA<@!@H%I1eSb@ico6H}IH(6PnZhrh2l0TlnEO2Z~NWq_bR|MdL!TspzWdUO@I*ue?g^f#jy%E#`)|XA=U1gN2D;9^%kc_M7-~v~lCXR#FB@=<y&ts6hAd$&N=5W|B!PT<0LjHq~CHJQi|~An!5mYOjH<Gmd4;txscH<U&xwFn3|x?TOf;INKCE8hSCR?Fh-DrFq&GEmil;SqAc%BQ<*i9FYN0*8P@el`bvyjf?wjXyv0)@_B21OUWBq&x_6|IuAc8)uPP$+}TGXEkRpfhHp=-TMU9f4;<65ub(?PoNk_B5gKA{GzW$-Ik<~gaN9;~-U}m!<E2wvg40NY6@-K-*kc^*CeKbYhnyz$j*V@h2!S1<#kWG1l-7CuY(@nv5@)V{E=euiVT<nIL?F)@KSd^zh4PO*rH=o6`>(Hmh+cpH{q47}U;TV5Losh}zq|eBXS)(9Z|N^@e*E6WA%;>#|MAz`H@CjUy7`j?{P){8umAF&(QP_DI~!feMId_h!?&XE0J$<SEV=npbo<@g+vv@Y-+lMzSKs`5fLO^b-)aU3N6?C5%y3_2^`^LyXG`&`w{LIX{2ZA(9N0scw<!As58Zt2#8jA8QeXc-Aoay%zE9uv*xVN*=>>Y*xZ<wJf7xYx(%Y5c$3q}mW$2sBzDJt_>~BAP3wjgc>)TsL9yfpTB?9tn@SZfl>E-D}K%bP1VCLad7v0_sfNACaH$Q&={p+9OprI90MWu7@pIT+MT_uk*V1M}-m|IPqWE~94PR|@6L3fzXn2$=C6ex#{|0gs1pD+tO5i!dnX8C^Uq~N^Z^DOE_w+cweGMkfOsEZr6N1}DP?a1$|1lAAryo0P%HaajE0t;fO)RGQ<7vI%GFWeiAsnhSdnV2RIic?Pz7Tr5@5}hsF01L_the-#(>dp`9wE5k_q7V-yt9~mM8PY>+qN5)BtdJgj-<~ap4l+!gaQNL5X`Yb$Y#U#25oc@U;)ko~pjW}-Q#1cKh)WObC;V2<#zdXQiEW=>=$s5ab;63^@5DG`E^=Ah4~e0<<6_kd4A^W5C$V21lCNrkTU}C0j6WKJ@o%W8GXtuev<})_fpQ<)>T6xcjfXj3+ygSar`9l>&2$LL9_MneV`)&Z8zJV+0X))mJmge+NbWhy&jsH)hxQF%Bw+6$JunB`IK7`XHR<@RLA(k#<0gFgs%51zFj`?ab9!n{rd?U<`c^ou)o6g>Ro+5TOPQnOBdz3I`48#RXll%PNE9s?_6S75as;BGgapE=4>QUI!L%c9jfwbouf;-5#@Ei>6SguzJe$>Oo-E$0wVqyOp_P??hjLLrOSfrqGUUMAY(??1c`4!F`^mQr9{7r@FUITflj7!uRk~JK4drz~>HK#KX-jz<Yx*k?p0Bbf(GxI!%H;ZJ6mnMbLMH;MZLs6C?1$NOWab$@^EaDkcYJd+S=QiNIy*Bd)6%*e)IHRkB*zVP1I6;Zz>K(Nf9cShM!M1u`LqEcwBXFxo@#3|<3v1)iSlS!0!<Lg7OGY-H^SZ(F*VZZGtSjgBVJfIuk3nlrOthIS~dOM9gckCpG?%PS~K*yZF?|Q_zO*63`U4)E(fBqrk&$IIXfi@*4IW4nlC)jv&=AH>iU29FBI%$rnTJ?V6IGBh!m63iec{@*YMi78HCuJ>P>)c+z0}64iZ984VyIhRuPN1>cQX~i_K8cFxgG^x6{!G%UKHc2QGs|i0#f^^sZvL5bg0Y&-m(>6`q0VwJ;8%XMmB{Xx}E<H|2PbDNL8jA!bZ=b<%qp)EI{f&dI{}UKWqDIi|ep*uJ?h5TBjSPfnJj;b?R=KfgLzoQ_9V%k<*pe7rbcEXIpuk(>=jr^&_qY?Pg!r=!!W%k<)6K0h5_F3+;lSW&jTTnsNR#@X^>cy)4>EKWxA(Kx-hO0Sj|SBuljWS*wO#d&g)WoN^a^lCJ|T%;Gni}YfYoeWQ7H_p&}7rf=j78{2KsEzP$wNmx;*D8*NXOTGga*;mgWVT6I%bDh(_sN~myJ{SXliGfmA@<SnL+8G~0XgD{bf!$_qBa=k9aYKKrS<JKpMw_o^EfD;TR%jsVW0-VUMi(LlkM!(I{DkS@Y5CcsGvU3Ydgd{Yd^%>o6sOm?sQ^9kUsGzl)em)z!LDHnS^%_7TH$*dG5hC_`q@c6aS(=LE&@G@Rt4tAnS!+Yj$d`s_Sa+O@^lQO~Yvek_kU5)p2kC7=RTNu9Mk%SF86n_eL%4qscpbfy~JZ7BAm}H~PNVd~lvhbR)0y*WGN$&aG6DKm)|otCHC=&sL5P?f+E>uOawYD|+9{ypUzy;Bwxn)DOz(*NMz|dh?-=W;l|K4J_PcGE=wIoUCI^&eAFt;E&f1ocYSd)Y4=hC^t8FDLJa%JZdnC-T>5(p3ms{Hvu`2-{a!f=7-fP(rVSvMwJ*XtXH1e?1IB~6TjxEy^cjk`eG3XrETjB3~Xy(q3?wOv}Q6vdpG&W8ACskK%6QaD$?dS7r`abXaDQ_UZBcC<BIt67odX+PuofpT47{5l?3diQwflb6e<DM<WZ3vyYcq1zK0R0b%y^$TAr2H(NC}5V&?7t0_K3Da{')))
```

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
