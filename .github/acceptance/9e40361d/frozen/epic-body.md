## Goal
Implement the concrete correctness and integration fixes confirmed in the 2026-10-06 review of main `3722f8083d08774a58d5234ceda817ec3951b7d7` (0.6.101). The owner requested implementation after reviewing the findings. Memory quality, automatic capture and governed provenance remain the primary requirements.

## Confirmed work
- [ ] Align supported summary promotion with current-context eligibility without inflating model confidence or weakening external-content controls.
- [ ] Preserve complete embedding profile identity in derived sqlite-vec indexes, backfill/readiness and pruning.
- [ ] Apply allowed scope before nearest-neighbor candidate selection; prevent nonempty partial KNN results from hiding relevant scoped memories.
- [ ] Remove the remaining recency-only 4096-vector window from automatic injection by sharing a correct bounded candidate generator.
- [ ] Distinguish recoverable model-output errors from permanently invalid source evidence.
- [ ] Bound each extraction input and preserve exact processing coverage across backlog/retries.
- [ ] Allow new evidence for a pending same-content candidate to receive a governed reassessment while preserving rejection/quarantine/suppression.
- [ ] Prevent lower-priority memory stages from indefinite starvation under continuous capture.
- [ ] Correct Windows worker liveness and normal-exit handling.

## Existing work to reuse
#1100 / PR #1101 (terminal worker diagnostics), #1102 (attempt usage evidence and missingness), #1103 (conservative workstream identity), #1104 (selected Codex home).

## Evidence baseline
Two vector findings were reproduced with native sqlite-vec 0.1.9 and synthetic in-memory data: same-dimension models reverse expected rankings and batch sync violates the mirror primary key; global top-k before scope filtering misses an eligible scoped result. Production SQL fixtures also reproduce the 0.74/0.70/0.80 promotion/visibility mismatch, the 4096 recency window, same-content/new-evidence suppression, and strict-priority starvation. No production database, real host CLI, or model was used in the audit. Implementation must add native Rust regressions and report their actual execution.

## Delivery contract
Use scoped commits and reviewable PRs. Update current PRODUCT/TECH contracts before cross-module implementation. Keep this epic open until all checked items have implementation and verification evidence. Existing #931, #933, #934 and #935 research/rollout programs retain their own acceptance and public-claim gates; this epic does not claim those programs complete.

## Implementation and verification update — 2026-10-06

Implementation is tracked in draft PR #1106, stacked on #1101 against
`fix/terminal-job-failure-diagnostics`. The source version is 0.6.103 with
unreleased synchronized metadata. The nine original checkboxes above are
preserved; code presence and earlier focused tests do not by themselves close
this epic.

A–I below refer to the original nine bullets, in their existing order. Source
paths are relative to the repository. Their implementation was established in
historical H and retained through T/E/F. The new clean producer source P also
contains the graph evidence dependency/registry correction and the report from
its actual evaluator run. Final acceptance applies to evidence-only review Q.

H/T/E/F remain historical identities: H produced the earlier native evidence,
T corrected a Windows fixture, E imported H's evidence, and F corrected two
integration fixtures before its additional eval run found stale graph evidence.
P/Q require a new source tree and new native evidence; H's 2b3 tree cannot
substitute. P is now verified as `47e4ecdeeda81557e644f303e68bec208a8defca`; Q does not
yet exist because the new native evidence has not been generated. These source labels are separate from the A–I audit item labels.
### Current checkpoint and blocking operation

The clean producer source is `47e4ecdeeda81557e644f303e68bec208a8defca` on
`audit/native-evidence-20261006-47e4ecde`. Its remote branch ref has been read
back and confirmed. The review branch for #1106 remains at historical H
`5a9336898905a5d5d884c77bd457fddda609f1a0`; it has not been advanced before
final local verification. The graph correction and complete prior work are
preserved in the producer branch while new evidence is pending.

The installed GitHub connector does not expose workflow dispatch. The normal
browser fallback failed before opening the workflow tab: automatic approval
review could not initialize its session because the Habitat thread-store
service returned HTTP 429. The requested browser action was not executed;
this was an internal service error, not an unsafe-action verdict or refusal of
authorization. No workflow was dispatched, and no approval check was bypassed.
A normal dispatch route must be restored, or the owner can start the existing
workflow for this exact frozen branch:

```bash
gh workflow run native-benchmark-evidence.yml --repo majiayu000/remem --ref audit/native-evidence-20261006-47e4ecde
```

After the four platform jobs and aggregate succeed, authenticate the artifacts,
import their unchanged producer identity and payloads as Q, and complete all Q
local gates before updating #1106. Then require and record Q’s actual remote CI
results before acceptance. Keep the epic and original A–I acceptance checkboxes
open while those results are pending.

### A–I implementation and regression map

