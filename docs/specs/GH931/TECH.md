# GH931 Flagship E2E Public Proof — Tech Spec

Status: Current contract (harness scaffold)
Issue: #931

## Harness artifacts (this PR)

- `eval/coding-bench/conditions.json`: machine-readable condition registry —
  3 primary + 7 diagnostic conditions, per-condition `runner_status`
  (`implemented`, `implemented_as_legacy_id`, `artifact_schema_only`,
  `pending_src_support`), isolation rules, `remem_e2e` forbidden shortcuts,
  and the 6-stage / 12-enum failure taxonomy.
- `eval/coding-bench/schemas/conditions.schema.json`: schema for the registry.
- `eval/coding-bench/schemas/curator-log.schema.json` +
  `eval/coding-bench/examples/curator-log.example.json`: the
  `curated_file_budgeted` run artifact contract.
- `eval/coding-bench/curated-file-budgeted-protocol.md`: curator protocol
  (per-session time budget, character cap, edit accounting, freeze hash).
- `eval/coding-bench/validate_schemas.py`: offline validator — schema checks
  plus cross-field rules (budget limits, totals consistency, taxonomy
  completeness, no legacy `remem`/`curated_file` ids, file references exist).
- `eval/claims/registry.json`, `eval/claims/claims-registry.schema.json`,
  `eval/claims/claim_gate.py`: claim registry with pre-registered #931
  thresholds and the wording gate (verdict enum, forbidden-phrase scan,
  `Directional evidence:` prefix for `INSUFFICIENT`, supporting-report SHA-256
  verification). `claim_gate.py --self-test` runs the embedded unit tests.

All validators are Python 3 stdlib only and run offline. The JSON-schema
subset validator lives in `claim_gate.py`; `validate_schemas.py` imports it
instead of duplicating it.

## remem_e2e execution contract (src follow-up, not this PR)

The Rust runner (`src/eval/coding_bench`) needs:

1. `BenchCondition` id rename `remem` → `remem_preloaded` and `curated_file`
   → `curated_file_expert`, no compatibility aliases.
2. New `remem_e2e` condition: feed fixture history episodes through real
   capture (`captured_events`) → extraction_tasks → observations/candidates →
   promotion policy → memories, then serve the target run via the production
   SessionStart/MCP retrieval path only. Hard-fail if the run plan attempts
   direct memory seeding or gold-evidence preloading. Before any capture write,
   construct `remem_e2e_capture_projection_v1` exclusively from
   `history_episodes[].raw_events`. Its closed event schema allows only
   derived `source_ordinal`, `event_id`, `timestamp_epoch`, `role`,
   `sanitized_content`, `tool_name`, `sanitized_tool_input`,
   `sanitized_tool_output`, and `host_boundary`. The adapter flattens
   `history_episodes` and each `raw_events` array in their literal registered
   nested-array order and assigns one projection-wide, contiguous
   `source_ordinal=0..N-1`; fixture-supplied ordinals are rejected. Every
   projection key is required: IDs are unique opaque
   `evt-` plus 32-lowercase-hex CSPRNG values, timestamps are signed 64-bit
   epoch seconds, role is `user|assistant|tool`, content is a string, tool
   fields are string-or-null, and boundary is
   `user_message|assistant_message|tool_call|tool_result`. The unique v1
   combinations are: user message = `user` + non-empty content + three null
   tool fields; assistant message = `assistant` + non-empty content + three
   null tool fields; tool call = `assistant` + non-empty tool name/input +
   null output; tool result = `tool` + non-empty tool name/output + null input.
   Tool-event content remains a required string but may be empty. Events are
   non-empty; timestamps are nondecreasing by `source_ordinal`, and equal-second
   events are valid. Ordinal is the sole canonical order: `event_id` never
   sorts or breaks ties. Projection preserves literal source order without
   filtering, reordering, deduplication, or merging. Unknown fields,
   non-contiguous ordinals, decreasing timestamps, or reordered IDs fail.
   Task/episode IDs bind projection hashes only in the outer manifest and are
   forbidden from projection bytes and capture identity. The registered
   capture-identity block instead supplies the closed production host DB value,
   a nonsemantic `e2e-` plus 32-lowercase-hex CSPRNG session ID, and the exact
   fixed `/workspace/remem-e2e/project` Git-root mount for both `project` and
   `cwd`; the mount is identical across tasks in separate isolation
   namespaces. Event/session IDs and paths may not derive from or contain
   task/episode IDs, prompt/gold tokens, or ambient paths. Authority attests
   CSPRNG generation, and the
   verifier enforces formats, the fixed path, and forbidden-token absence.
   Episode `summary`, `expected_memory_facts`, `memories`, gold/supporting refs,
   target prompt, score/hidden/oracle/scorer metadata, and every non-raw-event
   field are forbidden projection sources. Canonical projection bytes and
   SHA-256 are sealed before provider or target work and recorded in the run
   artifact/source manifest. Projection and call content use RFC 8785 JCS
   UTF-8, retain every required key including nulls, and hash the exact encoded
   bytes. Call content is the closed object `host_boundary`,
   `source_ordinal`, `sanitized_content`, `sanitized_tool_input`,
   `sanitized_tool_output`.
   Before DB mutation, the adapter materializes a same-length ordered call
   plan whose index equals `source_ordinal`, applies the production capture
   redactor to every encoded content value, requires byte-identical output, and
   binds its SHA-256. Each event maps to exactly one
   `record_captured_event_with_id_and_reference_time` call:
   `event_id_override=event_id`, `reference_time_epoch=timestamp_epoch`,
   registered host/session/project/cwd, and event type from the fixed v1 map
   (`user_message→user_prompt_submit`,
   `assistant_message→assistant_message`, `tool_call→tool_call`,
   `tool_result→tool_result`). The call uses exact role/tool name,
   `ObservationExtract`, and canonical JSON content
   containing all four sanitized content/tool/boundary channels.
   The isolated connection starts `BEGIN IMMEDIATE`, proves every
   `(host,session_id,event_id)` absent before the first call, then executes all
   calls and verifies the complete call-plan↔new-row bijection inside that same
   transaction. The writer lock makes the existing upsert API insert-only for
   this batch; any pre-existing ID, call/row mismatch, or error rolls back all
   captured rows and queued tasks before commit. Verification includes ID,
   reference time, role, tool, post-production-redaction content hash, project,
   session, and order; call-plan index and inserted `captured_events.id` must
   both increase strictly with ordinal. A duplicate/gapped/shuffled ordinal,
   decreasing timestamp, event-ID sort, or call/row inversion rolls back before
   commit. Worker drain starts only after commit. The independent
   verifier rebuilds projection/call-plan bytes and compares both hashes and
   all rows. Missing raw events, cardinality/order drift, hash mismatch, or a
   forbidden field reaching the plan fails before the first write. No
   summary/gold or partial-event fallback exists.