| Item | Implemented behavior and retained boundary | Main files / current contracts | Critical regression coverage |
|---|---|---|---|
| A | An eligible automatically promoted summary can establish current-context eligibility through its exact immutable activation receipt and current source evidence. Re-evaluate the original promotion gate; retain external trust, validity, poisoning and suppression checks. The generic G2 confidence floor stays unchanged. | `src/memory_candidate/current_proof.rs`; `src/truth/visibility.rs`; `docs/specs/summary-promotion-gate/{PRODUCT,TECH}.md`; `docs/specs/legacy-unverified-context/{PRODUCT,TECH}.md` | `src/session_rollup/tests/summary_current.rs`: real production activation reaches CurrentTruth/SessionStart; receipt, candidate and source tampering, suppression and lifecycle changes fail closed. |
| B | Partition derived vec0 mirrors/state by the exact model/artifact identity plus dimensions. Use the same profile for sync, backfill, readiness, cursors and pruning; canonical `memory_embeddings` remains authoritative. | `src/retrieval/vector/vec_index.rs`; `src/retrieval/vector/backfill.rs`; `docs/specs/local-semantic-embedding/{PRODUCT,TECH}.md` | `src/retrieval/vector/tests/profile_scope.rs`: `same_dimension_artifact_profiles_coexist_through_sync_and_prune`, independent atomic readiness, and rejection of the legacy dimension-only mirror. |
| C | Apply caller eligibility and profile membership inside the vec0 candidate restriction before k. Rebuild/unavailable-index paths use the same scope in exact scanning. | `src/retrieval/vector/scope.rs`; `src/retrieval/vector/vec_index.rs`; `src/retrieval/vector.rs`; local-semantic-embedding contracts | `knn_prefilters_scope_before_foreign_neighbors_consume_k` in `src/retrieval/vector/tests/profile_scope.rs`: more than k closer foreign/ineligible rows cannot displace permitted project, branch, type, lifecycle or suppression results; indexed/exact agreement. |
| D | Automatic SessionStart/UserPromptSubmit vector retrieval shares the corrected candidate executor and no longer truncates candidates to the latest 4,096 rows. Keep hook embedding/network policy, result limits, fusion weights and downstream governance. | `src/context/hybrid_context.rs`; `src/retrieval/vector.rs`; local-semantic-embedding and `docs/specs/GH953/{PRODUCT,TECH}.md` | `src/context/tests/engine_convergence.rs`: `injection_vector_retrieves_match_older_than_4096_newer_rows`, weights/distance/usage propagation and existing eligibility behavior. |
| E | Typed `ModelOutputError` separates generated-output parsing failures from permanently invalid source evidence. Existing retry limits and resolved failure classes remain authoritative; malformed output does not partially persist a result. | `src/db/failure_lifecycle.rs`; `src/observation_extract.rs`; `src/memory_candidate.rs`; `src/session_rollup/mod.rs`; `docs/specs/failure-lifecycle/{PRODUCT,TECH}.md` | `src/memory_candidate/tests/generated_output.rs`: malformed output can retry successfully; repeated failure exhausts the existing cap; unwrapped malformed source remains permanent. |
| F | Admit a contiguous same-host/project/session prefix within 64 events and the 256 KiB request budget, including a 4,096-byte wrapper reserve. Publish its checkpoint only after required effects complete. Replay members retain original range identity and resume their own verified progress within the shared 420-second family deadline. Preserve raw archival evidence on prompt rejection. | `src/db/extraction/{input,progress,replay_member,exact_family}.rs`; `src/extraction_worker.rs`; `src/session_rollup/persist.rs`; failure-lifecycle contracts | `src/db/extraction/progress_tests.rs`; `src/session_rollup/tests/bounded_input.rs`: chunk/retry/timeout boundaries, parent-success/child-failure recovery, foreign/expired checkpoint rejection, no inferred legacy success, later-range coverage, and idempotent raw-archive preservation. |
| G | New trusted evidence can reassess an untouched automatic observation/summary pending candidate through an audited replacement. Preserve the old payload/evidence and replacement relationship. Human decisions, quarantine, suppression and reduced trust/confidence prevent reassessment; preference reinforcement stays separate. | `src/memory_candidate/reassessment.rs`; `src/memory_candidate.rs`; `docs/specs/candidate-auto-promotion/{PRODUCT,TECH}.md` | `src/memory_candidate/tests/reassessment.rs`: replacement transaction/rollback, repeated or reordered evidence idempotence, human vetoes and TTL renewal requiring new trusted evidence. |
| H | Persist first-ready and last-claim service order across ordinary queues and stage/host/project groups. Claims and ledger updates are atomic; reopening retains position, failed/empty claims consume no turn, and exact replay stays separate. | `src/db/worker/dispatch.rs`; `src/db/extraction/lifecycle.rs`; `src/worker.rs`; failure-lifecycle and `docs/specs/GH969/{PRODUCT,TECH}.md` | `src/db/worker/dispatch_tests.rs`: continuous arrivals, retry/reopen, eligibility, rollback, concurrent claims and replay isolation. `src/eval/bench_artifact/verify/security_snapshot/tests/dispatch.rs`: accept only the actual task-bound 2/3 ledger rows; reject 11 SQL mutations and hidden columns while retaining full typed replay comparison. |
| I | Combine heartbeat freshness with actual process liveness. Windows uses a process handle and zero-timeout wait. An owner/PID-scoped exit guard retires only its own heartbeat and retains historical timestamps. | `src/db/worker.rs`; `src/worker/heartbeat.rs`; failure-lifecycle contracts | Worker current/dead/stale PID, ownership, once/daemon and exit/error cases. Actual H Windows CI passed the native `db::worker` filter, 18 passed / 0 failed. |

### Related implementation issues and CLI follow-up

| Work | Implementation / files | Regression boundary |
|---|---|---|
| #1100 / #1101 prerequisite | Terminal job transitions/logging in `src/db/job/state.rs` and `src/worker.rs`. `TerminalFailure` means persisted terminal failure with no scheduled retry. The later `src/extraction_worker/diagnostics.rs` follow-up belongs to this PR, not the prerequisite base. | Corresponding job-state/worker tests retain the separate PR/base review boundary; extraction diagnostics are reviewed as this PR’s follow-up. |
| #1102 usage evidence | `src/ai/{usage_observation,codex_usage,codex_cli,usage,pricing}.rs`, `src/db/usage.rs`, stats/API/CLI surfaces; `docs/specs/pricing-config/{PRODUCT,TECH}.md` and `docs/specs/SPEC-web-api.md`. Preserve raw/cache/reasoning meaning and report observation, attempt and cost coverage independently. | `src/ai/usage_tests.rs`: observed zero vs missing/partial/invalid, contradictory subtotals, failed/empty/timeout calls, stdin backpressure, outer cancellation, split JSON events, exactly-once accounting, and unknown auto-model pricing. Estimated cost stays incomplete; named-model rates are unchanged. |
| #1103 workstream identity | `src/workstream/{identity,matcher,write}.rs`; `docs/specs/workstream-identity-continuity/{PRODUCT,TECH}.md`. Require meaningful unique identity and abstain on ambiguity. | `src/workstream/identity.rs` and `src/workstream/tests/`: broad substring/repeated-token rejection, rename continuity, alias ambiguity and content-session collision behavior. |
| #1104 selected Codex home | `src/host_roots.rs`, install/uninstall/import/doctor callers; `docs/specs/shared-agent-sessions/{PRODUCT,TECH}.md`. Validate and consistently use the selected `CODEX_HOME`. | `tests/install_status.rs`: selected-profile install/doctor/uninstall, invalid roots before writes, and scoped diagnostics. `src/doctor/environment/tests.rs` preserves explicit-path host-probe coverage. |
| CLI startup | Ordered internal Clap construction groups in `src/cli/types.rs` and narrowly checked normalization in `scripts/ci/surface_lifecycle_cli.py`. Public commands, flags and ordering are retained. | `tests/support/cli_startup.rs` executes nine real version/help/nested-command children; Linux uses a 1 MiB stack. H native Windows passed the startup test. Local comparison preserved all 165 help outputs byte for byte and all 198 CLI fingerprints. |
| Windows fixture T | Restrict temporary-`HOME` Claude-profile assertions to platforms where they actually redirect home discovery. Keep invalid-Codex diagnostics running on every platform. Production KnownFolder/profile resolution and the startup regression are unchanged. | `tests/install_status.rs`; exact T SHA and rerun results are recorded in the acceptance table below. Do not equate the earlier 6/7 Windows install run with success. |

### Historical regression evidence

The following focused results were observed during implementation. They are
overlapping filters, not counts to sum. Except for the explicitly named clean
commit, the early log did not record an exact producer SHA; none is relabelled
as P/Q acceptance evidence.

| Scope | Observed result | Binding / retained log |
|---|---|---|
| Summary current proof | 7 passed / 0 failed | Earlier integrated build; `test-summary-current.log` |
| Vector profile, scope and fallback | 46 passed / 0 failed | Earlier integrated build; `test-vector.log`; shared coverage for B/C/D |
| Context engine convergence | 13 passed / 0 failed | Earlier integrated build; `test-context-convergence.log` |
| Typed generated-output errors | 5 passed / 0 failed | Earlier integrated build; `test-generated-output.log` |
| Bounded input / extraction recovery / replay family / replay waiting / progress migration | Respectively 6/0, 50/0, 7/0, 5/0 and 2/0 | Earlier integrated builds; corresponding `test-bounded`, `test-extraction-recovery-final`, `test-replay-family`, `test-replay-waiting-final` and `test-migration-progress` logs |
| Candidate reassessment | 10 passed / 0 failed | Earlier integrated build; `test-reassessment.log` |
| Fair dispatch | 8 passed / 0 failed | Earlier integrated build; `test-dispatch.log` |
| Usage / workstream / selected-home focused filters | Respectively 40/0, 43/0 and 6/0 | Earlier integrated builds; `test-usage-final.log`, `test-workstream-final.log`, `test-install-status.log` |
| Candidate / session rollup / dispatch snapshot | Respectively 88/0, 65/0 and 4/0 | Clean `4c7f0a12d5426a09e1ea85d575ca4a2ba2271c85`; `focused-candidate-4c7f0a1.log`, `focused-rollup-4c7f0a1.log`, `focused-dispatch-4c7f0a1.log` |
| CLI local checks | Install 7/0; CLI units 87/0; normalization self-tests 6/0; full surface guard passed | Same CLI Rust/guard bytes before the local commit; binary metadata stamped the older 4c7 source as dirty. These are focused working-tree checks, not clean H producer proof. |