3. `remem_e2e` requires a configured remem LLM provider key at execution time;
   `--dry-run` must not require it.
4. `curated_file_budgeted` run support: inject a curator-produced `MEMORY.md`,
   verify its SHA-256 against the curator log artifact, and attach the log to
   the run artifact.
5. Report additions: failure `stage` attribution (6-stage enum), curator
   maintenance metrics, and paired task-cluster bootstrap statistics.

The ContextAudit binding slice is implemented independently of the remaining
flagship runner work: remem-backed runs execute the production SessionStart
emission path, resolve its persisted `injection_run_id`, embed the payload-free
canonical audit plus version/hash/count/budget summary, and recompute both the
audit hash and injection-run binding during verification. Missing evidence is a
runtime contract failure; control conditions are explicitly not applicable.
The same contract is required when `remem_e2e` execution support lands.

## Completion implementation contract (pending)

The implementation must preserve these fail-closed boundaries:

1. Canonical identity includes `run_phase` and `matrix_namespace`.
   `issue385-v1/official-v1` contains the only 144 claim-bearing keys; reviewed
   smoke policy entries derive disjoint namespaces that cannot be promoted or
   passed by the caller. All phases reserve against the same authoritative
   ledger and cumulative cap.
2. The production-clock tranche inventories semantic capture/candidate/
   promotion writes plus the complete SessionStart, PromptSubmit, visibility,
   temporal/fact, graph, usage, rerank, staleness, audit, and explain graph.
   Normal adapters construct one system evaluation clock; the benchmark uses
   registered `evaluation_as_of` through every layer. Its MCP router exposes
   only `search` and `get_observations`; search has no raw fallback and detail
   accepts only same-connection-issued `source=memory` IDs. All other tools are
   rejected before DB access.
   Focused tests and a static check reject `Utc::now`, `SystemTime::now`, and
   SQLite `strftime(..., 'now')` when they can affect target-visible selection,
   ordering, labels, or access feedback. Approval expiry, cost accounting,
   timeouts, and supervisor monotonic duration use a separately inventoried
   real security/operational clock and cannot consume the virtual clock.
3. `src/eval/coding_bench.rs` declares each child only in the same compiling
   milestone that creates it; no handoff leaves a missing module or stale enum
   consumer. The default-branch
   approval binds the OS-anchored host supervisor plus exact executable/profile/fixture/registration
   hashes, tuples/namespaces, pricing and hard caps, the sole ledger-writer App
   identity/signing key, exact update-authority and no-bypass integrity
   rulesets, and pinned Sigstore TUF/Rekor trust material. Unknown, duplicate,
   expired, unmerged, unsigned, wrong-writer, rolled-back, or drifted inputs
   fail before provider/host/agent work.