An earlier full production suite ended with 4,120 passed, 11 failed and 1 ignored.
Ten failures came from test helpers assuming the old global queue order; one
found raw archive loss on transcript prompt rejection. The helpers now bind the
intended task/session/stage/range explicitly, and the archive path has focused
regressions. An earlier migration filter also had 159 passed / 3 failed.
These earlier failures are retained as history, and only the final full run
can establish their complete integrated revalidation.

### Historical H: authenticated native producer evidence

- Producer: `5a9336898905a5d5d884c77bd457fddda609f1a0`.
- Git tree: `ffc01852c30d597f1963ee5fe1910badb33fdf1a`.
- Production-input tree SHA-256:
  `2b3e948c9b1662506baca78e077f1d9d55acc2c686d56fce46099a7f6839c916`.
- Production pathspec SHA-256:
  `d3b28d78b492388f5e01ebf51519f8d5d35775e0db89d1e2f3306f9343875c20`.
- [Native workflow 37471764448](https://github.com/majiayu000/remem/actions/runs/37471764448):
  four expected native jobs and aggregate succeeded. Targets are macOS Intel,
  macOS ARM, Linux x86_64 and Linux ARM.
- All four ZIPs were authenticated to the same repository/run/head and matched
  the GitHub artifact digests. Original receipts, JSON and raw payloads were
  retained through staging.
- The actual Rust verifier on the assembled canonical public root exited 0:
  authority/security PASS, 80 recomputed security runs, zero policy failures,
  four current targets, no missing or stale targets, clean build/checkout
  bindings and `executable_source_equivalent=true`.
- Its closed-target security matrix reported `release.ready=true`.
  GH931 remained `INSUFFICIENT`; this result authorizes no release or live
  quality claim.

The earlier 4c7 four-platform run remains historical and is not the producer
for the historical E evidence import. Import helpers establish path/byte handling only;
GitHub provenance and the actual Rust verifier remain separate requirements.

### Historical H Windows and ordinary CI: distinct results

Both [CI run 37471765530](https://github.com/majiayu000/remem/actions/runs/37471765530)
and [CI run 37472052518](https://github.com/majiayu000/remem/actions/runs/37472052518)
were bound to H (ordinary PR checkout `ff47684705c9853a5e890e0d47594b3171000607`).

| H check | Observed result |
|---|---|
| Windows worker liveness | 18 passed / 0 failed in each run |
| Windows local embedding | All-target check succeeded; 59 passed / 0 failed in each run |
| Windows CLI startup regression | Passed in each run; all nine child probes completed without the earlier stack overflow |
| Windows install/status | 6 passed / 1 failed in each run; the remaining fixture assumed environment variables redirect the Windows KnownFolder home |
| Ordinary public benchmark authority | Failed on 80 old snapshots missing `worker_dispatch_state`; subsequent skipped gates are not passes |
| Overall ordinary H CI | Failure, despite the independently successful native security matrix |

T corrects the unsupported fixture assumption. E imports the verified H
evidence. Neither the H native verdict nor these partial ordinary results
replace final-head Windows, production, eval and integration checks.

### E full preflight failure and test-only follow-ups

The full preflight on clean E
`5b0b7471e8bf135bb0df99b5219c20c2b2d8636f` completed with **FAIL: 29 PASS /
1 FAIL / 0 SKIP**. The failed gate was the production Cargo suite. Its completed
outer targets reported **4,211 passed / 1 failed / 6 ignored**; the library
target itself reported **4,137 passed / 0 failed / 1 ignored**. These totals
exclude nested subprocess harness summaries. Cargo stopped after the fourth
test in `tests/shared_session_roots.rs` failed, so later targets and doc-tests
were not executed. This is not a complete successful production-suite result.
The actual log is `preflight-final-5b0b7471.log`.

The failing test,
`unavailable_explicit_roots_fail_without_importing_other_hosts`, still expected
a scan-summary JSON response when the selected `CODEX_HOME` was a regular file.
The #1104 contract correctly rejects that invalid root before database opening,
scanning or JSON emission. An independent synthetic reproduction on E returned
exit 1, empty stdout and `Error: Codex home is not a directory` both with and
without a pre-existing encrypted fixture. The fresh case created no database;
the pre-existing fixture remained present. The retained reproduction record is
`shared-root-host-file-repro-e5b0.json`.

An initial test-only iteration changed only this integration test
(**30 added / 6 removed lines**) to assert
the fail-fast contract for that one case. It preserves exact JSON/path assertions for every other discovery fault
that executes on the platform (nine on Unix, seven on Windows), as well as all
common empty-table assertions.
Production root validation, database/scanning behavior, evidence bytes and the
remaining fault coverage are unchanged. This fixes a stale fixture expectation;
it does not make the earlier E run pass retroactively.

That **preliminary iteration**, preceding historical F, was remote object
`12838cb87cf635f96041fe04adaa15a8cbb53f3a`, parent E, with local equivalent
`f42f7710a4bea6766a34fc87ab75c5086dbb14f8` and Git tree
`358cb97bf3d6e99282f657275d5f72e808c752c2`. Its actual follow-up checks were:

| Preliminary 12838 check | Actual result |
|---|---|
| `shared_session_roots` | 4 passed / 0 failed |
| `Doc-tests remem` | 0 tests, 0 failures; exit 0 |
| `vector_benchmark::vector_search_10k_candidate_gate` | 0 passed / 1 failed; exit 101. It scanned 10,000 eligible rows, returned 10 results and took 262 ms, then failed its old `scanned <= 4096` / `scanned < 10000` assertions. |

The vector failure is a second stale test contract: the approved B–D exact
fallback deliberately covers all eligible rows. Restoring the recency cutoff
would reintroduce the retrieval defect. The later F iteration therefore combined the root
fixture correction with a meaningful deterministic vector regression:
`vector_search_10k_exact_fallback_preserves_oldest_best_match` in
`tests/vector_benchmark.rs`. It retains 10,000 rows and 768 dimensions, uses
explicit orthogonal unit vectors with the unique best match at oldest ID 1,
and fixes embedding timestamps so that ID is oldest in both recency orders.
Removing only the fixture's derived readiness state makes exact fallback
explicit regardless of whether sqlite-vec is registered. Assertions require
10,000 source embeddings, a non-disabled result, the exact-scan phase, 10,000
scanned candidates, exactly 10 distinct hits, ID 1 at distance 0 and all other
returned hits at distance 1. No model or CLI produces these vectors, and elapsed
time remains diagnostic rather than a new performance threshold.

The amended historical F changed only `tests/shared_session_roots.rs` and
`tests/vector_benchmark.rs` relative to E (**70 added / 15 removed lines**).
The original root validation and all other fault assertions remain intact.
No production input or imported evidence changes.

- Historical remote F, fetched and verified clean:
  `0ec3164b5619d4366c71a2df32c87723a6bd6330`.
- Parent E: `5b0b7471e8bf135bb0df99b5219c20c2b2d8636f`.
- Local equivalent: `a77163ad9e7740128a5f780f542462e14a79ab18`.
- Git tree: `d9dda917ddd80d2839a447a309d26658cd2d0589`.
- `source-proof-final-0ec3164b.json` confirms the production-input tree,
  production pathspec and suite identities all match H/E.
- F's focused targets, default `cargo check` and public-claims self-test passed
  as recorded below. Its additional eval library subsequently failed; e2e and
  the standalone committed-root verifier did not run, and full F preflight
  never started. F has been superseded by the required P/Q evidence refresh.

### Historical F eval failure and the new P/Q boundary

On clean F `0ec3164b5619d4366c71a2df32c87723a6bd6330`, the eval library
completed with **915 passed / 2 failed / 0 ignored / 3,528 filtered** in
**5,581.30 seconds**. Cargo exited **101**. The two failures were:

- `eval::graph_decision::evidence_fingerprint::tests::checked_in_graph_decision_report_matches_generated_fingerprint`;
- `eval::graph_decision::evidence_fingerprint::tests::migration_bundle_matches_runtime_registry_structurally`.

The `e2e_eval` target was not executed after the library failure. The
`committed_public_fixture_passes` regression belongs to the eval library,
not to that skipped target. Separately, the additional runner skipped the
standalone committed-root Rust-verifier CLI, and the full F preflight was
**never started**. The 915 passes are one completed library target's result,
not a complete eval command or final acceptance. Preserve the log as
`eval-tests-final-0ec3164b.log` and the runner's actual command/exit record.

Read-only recomputation found 11 stale source entries among the report's 151
fingerprint inputs; the dataset fingerprint was unchanged. The migration SQL
bundle still ended at v093 although the runtime registry contains v094–v096.
The failures are real evidence-maintenance failures, not simulated negative
cases or permission to drop a strict guard.

P corrects `src/eval/graph_decision/evidence_fingerprint.rs` with the narrow
25-line addition reviewed independently: 11 actual implementation dependencies,
three registered SQL paths and corresponding existing coverage assertions.
The implementation file count changes from 149 to 160; SQL migration count
changes from 93 to 96. The newly bound files are:

- `src/retrieval/vector/{scope,search,vec_index}.rs`;
- `src/migrate/schema_drift/invariants/{legacy,v067,v074,v075,v092,v094,v095,v096}.rs`;
- SQL `v094_extraction_completed_progress.sql`, `v095_worker_fair_dispatch.sql`
  and `v096_ai_usage_observation.sql` under `src/migrations/`.

The vector paths are in actual graph-arm search and corpus-insertion calls;
the invariant modules participate in the migrations run for every arm. The
runtime registry order/name/SQL-byte checks and committed-report assertions
remain unchanged. No old metrics receive manually substituted new hashes.
Generate a fresh `eval/graph-decision/report.json` through the actual evaluator:

```bash
cargo run --locked --features eval -- eval-graph-decision --dataset eval/golden.json --k 5 --json-out eval/graph-decision/report.json --json
cargo test --locked --features eval --lib eval::graph_decision
```

Record the actual command exit and report verdict. The CLI writes JSON before
its final gate, so file creation alone is not success. The evaluator forces
feature-hash for all three arms and keeps the existing decision/check/metric
and latency-budget gates. This refresh does not change graph activation or
establish live-model quality. Actual generation exited 0 on the new working bytes under F before P was
committed. The generated report was then preserved byte for byte in P. Its
independent comparison found only fingerprint and actual timing changes, with
no other changed report fields. Working-tree graph regressions passed 6/0/0;
the clean-P rerun also passed 6/0/0. Exact source records are below.

The fixed `eval/production-input-pathspec-v1.json` includes the entire `src`
tree. This source-file correction changes the production-input tree even
though it maintains evaluation evidence. Do not alter the pathspec to exempt
it. H's native 80/0 and source tree 2b3 remain true historical records; they
cannot authorize P or Q. Freeze the new clean P source, run and authenticate
its new four native bundles, then import only their evidence in Q. Q must
have the same production-input tree, pathspec and suite as P. The actual Rust
verifier on committed Q must resolve P's real Git object and independently
check its source bindings, current target matrix and payloads.

### Compatibility and deployment boundary

- **Rust callers:** `AiUsageTotals`, `AiUsageSourceTotals`,
  `AiUsageBreakdown`, `DailyAiUsage` and `WeeklyAiUsage` add
  `coverage: AiUsageCoverage`; `LatestSessionMemorySpend` adds
  `ai_usage_coverage`. Struct literals and exhaustive destructuring need
  updates. Use query-returned coverage for existing data.
- **Failure matching:** #1101 adds `JobTransitionOutcome::TerminalFailure`.
  Exhaustive matches must handle terminal persistence with no scheduled retry.
- **REST consumers:** Coverage fields, including `ai_usage_coverage` and
  `ai_cost_complete`, are additive. Existing methods/paths and cost fields
  remain; strict clients must accept added fields. Absent coverage from an
  older server means unknown. Existing monetary totals are the known priced
  portion, not evidence of complete paid cost.
- **Surface lifecycle:** 75 superseded Rust declaration identities are retired
  and replacements staged. The published `v0.6.82` baseline stays unchanged;
  this does not claim source compatibility or publication.
- **v094:** Nullable completed-event/replay-origin fields preserve range
  identity without inferring historical successful processing.
- **v095:** An initially empty `worker_dispatch_state` ledger introduces
  persistent service order without rewriting historical jobs.
- **v096:** Observation/attempt/cost status and usage detail fields keep older
  rows historically unverified/unknown.
- **Upgrade/rollback:** Upgrade every active worker; mixed-version execution
  is unsupported and older binaries reject newer schema. Restore a
  pre-upgrade database backup with its matching binary, or apply a forward
  fix. Never delete migration markers or fabricate checkpoints.
- **Delivery:** #1106 remains a draft stacked on #1101. No merge, release or
  deployment is performed by this work.

### Historical T/E/F verification record

These results retain their original source identities. They do not certify P or Q.

| Historical record | Actual result / boundary |
|---|---|
| Test-only T SHA / scope | Remote `0b361ba228bea0969c303de5353a779270abafe5`; clean local `80d95b8a1ae6ea3c8368e08cc86f2f542ce78f73`; identical Git tree `94770718bb532c22150111f4949360f3838e2fdd`. Windows integration fixture correction in `tests/install_status.rs`. |
| T production-input tree / local focused result | Production-input tree equals H's `2b3e948c9b1662506baca78e077f1d9d55acc2c686d56fce46099a7f6839c916`. On clean local T, `cargo test --locked --test install_status`: 7 passed / 0 failed / 0 ignored; test execution 0.92 seconds. This local result does not substitute for native Windows CI. |
| Evidence-only E identity / four-group diff | Remote object `5b0b7471e8bf135bb0df99b5219c20c2b2d8636f`, parent T `0b361ba228bea0969c303de5353a779270abafe5`; fetched by SHA and verified as a clean detached checkout. Local E `54120b77e937e4cc015fe45f2d7f331f391a6779` has the identical Git tree `8cb31d0268793a956e01dbea39ec78f77d8eb388`. Four canonical groups, 162 actual changed files; all 568 mapped files hash-checked, including 80 SQLite payloads. |
| E production-input tree / evidence identity | Verified production-input tree `2b3e948c9b1662506baca78e077f1d9d55acc2c686d56fce46099a7f6839c916`, production pathspec and suite identities all match H. H producer identities and the E import identity remain unchanged. |
| Historical E full preflight | **FAIL: 29 PASS / 1 FAIL / 0 SKIP**, production gate failed; `preflight-final-5b0b7471.log`. Completed outer targets: 4,211 passed / 1 failed / 6 ignored; lib: 4,137 passed / 0 failed / 1 ignored. Later targets/doc-tests did not run after the shared-root fixture failure. |
| Preliminary test-only iteration history | `12838cb87cf635f96041fe04adaa15a8cbb53f3a`: shared roots 4 passed / 0 failed; doc-tests 0 tests / exit 0; old vector benchmark 0 passed / 1 failed with scanned=10,000 and returned=10. This precedes historical F and is not a P/Q result. |
| Historical F identity / scope | Remote object `0ec3164b5619d4366c71a2df32c87723a6bd6330`, parent E `5b0b7471e8bf135bb0df99b5219c20c2b2d8636f`; local equivalent `a77163ad9e7740128a5f780f542462e14a79ab18`; Git tree `d9dda917ddd80d2839a447a309d26658cd2d0589`. Only `tests/shared_session_roots.rs` and `tests/vector_benchmark.rs`, +70/-15 relative to E. |
| F clean checkout / production-input tree, pathspec and suite equivalence to H/E | Verified clean in `source-proof-final-0ec3164b.json`. Production-input tree `2b3e948c9b1662506baca78e077f1d9d55acc2c686d56fce46099a7f6839c916`; pathspec `d3b28d78b492388f5e01ebf51519f8d5d35775e0db89d1e2f3306f9343875c20`; suite `56dad240cc175fb3d3900875f05351b9541f9ef845aa54eddfc460151f3e257d`; all match H/E. |
| Historical intended F PR body SHA-256 | `f243151379534d9f26fc4a0edb787ab6168d5e5559e7ae8fb0c736d204be5619` |
| F focused shared-root and exact-vector targets | `cargo test --locked --no-default-features --features local-onnx --test shared_session_roots --test vector_benchmark -- --test-threads 4 --nocapture`: shared roots 4 passed / 0 failed / 0 ignored (7.75s); vector 1 passed / 0 failed / 0 ignored (3.74s). Exact scan observed 10,000 candidates / 10 results / 223ms; elapsed time is diagnostic, not a latency gate. Log: `integration-contracts-final-0ec3164b.log`. |
| F standalone committed-root actual Rust verifier | Not executed: the additional runner skipped this CLI after eval exited 101. No standalone F committed-root verdict is claimed. |
| Full F preflight | Never started. There is no full-F preflight exit code or gate-count result. |
| Complete F production + integration + doc-test suite | Not executed as a complete suite on F. The two focused targets above are the F integration evidence; E/preliminary-iteration counts remain their own history. |
| F default `cargo check` | `cargo check --locked`: exit 0, 27.87 seconds, completed 2026-10-06 15:46:04 UTC on clean F. Log: `check-final-0ec3164b.log`; SHA-256 `914e6bcc38817e67f62f0a0a0def4c5621daa7ffce6363f7bbbbdfd197ec1594`. |
| F public-claims self-test | `python3 scripts/ci/check_public_claims.py --self-test`: exit 0. Log: `public-claims-selftest-final-0ec3164b.log`. |
| F eval library / skipped e2e | Library: 915 passed / 2 failed / 0 ignored / 3,528 filtered in 5,581.30 seconds; Cargo exit 101. Stale graph-report fingerprint and migration-bundle structure failed. e2e_eval did not run. The committed_public_fixture_passes regression is a library test and is distinct from the separately skipped standalone verifier CLI. |
| F ordinary/Windows CI | No final-F CI acceptance result is claimed. P/Q supersede this iteration; H CI remains historical. |
| F native CI | No final-F native acceptance result is claimed. H native artifacts retain H identity and cannot cover P's changed source tree. |
| Historical independent review / maintainer boundary | Before the later eval failures, three read-only reviews found no new blocker in the F test changes, source/evidence chain or implementation/compatibility claims. Their scope did not replace the subsequent eval gates, which did find the two failures above. Windows all-targets compiles the two F integration targets; their execution belongs to Linux production CI, not the Windows jobs. Recovery remains at-least-once with idempotent retry, not an atomic transaction across every file/DB/checkpoint effect. Normal maintainer review remains pending. |

### P/Q acceptance record — actual checkpoint, final acceptance pending

P is the new clean source that must produce replacement native evidence.
Q is the final evidence-only review commit; no P/Q success is established by
H/T/E/F history. Use [PR #1106 Checks](https://github.com/majiayu000/remem/pull/1106/checks)
for the current review head and record each run's actual SHA/attempt.

| New acceptance record | Value |
|---|---|
| P exact SHA / Git tree / clean checkout | `47e4ecdeeda81557e644f303e68bec208a8defca`; parent F `0ec3164b5619d4366c71a2df32c87723a6bd6330`; Git tree `d23b4b40c6f942c16f5732d8256271a6e764e6e0`. Fetched remote object and clean checkout verified in `source-proof-producer-47e4ecde.json`. Local equivalent `377cbb1e39d59a9501961a61d2e2cd7fb12b2805` has the identical tree. Two-file diff: +397/-306, including the full generated report. |
| P production-input tree / unchanged fixed pathspec / suite | Freshly measured production-input tree `83a1b162258d5f8a34222edbd861036bee6e507d90682bb67116104f876f4506`; unchanged pathspec `d3b28d78b492388f5e01ebf51519f8d5d35775e0db89d1e2f3306f9343875c20`; unchanged suite `56dad240cc175fb3d3900875f05351b9541f9ef845aa54eddfc460151f3e257d`. |
| P actual graph-report generation command / exit / report fingerprint / deterministic fields / unchanged gates | Actual command above exited 0 on F with the new dirty working bytes before committing P; all report checks passed. Report SHA-256 `be33ae6541768c0b134d37b97e1c1b5f4a65f2ef532bccf7664f3d89beefde92`; combined fingerprint `58641c845dbdb46867d848edbfc7ada5c878ffb7de76a96c2b982b4acca3d325`. 160 implementation files + dataset + 96-SQL bundle = 162 inputs. Independent full recomputation matched; all non-timing fields outside the fingerprint, including three arms × 71 queries, remain unchanged. No threshold/guard was weakened. |
| P focused graph/fingerprint/migration regression command and counts | On clean P, `cargo test --locked --features eval --lib eval::graph_decision::evidence_fingerprint::tests -- --test-threads 4`: exit 0; 6 passed / 0 failed / 0 ignored / 4,439 filtered; 7.12 seconds. Log `graph-fingerprint-producer-47e4ecde-tests.log`. This includes unchanged migration structural and committed-report guards. Earlier same-byte dirty working-tree run also passed 6/0/0 in 7.40 seconds; it remains a distinct run. |
| P source-preparation/iteration gates and exact source binding | Clean P `cargo check --locked`: exit 0 (Cargo reported 26.05s); `cargo fmt --check`: exit 0; public-claims self-test: exit 0. File-size and diff-whitespace guards also passed on the same patched bytes. `producer-47e4ecde-blocked-checkpoint.json` records clean HEAD/source proof, exact log hashes and real exits. These focused/iteration checks do not replace full Q preflight or full Q eval lib/e2e. |
| P native run/attempt/head: four platform jobs and aggregate | **Not started.** Dispatch is blocked as described above; no run/attempt ID or native-P success is claimed. Frozen branch ref is verified at P. |
| P bundle repository/run/head authentication / four artifact digests / original receipts | **Pending:** no P run or artifacts exist yet. Historical H receipts remain H evidence and cannot certify P. |
| Q exact SHA / parent / Git tree / clean checkout / evidence-only scope | **Pending:** Q has not been created; it must import authenticated P evidence only. |
| Q import changed-file/byte/payload counts and all mapped-byte validation | **Pending — not executed or measured because Q does not yet exist**; do not reuse historical E's 162/568 counts. |
| Q production-input tree, pathspec and suite equality with P | **Pending — not executed or measured because Q does not yet exist** |
| Intended Q PR body SHA-256 / actual stacked base SHA | **Pending final Q body.** Existing draft PR base is `fix/terminal-job-failure-diagnostics`, SHA `af74c0665b170a8bb258cbdc09e04c991e82f393`. No new PR body or review ref has been published. |
| Q actual committed-root Rust verifier exit / P producer object resolution / required security runs / current target coverage / source bindings | **Pending — not executed or measured because Q does not yet exist** |
| Full Q preflight exact command, exit, gate counts and log | **Pending — not executed or measured because Q does not yet exist** |
| Complete Q production + integration + doc-test targets and total passed/failed/ignored counts | **Pending — not executed or measured because Q does not yet exist**; count outer target summaries, not nested subprocess harnesses. |
| Q default cargo check | **Pending — not executed or measured because Q does not yet exist** |
| Q public-claims self-test | **Pending — not executed or measured because Q does not yet exist** |
| Q eval lib + e2e targets and counts, including committed_public_fixture_passes | **Pending — not executed or measured because Q does not yet exist** |
| Q ordinary CI and both Windows jobs: exact run/attempt/head and result | **Pending — not executed or measured because Q does not yet exist** |
| Q four native jobs + aggregate: exact run/attempt/head and result | **Pending — not executed or measured because Q does not yet exist** |
| New independent review findings / normal maintainer review | Read-only reviews confirmed exactly 11 necessary source inputs + 3 SQL paths + 11 existing coverage assertions; the hashing logic and both strict failing guards are unchanged. Independent report recomputation matched all input hashes and non-timing report fields. A separate delivery review corrected prerequisite file ownership, Unix/Windows fault counts and the P→Q scope wording. Maintainer review remains pending. |

Run the stable final command plan on Q using the actual intended Q PR body:

```bash
python3 scripts/ci/check_pr_preflight.py --base origin/fix/terminal-job-failure-diagnostics --pr-body-file /tmp/pr-body.md
cargo check
python3 scripts/ci/check_public_claims.py --self-test
cargo test --features eval --lib eval --test e2e_eval
```

The full preflight includes the complete
`cargo test --no-default-features --features local-onnx` production/integration
suite, formatting, Clippy, hook and governance checks, plugin tests, native Web
API and isolated SessionStart smoke, extraction baseline, temporary security
generation, eval gates and the two expected rejection cases. A `--fast`
iteration result is not the full final gate. The expected eval-gate negative
cases are separate from F's two actual Rust test failures.

The extraction baseline update changes 27 observation request hashes for the
intentional input-budget metadata. Corpus, fixed outputs, candidate hashes,
metrics and thresholds remain unchanged and require the full Q gate. This
baseline adjustment is separate from the freshly executed graph report.

Publication order: after all local Q gates actually finish, first update this
Epic with their exact results and clearly state which Q CI gates still await
execution. Then publish the matching intended PR body and review-branch ref.
After final Q CI completes, update this Epic's acceptance record. Do not mark
unrun checks passed or close the Epic on local results alone.

Before checking the original A–I items or closing this epic:

- [x] P correction and actual graph-report generation preserve the strict current contract; the new clean source identity is verified.
- [ ] Authenticated P evidence is imported in Q without relabelling producer IDs or changing raw payload bytes, and the committed Q root passes the actual Rust verifier.
- [ ] P→Q evidence-only diff and production-input tree/pathspec/suite equality are verified against actual Git objects.
- [ ] Full Q preflight, required additional default/public-claims/eval checks and complete production/integration/doc-test counts are recorded without omitted failures.
- [ ] Final Q ordinary CI, both Windows jobs, four native target jobs and aggregate are successful.
- [ ] A–I and #1102–#1104 regressions are mapped to final Q execution evidence.
- [ ] Compatibility, migration and rollback guidance has been reviewed.
- [ ] Normal maintainer review is complete and remaining findings are resolved.

### Research and operational limits

Synthetic events/vectors, fixed model outputs, deterministic benchmark readers
and harmless fake subprocesses verify parsing, policy, storage and recovery.
Real native OS execution adds platform evidence; it does not turn those
fixtures into live-model extraction quality, recall, user benefit or latency
measurements. No real model, Claude/Codex CLI or private memory database is used
in these checks.

#931, #933, #934 and #935 retain their own acceptance and public-claim gates.
The GH931 registry remains unready/INSUFFICIENT; this epic does not complete
its official matrix. Broader GH953 S2–S5 convergence, the complete router
executor, graph activation and governed cross-host/live evaluation remain
separate work.

Exact vector fallback still scans eligible embeddings while bounding result
memory. Fair dispatch orders queues and stage/host/project groups; it does not
preempt a running call or establish per-session CPU fairness. Summary current
proof remains bounded to 256 source events and fails closed. Usage writes and
heartbeat retirement are best effort with explicit error logging; cancellation
can retain received evidence but cannot invent telemetry never observed.


Recovery is at least once with idempotent retry. Required effects precede the
checkpoint; file, memory and task-checkpoint writes are not one atomic transaction.

### Follow-up design priorities

These remain separate optimization or measurement work; they are not extra
completed capabilities of this PR.

| Priority | Next concrete work | Acceptance boundary |
|---|---|---|
| 1. Real default-pipeline quality | Complete the governed paired-task protocol under #931/#935 with fixed task sets and budgets; record failures, aborts, maintenance work and memory-hurt cases as well as successful runs. | Report the official matrix, uncertainty and stop-loss conditions. Offline security PASS does not establish live-model quality or coding superiority. |
| 2. Retrieval semantics across entry points | Continue GH953 S2–S5 beyond the shared vector candidate executor: compare FTS, fusion and overall eligibility under identical query/profile/scope/policy inputs. | Differential tests should explain exclusions consistently; entry-specific channel and budget differences must be explicit. Keep #934's independent acceptance. |
| 3. Meaning across extraction chunks | Compare whole-input and bounded-chunk fixtures at the same budget for references, retractions, revisions and long decisions. If measured loss warrants it, evaluate bounded prior state with source IDs. | Coverage and resumability alone do not prove semantic fidelity. Facts and revisions must retain traceable evidence without incorrect merging. |
| 4. Large-memory retrieval and maintenance | Measure cold, rebuilding and warm index paths using rows scanned, readiness/backfill work and latency distributions before changing caches or incremental maintenance. | Preserve complete exact fallback and agreement with indexed results for the same profile and scope. The 10,000-vector fixture's elapsed time is only diagnostic. |
| 5. Verification test cost | Separate local schema/hash/path/presentation tests, single-task production snapshot mutation tests, and a smaller set of complete public-root integration tests. Static inspection found 60 direct full-verifier call sites plus two report-generation callers in the reviewed artifact test modules. Run the small graph-fingerprint and migration-consistency filters early during iteration. | Preserve every attack class, a valid baseline, the specific rejection rule, top-level failure propagation and invocation isolation. Do not use a global cross-fixture verdict cache or loosen official suite coverage. The call-site count is static analysis, not measured replay execution. |


## 2026-10-06 UTC follow-up: authenticated native import and blocked evidence candidate

This is a new execution record; the earlier P/H/T/E/F acceptance history above remains unchanged. The review branch #1106 is still `5a9336898905a5d5d884c77bd457fddda609f1a0`.

- Frozen producer P′: `2c153b704597ee09eef9606f01167d25521174a6`, containing P plus the #1101 surface-contract correction S `f5378f49c418d69659589cd806fd16d58a56ab9d`. [Native run 37516111367, attempt 1](https://github.com/majiayu000/remem/actions/runs/37516111367) completed successfully on all four native targets and its aggregate job. The 80 actual task snapshots recomputed with zero policy failures. All five original artifact ZIP digests, row receipts and the aggregate receipt were verified against the GitHub run metadata and retained without byte changes.
- Evidence-only candidate Q: `977ffb49fee3170ec29117b30213a02ce8f82f53`, tree `0677435fcee71bc83443c58bc1569079750e0f07`, sole parent P′. It is retained only on `audit/evidence-review-20261007-977ffb49`. Its 372 path changes are limited to evidence/provenance; the production input hash, pathspec hash and suite hash equal P′. The four active v2 manifests refer to the original full-target reports, with the required generic manifest retained as the ARM macOS template. The obsolete Linux alias manifest is deleted.
- On the actual clean Q checkout, `cargo run --locked -- bench verify --root eval/public --json-out <receipt>` passed (exit 0, 321.505 s): build and checkout both identify Q; 80 security runs, four current targets, zero policy failures, source-equivalent executable and `release.ready=true` for the security authority. GH931 remains INSUFFICIENT.
- The next actual command, `cargo run --locked -- eval-gates --json-out <receipt>`, failed (exit 1, 220.724 s). All 114 legacy metrics passed and `failures=[]`; the required `production_security_e2e` row is incomplete because `ship_matrix` still selects the retired `adversarial-policy-v2-linux-x86_64.json` report path. The exact Q `consumer_convergence` regression also failed at its selected-report assertion (0 passed, 1 failed, 4447 filtered).

Q is therefore **not accepted and has not been moved into #1106**. Full Q evaluation/preflight and final Q CI are not reported as passed. Fixing the remaining consumer paths changes source identity: prepare a new immutable producer, correct all active report consumers and mutation fixtures without aliases or weaker authority checks, rerun its complete native workflow, and import those original artifacts into a new evidence-only candidate. The final local gates, frozen intended PR body and review-branch publication order above still apply to that new candidate. No merge, release or research acceptance is implied.


## 2026-10-06 UTC follow-up: preserve the concurrent source update before new evidence

The earlier execution records are historical. The review branch #1106 is currently `1692f1da92d51011d9f4fbbccf43ff45538ee354`; it has not been replaced with an evidence candidate.

- P″ `b65e3e69bf2ed5f35f13fb8ff42edf8a0ecfa954` ([#1108](https://github.com/majiayu000/remem/pull/1108)) fixed the selected native report consumers and the mutation fixture. Its 17 focused regressions passed. [Native run 37523629129, attempt 1](https://github.com/majiayu000/remem/actions/runs/37523629129) completed all four targets and the aggregate successfully; the original five artifact ZIPs, their GitHub digests, 80 task snapshots and receipts were authenticated.
- Q′ `b5da4a8d1db95fdb4b6943e17d095d22a5f03f51`, tree `8d9b39a6606962c9a194592e3c225f0beeacfd1a`, is an immutable evidence-only child of P″ with 167 evidence/provenance path changes. No clean-Q′ acceptance command was started: the fresh #1106 lease check found the concurrent source update before publication. Q′ and its frozen intended body remain unchanged checkpoints.
- New producer P3 `89d170df019795f0e1d6b5b83557bcd3a63dc4c3`, tree `6ecdb33e8d7d9f456f973d240b6607b6aa6bbf9a`, preserves both histories with ordered parents Q′ then `1692f1da92d51011d9f4fbbccf43ff45538ee354`. It retains the concurrent atomic embedding source comparison, fresh coverage, protective-constraint source checks and mandatory pending review, along with the prior Windows, shared-root, graph and native-report corrections. The long-whitespace protective-preview regression is fixed without changing original event bytes.
- P3 production-input SHA-256 is `21bd0e31c5c00dff5c42642a6b4d471c45f23cec0929d87e3336de2220d2bb64`; pathspec and suite remain `d3b28d78b492388f5e01ebf51519f8d5d35775e0db89d1e2f3306f9343875c20` and `56dad240cc175fb3d3900875f05351b9541f9ef845aa54eddfc460151f3e257d`. The real graph command regenerated report SHA-256 `4ff4619e32e7306d103c7fad8a4f63719b858e7e19b0411a37c7fe91b8de5a24`; all 162 inputs and the migration bundle were independently verified, all eight checks passed, and raw output bytes were imported unchanged.
- On the staged merge, 204 focused tests passed (53 vector, 145 user-context, six graph fingerprint), with zero failures or ignored tests. Strict all-target Clippy, formatting and source/governance checks passed. These remain preparation results, bound to the recorded staged tree; they are not relabelled clean-Q3 acceptance. After canonical fetch, exact clean P3 tree/parents/source hashes and the spec-lifecycle check against S with frozen body SHA-256 `7e670997d79d9070372b67a15c84d1aea0c4719628b189bd6cdad104c7a33954` also passed.
- The frozen producer branch is `audit/native-evidence-20261007-89d170df`; [draft #1109](https://github.com/majiayu000/remem/pull/1109) triggered [native run 37529659434, attempt 1](https://github.com/majiayu000/remem/actions/runs/37529659434). At this record, the fresh native matrix is running; no P3 native result is claimed yet.

Existing P″ receipts cannot certify the changed P3 production inputs. The next evidence-only child Q3 must import the new original artifacts, pass the actual committed-root verifier and all required clean final-candidate gates with its frozen intended body, and receive final-head ordinary, Windows and native CI results. Re-read #1106 and preserve any further concurrent update before the ancestry- and lease-safe ref update. This record does not check off final acceptance, close this Epic, merge a PR, release a build or complete the separate research gates.


## 2026-10-06 UTC final integration and acceptance

This is the continuing execution record for the final source integration. It does not replace the failed, interrupted or unexecuted checkpoints above. No final Q4 candidate has been accepted or published at this checkpoint.

### Completed P3 evidence and observed ordinary failures

P3 `89d170df019795f0e1d6b5b83557bcd3a63dc4c3` completed [native run 37529659434, attempt 1](https://github.com/majiayu000/remem/actions/runs/37529659434): all four native targets and the aggregate succeeded. The five original ZIP artifacts were downloaded and verified against the actual artifact byte counts and SHA-256 digests. The aggregate artifact was `11444692957`, SHA-256 `de92f76f2590b9b73e2c325d3728fd7bd87ad0ac49f6ced853a3eb0206c8a2e2`. These results belong to P3.

P3 ordinary [run 37529659424](https://github.com/majiayu000/remem/actions/runs/37529659424) retained two actual failures: the stale complete-file caller fingerprint and the Windows doctor/profile assertion. Its aggregate correctly failed. Q3 was not imported, frozen, accepted or published.

### Preserve both subsequent source updates

The unpublished P4 preparation tree `8feced52a8852c0c11bead9be6605ef326600b9f` integrated the Windows home repair at `a345c8ba4098952154f5168b7e558439fceeffa3` and completed its recorded local preparation. A fresh review-head check then found `479590b13232b35776a3c17cf90d309d90e135da`, which makes ordinary doctor file logging read-only. The former preparation source, body and results remain frozen; none of its successes certifies the later source.

The revised integration preserves the complete incoming HOME/USERPROFILE selection, the synchronous thread-local logging suppression, stderr diagnostics, actual log-health reads and the real filesystem regressions. The original no-data assertions remain strict; the temporary provider pin was removed. The additional invalid-Codex/provider fixture checks actual API and Windows Auto diagnostic wording and requires the whole data directory to remain absent.

On intermediate tree `df7fb1526bbb9a3212d8ec324443a9a38e1aa939`, Linux install/profile tests passed 9/9 and logging tests passed 20 with one existing subprocess-helper test ignored by the outer harness. The complete doctor module exposed a real test failure: 204 passed, 1 failed. The existing provider-profile fixture inserted literal `content_hash='hash'`; the new freshness contract correctly excludes that stale row. The correction changes only this test to store the actual versioned passage hash and retains its provider-normalization and 1/1-coverage assertions, adding failure detail. The production freshness algorithm is not relaxed. The failed run is retained.

The current unpublished source preparation is tree `76bdccdb80050e46aab6e64accfda68aa1212e92`, planned ordered parents P3 and `479590b...`, with production-input SHA-256 `162894304341909c4822a28126e73ab39ec022ab8379426834137850a78bf794`. Its changed-source doctor tests are being executed before producer publication. There is no producer commit SHA, native run or final-Q4 pass assigned to this preparation yet.

A separate host-roots invocation ran 5 successful tests, but its post-run source-recording wrapper encountered an index lock while another independent source verification was running. That wrapper interruption is retained separately and will be closed with a sequential recorded invocation; it is not represented as a complete gate pass.

### Existing evidence and base results

Current review head `479590b...` passed its native four-target run and aggregate, but its [ordinary benchmark job 112514024423](https://github.com/majiayu000/remem/actions/runs/37535135257/job/112514024423) failed after compilation: all 80 old committed snapshots lack `worker_dispatch_state`. This requires authenticated new native payloads and the intended evidence-only import, not weaker schema or verifier checks.

Base S `f5378f49c418d69659589cd806fd16d58a56ab9d` has now completed [ordinary run 37515337293](https://github.com/majiayu000/remem/actions/runs/37515337293) and [ordinary run 37515411165](https://github.com/majiayu000/remem/actions/runs/37515411165), both successful, each with 908 passing eval tests and one passing sandbox e2e. Another run on the same S head remains in progress. Its native four targets and aggregate were already successful.

### Final candidate remains pending

The remaining order is unchanged: publish and read back the actual immutable producer identity; authenticate all five original same-run native artifacts; create the evidence-only child with exactly four active manifests and original payload bytes; freeze its intended review body; run the committed-root verifier, required regression/eval tests and complete original preflight on the actual clean candidate; record real exits, outer counts and log hashes here; then update the matching body and review ref with a fresh ancestry/non-force lease check. Final-head ordinary CI, both Windows jobs, native targets and aggregate remain required.

GH931 remains INSUFFICIENT. Native execution of offline synthetic security fixtures does not establish live-model quality, merge approval or release readiness.


<!-- nightly-current-final-status:start -->
### Current final-candidate execution checkpoint

**Actual Q5 is [`9e40361d9346d09270864da675d3ef17aebd6ad7`](https://github.com/majiayu000/remem/commit/9e40361d9346d09270864da675d3ef17aebd6ad7)**, tree `2e538e34d86001005ba2cb0c77ff16755711b544`. Root verified the GitHub commit/tree, branch and ordered parents: native producer P5 `c144b36a39415a28cd761d48b5ff78c88a6dfb10`, prior candidate Q4 `e6735b1165bd92e47333ad7b322640791ecbe689`, and concurrent author head `049fbcba7b8f0d2e700159aedde197f6575a565e`. Q5's support run and eight gate results do not yet exist at this checkpoint.

`tests/shared_session_roots.rs` is the sole Q4-to-Q5 change: all 049 tests are retained and the common rejection helper requires `Some(1)`. The Codex file-root case proves no data/DB creation and preserves encrypted-fixture zero-row/diagnostic checks; other root-fault coverage remains. P5 to Q5 changes 168 paths: 80 original SQLite snapshots, 80 run records, seven provenance files and this test. This is **test-and-evidence integration, not evidence-only**. Integration manifest SHA-256: `33909aae9642bedabc4127cdbb095da4adf5067e3bf66e0400f556397c84df66`. All 568 mapped public files and five original receipts retain exact bytes/Git blobs. Production input, pathspec and raw suite are unchanged. Four active manifests reference full-target reports/artifacts; the original aarch64-apple-darwin manifest bytes occupy the generic committed filename. Concurrent source/evidence histories are retained by the parents.

**Completed P5 results remain attributed to P5.** [Native run 37539811793](https://github.com/majiayu000/remem/actions/runs/37539811793), attempt 1, passed four native jobs and aggregate. All five original ZIPs matched API artifact IDs, byte counts and SHA-256. The original aggregate recomputed 80 cases: zero policy failures, four current targets, no missing/stale targets. Build/checkout were clean P5; production input SHA-256 `162894304341909c4822a28126e73ab39ec022ab8379426834137850a78bf794`. Input-authentication report SHA-256 `294d25f90608382fe8625ab38bf39de146aba3ad47121f5401929191d5e36f72`. [Ordinary run 37539811670](https://github.com/majiayu000/remem/actions/runs/37539811670), attempt 1, completed SUCCESS (updated 2026-10-07 00:02:01 UTC); runtime finished 00:01:55 with production and eval successful. All five ordinary and five native head checks passed. These are not Q5 build/gate results. Earlier 233/0/1 preparation retains its original source-tree scope. GH931 remains INSUFFICIENT.

**Frozen Q5 PR body:** 50,965 bytes, SHA-256 `bc87a2fbb163ff386ca22e03c58d85ef8365a8042067891f63fe706add323929`. The latest author's 049 body is preserved verbatim exactly once: 38,283 characters, SHA-256 `befd83b92c43131b3a530517736f0410bd0264812f006ba6d9e76690fe6a456f`, including earlier failures, tests and pending checks. At this checkpoint #1106 remains at 049 with base S `f5378f49c418d69659589cd806fd16d58a56ab9d`. Result changes update this ledger, not the frozen body.

**Q4/R1 are separate history.** Q4 `e6735b1165bd92e47333ad7b322640791ecbe689` retains its frozen body SHA-256 `4ab1ccdc4ada142ca0a480e7450b26f99ee4ed4ec21e9a528fc212c3f5665c73`. Its [run 37546262464](https://github.com/majiayu000/remem/actions/runs/37546262464), support R1 `ab682208a24951838eb1d579af0e5dc6ffdb2ca0`, had both groups running at 00:10:53 UTC, without a terminal verdict. Its results/failed attempts remain under Q4; the live-log 404 was not a test failure. No Q4 result or helper self-test is counted as Q5 acceptance.

**Q5's fixed acceptance contract:** run all eight original gates on actual Q5 and two isolated GitHub-hosted machines: default check; public-claims self-test; committed-root verifier; committed-root eval gates; consumer-path regression; snapshot-mutation regression; full eval-library/e2e; complete original preflight using base S and the frozen Q5 body. Preflight must execute production/integration/doc tests, including the integrated fixture, plus original smoke, surface, lint and constructed regressions. No fast mode, skipped phase or output adapter. Record runner environments, including the pinned library's standard loader path; candidate source and gate commands stay unchanged. Hosts, targets and temporary directories are separate. Retain raw logs/outputs, commands/counts/exits, hashes, failed attempts and pre/post candidate/body/configuration/helper identities. Require the complete successful eight-gate union from this candidate/body and run attempt.

Q5 gates had not run at this checkpoint. After recorded completion, update this ledger first; publish the frozen body and advance #1106 only with fresh head/base/body, ancestry and non-force lease checks, preserving subsequent author updates. Final-head ordinary CI, both Windows jobs, four native jobs plus aggregate and normal maintainer review remain required. Keep draft and GH931/research/live-model boundaries; no merge or release is claimed.
<!-- nightly-current-final-status:end -->