4. The ledger writer signs canonical envelopes over the prior head, sequence,
   policy, execution identity, originator role, and event digest. After each
   non-force remote compare-and-swap, it submits a digest-only DSSE checkpoint
   to the Sigstore Rekor public-good log and verifies its inclusion proof,
   signed checkpoint, consistency proof, operator/log identity, and strictly
   increasing log index before accepting the transition or dispatch. Active
   shard URLs and rotated keys come only from approval-pinned TUF
   `TrustedRoot`/`SigningConfig`, never a hard-coded URL. A fresh clone trusts
   neither its local ref nor the current GitHub tip without the verified Rekor
   bundle chain. This rejects rollback/proof inconsistency relative to pinned
   and previously observed checkpoints; without separately approved
   witness/gossip evidence it does not claim detection of a malicious Rekor
   operator's self-consistent split view. Receipts therefore fix
   `view_assurance=operator_consistency_only`.
   A terminal flagship run first freezes a receipt-free immutable RFC 8785 JCS
   payload. It excludes the terminal attestation/checkpoint, source-manifest or
   report hashes, and every field derived from its own payload digest. The
   supervisor computes `payload_sha256` and CAS-seals it before any receipt
   exists. After sealing, `source-manifest.json` stores a detached
   `(matrix_key,attempt_id,payload_sha256) → (ledger attestation OID/digest,
   checkpoint receipt digest)` mapping. Verification recomputes the JCS bytes
   and then validates the payload digest, ledger seal/signature/ancestry,
   checkpoint proof, and detached mapping in that order.
5. Hidden scoring runs under an independent scorer OS principal, process, and
   read-only tree after target teardown. The controller never imports or
   executes patched code. A separate untrusted code worker has no hidden mount
   and exchanges only size-bounded, closed-schema RFC 8785 JCS JSON RPC over
   supervisor-created pipes. Only scorer-owned validation bound to the exact
   patch/tree and hidden-rule digests may produce PASS; stdout, exit zero,
   visible tests, or a worker-asserted result cannot. Shared interpreter state,
   monkeypatch reachability, extra/truncated RPC fields, crash, timeout, or
   exception fails closed.
   The registered scorer-only `memory_harm_rules` closed set deterministically
   classifies every `remem_e2e` tuple as `memory_caused`,
   `independent_cause`, or `no_wrong_action` from sealed evidence.
   Zero/multiple matches or incomplete traces make the gate `INSUFFICIENT`.
6. `remem bench coding --verify-live-approval-only` validates the real
   default-branch policy and exact plan while performing zero agent/provider
   calls. Live smoke, official execution, and report recomputation use the same
   no-symlink handle from a read-only content-addressed mount; the supervisor
   hashes and executes that same handle on every invocation. The
   fixed root-owned supervisor obtains digests from authority, performs
   `openat(O_NOFOLLOW)`, same-fd hash/fstat, and Linux
   `execveat(AT_EMPTY_PATH)` or a reviewed equivalent, then signs a
   caller-unforgeable attestation; unsupported platforms fail closed. Claim-bearing
   report recomputation uses `remem bench report --root ...`.
7. The target has no public network. The pinned Codex main process alone may
   reach a private loopback provider adapter with no network; that adapter
   forwards fixed-schema frames over inherited pipes. Tool subprocesses cannot
   reach loopback/Unix sockets, and a feasibility test blocks unsupported
   agent/platform combinations.
8. E2E events use stable IDs and production `ObservationExtract` tasks. One
   normal worker drains only ObservationExtract → MemoryCandidate →
   GraphCandidate to quiescence; exact replay, unexpected/residual/failed
   tasks, SessionRollup/UserContext/background jobs, and native-memory effects
   are invalid for this registered adapter.
9. The report builder validates all 144 keys before aggregation. For each
   task/condition it computes the arithmetic mean of exactly three binary
   `resolved` values; target-started timeout/crash/score failure is `0`, while
   any pre-target missing or integrity-invalid tuple makes the matrix
   `INSUFFICIENT` without imputation. A primary point estimate is the mean of
   16 within-task treatment-minus-control differences. Each fixed-seed
   percentile-bootstrap replicate resamples 16 task IDs with replacement and
   recomputes the two three-run means and paired difference inside every
   sampled cluster. Pair/hash absence is `INSUFFICIENT`; the registration
   freezes algorithm/version, replicate count, seed, and 95% percentile-index
   rule before official execution.
10. `remem bench report` defaults to a fully offline path. It verifies only the
    execution-time receipts/proofs carried by the governed bundle and explicitly
    reports that current authority freshness was not checked. A separate
    `--verify-current-freshness --freshness-receipt-out <path>` invocation is
    explicitly networked and emits, without rewriting report bytes, an
    authority-signed detached receipt binding `report_sha256`, ledger tip,
    ruleset digests, TUF metadata digest, Rekor bundle/checkpoint digest,
    `observed_at`, and `expires_at`. Publication, closure, and release gates
    require an unexpired receipt for the exact report. Network denial,
    stale/expired receipt, wrong report hash, or any tip/ruleset/TUF/Rekor drift
    fails closed and leaves the report unchanged.

Detailed task ownership and negative-test expectations are retained in
`specs/GH931/tasks.md` as planning evidence. That file does not replace this
current contract or mechanically authorize implementation.

## Validation commands

```bash
python3 eval/coding-bench/validate_schemas.py
python3 eval/claims/claim_gate.py check
python3 eval/claims/claim_gate.py --self-test
cargo run -- bench coding --suite issue385-v1 --dry-run \
  --json-out /tmp/gh931-dry.json | tee /tmp/gh931-dry.txt
grep -q '^planned_runs: 144$' /tmp/gh931-dry.txt
```
