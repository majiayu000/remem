# Current evidence refresh and acceptance record

This intended evidence-only update preserves the complete source and verification history below. References to “current source” or “pending” inside that preserved history retain the scope of their original commit and date. Current producer/evidence identities and final execution results are governed by this leading section and the [fixed execution record in Epic #1105](https://github.com/majiayu000/remem/issues/1105#2026-10-06-utc-final-integration-and-acceptance).

The frozen source producer is P5 [`c144b36a39415a28cd761d48b5ff78c88a6dfb10`](https://github.com/majiayu000/remem/commit/c144b36a39415a28cd761d48b5ff78c88a6dfb10), introduced in [draft producer PR #1110](https://github.com/majiayu000/remem/pull/1110). Its ordered parents are `cc890bef2d65d99ea42b5306637a74ee64f664fb` and the concurrent source `9ab293eed7f3d9471aacb2886a0480d798378a4c`. Both canonical fixture fixes already existed in the verified source; the merge preserves both histories with the identical tree `76bdccdb80050e46aab6e64accfda68aa1212e92` and the stronger coverage failure diagnostic. The earlier 233/0/1 results remain preparation on that exact staged tree, not tests claimed to have executed again on the merge commit.

The intended evidence-only candidate Q4 has ordered parents `c144b36a39415a28cd761d48b5ff78c88a6dfb10` (P5, the unchanged source producer) and [`70b2de0d414a1adeaf2623e00843bdf1ee6d35dd`](https://github.com/majiayu000/remem/commit/70b2de0d414a1adeaf2623e00843bdf1ee6d35dd) (the concurrent evidence-only follow-up to `9ab293eed7f3d9471aacb2886a0480d798378a4c`). The second parent preserves the author’s 80 SQLite snapshots and 80 run records as immutable Git history. Its older producer payloads are not relabelled as P5 evidence. Active evidence is imported from the new P5 native run under the four correct full target paths, without restoring obsolete manifest aliases.

Relative to the first parent P5, Q4 changes only authenticated native payloads, their active manifests, reports and provenance records. It preserves the complete source byte identity of P5; the concurrent second parent contains no additional source, workflow or documentation changes. The actual candidate SHA, tree, ordered parent identities, byte-checked import manifest and gate ledger are recorded in the Epic execution record after that commit exists. No implementation source, production pathspec, suite, policy threshold, published surface baseline or source-authority rule changes in this evidence import.

| Identity | SHA-256 |
| --- | --- |
| Production inputs | `162894304341909c4822a28126e73ab39ec022ab8379426834137850a78bf794` |
| Canonical production pathspec | `d3b28d78b492388f5e01ebf51519f8d5d35775e0db89d1e2f3306f9343875c20` |
| Original adversarial-policy suite | `56dad240cc175fb3d3900875f05351b9541f9ef845aa54eddfc460151f3e257d` |

## Native evidence bound to this producer

[Native workflow run 37539811793](https://github.com/majiayu000/remem/actions/runs/37539811793), attempt 1, completed successfully on the exact frozen producer `c144b36a39415a28cd761d48b5ff78c88a6dfb10` at `2026-10-06T22:36:16Z`. All four native target jobs and the aggregate job succeeded. Each original ZIP was downloaded, matched to its GitHub artifact identity, byte count and API SHA-256 digest, then safely extracted.

| Scope | Successful job | Artifact ID | Cases | Original ZIP SHA-256 |
| --- | --- | --- | --- | --- |
| `x86_64-unknown-linux-gnu` | [112529768639](https://github.com/majiayu000/remem/actions/runs/37539811793/job/112529768639) | `11447844084` | 20 | `13592f8df2954d51fd2f9cd3398ea752be45f3f2fb9a39375f494a0a72b67e64` |
| `aarch64-unknown-linux-gnu` | [112529768389](https://github.com/majiayu000/remem/actions/runs/37539811793/job/112529768389) | `11447564052` | 20 | `e8da3b7127c7397efba73b6aafe5b05a27e69f8fff46e4aebcb8c778416f3a41` |
| `aarch64-apple-darwin` | [112529768815](https://github.com/majiayu000/remem/actions/runs/37539811793/job/112529768815) | `11447734570` | 20 | `fb67169cae66cdd3b3b00222e693efdd5c56dcbf7067f3cbacd834ae64204a51` |
| `x86_64-apple-darwin` | [112529768525](https://github.com/majiayu000/remem/actions/runs/37539811793/job/112529768525) | `11448565310` | 20 | `039d67cabd2eea21730107c3001e41b108a5a09c006b3d57335ca8e17922f7a5` |
| Aggregate of the same four targets | [112533973818](https://github.com/majiayu000/remem/actions/runs/37539811793/job/112533973818) | `11448413711` | Same 80 | `438384725ada34affb08a05c3ef415d3e9e217561404385a85f14b60c7394d38` |

The original aggregate receipt reports `PASS`, 80 recomputed security runs, zero policy failures, all four current release targets, and no missing or stale targets. Its build and checkout are both clean P5 with production hash `162894304341909c4822a28126e73ab39ec022ab8379426834137850a78bf794`; executable/source equivalence is true. The aggregate's 734 consumed paths were checked against original bytes. Its complete temporary root also includes pre-existing suites; its totals of 8 manifests, 8 reports, 105 run artifacts and 605 artifact files are not represented as extra new security cases. GH931 remains `INSUFFICIENT`. The release-readiness field is the verifier's scoped native security verdict, not permission to release or a claim of final Q4 acceptance.

Full local bundle authentication completed at `2026-10-06T22:46:54Z` with exit 0 in 1.924 seconds, before import and without changing clean P5. It verified 568 mapped public files, 80 run records, 480 payload files including all 80 SQLite snapshots, and the five original receipts. The validation report SHA-256 is `294d25f90608382fe8625ab38bf39de146aba3ad47121f5401929191d5e36f72`; its execution log SHA-256 is `5864138ae62363d3b499d5236047b54872d3d2e9f7a7385a04d80978c4f50414`. This is authenticated input validation, not an execution of the future committed Q4 verifier or preflight. The imported provenance under `eval/native-evidence/run-37539811793/` preserves the four row receipts and the aggregate receipt; the aggregate receipt SHA-256 is `52b5187269c2f5193bd0f567700313e3a6911d232cb8ffab61e850f19aae8d0d`.

Retain exactly four active committed manifests. The original `aarch64-apple-darwin` manifest bytes occupy the required generic committed filename; its original native filename and bytes remain in the receipt. All four reports and their run artifacts retain their full target paths. Preserve the five original receipts and payload bytes. Historical inactive evidence remains historical, without new aliases, repaired empty tables, rewritten receipts or weaker verification.

## Actual candidate gates and publication order

This intended body is frozen before the actual evidence-child commit and local acceptance runs. It does not predeclare those outcomes. Exact candidate identities, real commands, exits, test counts, log hashes and all failed/interrupted attempts belong to the Epic's fixed execution record. The final body remains this exact reviewed byte sequence while those execution records are updated separately.

The actual clean candidate must pass the committed-root Rust verifier, committed-root eval gates, the two focused report-path and snapshot-mutation regressions, default `cargo check`, public-claims self-test, full eval-library/e2e invocation, and the complete original preflight using this body and stacked base `f5378f49c418d69659589cd806fd16d58a56ab9d`. Full preflight includes actual production, integration and doc tests, surface discovery, strict Clippy, extraction baseline, native Web API and isolated SessionStart smoke, and the two constructed-regression rejections. Its own temporary workspace isolates all eval outputs. No output adapter, fast mode or skipped full-preflight step is used.

Separate execution-record tags may isolate ledgers and files for independent gate groups; their union must contain every required gate. Tags do not change a source command, verdict or candidate/body/environment/helper identity. The full preflight's real production/integration/doc execution need not be duplicated by another identical standalone run. Outer test totals exclude internal helper subprocess summaries.

After the actual local gates finish, first update the current execution section of Epic #1105 with actual results and any remaining CI requirements, then publish this intended body and move the review branch only after a fresh ancestry and lease check. Preserve any additional source or body changes submitted concurrently. Final-head ordinary CI, both Windows jobs, all four native targets and their aggregate, and normal maintainer review remain required. Keep this PR draft until those conditions are satisfied.

Native execution of synthetic fixtures is not live-model extraction or research acceptance. GH931 and the separately parked research/cross-host work remain outside this completion claim. No release, deployment or merge is authorized by a security readiness field.

## Preserved author source and verification history

The following is the complete original PR body captured at head `70b2de0d414a1adeaf2623e00843bdf1ee6d35dd`, updated `2026-10-06T22:31:30Z`. It is preserved verbatim as chronological history, including its original commit scopes, unchecked readiness boxes, external-session results and unexecuted or failed attempts. Those historical statements are not relabelled as this evidence candidate’s results.

<details>
<summary>Original source implementation and verification record</summary>

## Latest source and verification status

Current PR evidence head: `70b2de0d414a1adeaf2623e00843bdf1ee6d35dd`, tree `f2944f998d15c3c962b17f0248dfc51c1dee1f5c`. It has parent/actual native producer `9ab293eed7f3d9471aacb2886a0480d798378a4c` and changes only the 160 evidence files described in the final section. All 1,384 production input paths retain fingerprint `8a67b532f470a27466a1433c80c104f1b8397e67bbdcc82fd07ebac691a7a862`.

The source and verification statements in the earlier chronological sections retain their original commit scope. The final section records the authenticated import and its actual verification. Full default preflight is not claimed as an overall PASS here; use its recorded command outcomes and the final-head checks separately.

## PR Type

- [ ] Spec only
- [x] Implementation
- [ ] Bugfix
- [ ] Release/docs/process

## What

This is the implementation follow-up to the memory pipeline audit in #1105.
It is stacked on the terminal diagnostics change in #1101; review this diff
against `fix/terminal-job-failure-diagnostics`.

- Verify the immutable activation receipt and current source evidence before
  admitting an eligible summary into CurrentTruth. Reassess eligible untouched
  non-preference candidates with new trusted evidence through an audited
  replacement candidate, preserving human review decisions and suppression.
- Isolate vector mirrors by the complete embedding profile, apply scope before
  KNN limiting, and search all eligible rows in the exact fallback.
- Retry typed malformed model output, bound extraction to an explicit contiguous
  input prefix, and persist progress only after that prefix's required effects
  complete. Recover replay family members from their own verified checkpoints.
- Persist fair service order across ordinary worker queues and project/stage
  groups. Retire worker heartbeat ownership on exit and check real Windows
  process liveness.
- Record usage evidence on successful and failed AI attempts, preserve partial,
  missing, invalid, estimated and historically unverified states, and expose
  pricing coverage alongside the known cost portion in CLI and API aggregates.
- Make automatic workstream identity matching conservative under broad names,
  ambiguous aliases and content-session collisions. Share `CODEX_HOME` selection
  across install, uninstall, native import and diagnostics.
- Split Clap command construction into ordered internal groups to fit the
  native main-thread stack. Preserve the public command definitions and teach
  the surface guard to recognize only this explicitly checked construction.

Source version is 0.6.103 with synchronized unreleased package metadata.
Migrations v094-v096 add checkpoint/range, dispatch and usage evidence fields;
they do not infer historical successful processing or historical usage coverage.

## Why

The audit found paths where captured evidence could fail to become usable
memory, scoped vector retrieval could miss valid older results, replay recovery
could lose its place, a busy project could monopolize work, and missing usage
could be represented as complete zero cost. These changes make the evidence,
scope and lifecycle boundaries explicit and exercise them with synthetic
fixtures, including failure and restart cases.

Native Windows CI then exposed a CLI construction stack overflow before command
dispatch. Local low-stack probes reproduced startup failures even for `--version`
and `--help`. The CLI follow-up reduces the constructor's stack use and adds an
isolated real-binary startup regression. Its local results and the still-pending
native Windows validation are separated below.

## Issue Links

- Refs #1105
- Closes #1102
- Closes #1103
- Closes #1104
- Depends on #1101; this PR does not merge or replace that draft.
- #931, #933, #934 and #935 remain independently parked.

The complete intent-controlled retrieval executor, broader GH953 S2-S5
convergence, graph activation decisions, and governed live E2E/cross-host reports
remain future work.

## Spec Lifecycle

- [x] If adding/updating `docs/specs/<id>/`, updated `docs/specs/README.md`
- [ ] If this is spec-only, linked or created implementation issue(s)
- [x] If this is implementation, updated relevant spec, README, API docs, or wrote why not needed
- [x] If touching `src/api/**`, updated `docs/specs/SPEC-web-api.md` or wrote an explicit API docs waiver with rationale

Current PRODUCT/TECH contracts cover summary promotion, candidate reassessment,
local semantic retrieval, failure/replay lifecycle, usage pricing, workstream
identity and shared host roots. The published surface baseline is retained;
new or changed source surfaces remain staged until a release is published.

### Caller compatibility

This PR adds `coverage: AiUsageCoverage` to `AiUsageTotals`,
`AiUsageSourceTotals`, `AiUsageBreakdown`, `DailyAiUsage`, and `WeeklyAiUsage`,
and `ai_usage_coverage` to `LatestSessionMemorySpend`. External struct literals
and exhaustive destructuring require updates. Use query-returned coverage for
existing usage, especially historically unverified rows. The prerequisite
#1101 also adds `JobTransitionOutcome::TerminalFailure`, requiring exhaustive
match updates; it means persisted terminal failure with no scheduled retry.
REST coverage fields are additive; existing method/path and cost fields remain
available. Clients that reject unknown fields must accept the additions, and
clients of older servers must treat absent coverage as unknown.

The surface ledger records 75 superseded declaration identities across one
enum and six structs, with same-named replacement signatures staged. The
published `v0.6.82` baseline is unchanged. This ledger update is not a release
or a claim of Rust source compatibility.

The CLI grouping retains the existing command syntax and order. Local comparison
against `4c7f0a1` found all 165 help outputs byte-for-byte identical and all 198
CLI surface fingerprints unchanged. The guard rejects unsupported constructor
changes, group attributes and feature-gate mismatches; this change does not
introduce a general macro or dynamic-parser exemption.

Migrations v094-v096 are forward database changes. Upgrade all active workers
before relying on resumable progress and fair dispatch; mixed-version execution
is unsupported. Older binaries reject a newer recorded schema. Rollback requires
a pre-upgrade database backup with its matching binary, or a forward fix. Do not
remove migration markers or fabricate checkpoints to force a downgrade.

## Test Plan

- [ ] Tests pass
- [ ] Tested manually

Validation below distinguishes the earlier clean source commit from the CLI
follow-up. The follow-up is committed locally as
`d3c17c0b36db589343753b235b838189470ce4ca`, with Git tree
`ffc01852c30d597f1963ee5fe1910badb33fdf1a`. The identical tree is now published
as source commit `5a9336898905a5d5d884c77bd457fddda609f1a0`; its production-input
tree is `2b3e948c9b1662506baca78e077f1d9d55acc2c686d56fce46099a7f6839c916`.
[Ordinary CI 37471765530](https://github.com/majiayu000/remem/actions/runs/37471765530)
and [native evidence 37471764448](https://github.com/majiayu000/remem/actions/runs/37471764448)
are running on this source. No new native success is claimed yet.

### Earlier clean source: `4c7f0a12d5426a09e1ea85d575ca4a2ba2271c85`

- `cargo fmt --check` and `cargo clippy --locked --all-targets -- -D warnings`
  passed.
- Native default-feature focused tests passed: `memory_candidate::tests` 88/88,
  `session_rollup::tests` 65/65, and production security snapshot dispatch
  regressions 4/4.
- File-size, active-memory writer, module dependency and stacked-PR spec
  lifecycle guards passed. Existing reverse-dependency exceptions were not
  expanded.
- [Native evidence run 37460017967](https://github.com/majiayu000/remem/actions/runs/37460017967)
  passed all four macOS/Linux target jobs and the aggregate job. The authentic
  four bundles were also staged and assembled locally; the actual Rust verifier
  passed all four canonical reports with 80 recomputed runs, zero policy
  failures, clean source bindings and `release.ready=true` with no missing
  targets. This verifies the earlier source and the import procedure.
- Windows CI on this source failed during CLI startup, including child exits
  with stack-overflow status `0xC00000FD`. The successful macOS/Linux security
  matrix does not establish Windows runtime success.

The preceding full production run completed with 4,120 passed, 11 failed and
1 ignored. All 11 failures were reproduced and addressed: 10 came from test
helpers assuming the old global queue order; one exposed raw archive loss on
transcript prompt rejection. The focused runs above cover those fixes. A
complete production-suite pass is still required on the final evidence commit.

### CLI follow-up: focused local checks

These checks ran before the local commit against the same Rust and guard bytes.
The test binaries recorded the earlier `4c7f0a1` build SHA with dirty source
metadata. They are focused working-tree results; clean new-source benchmark
authority remains pending.

- `install_status`: 7/7 passed, including real `remem` child processes on a
  1 MiB Linux stack for version/help and representative nested commands.
- CLI-related unit tests: 87/87 passed.
- All 165 help outputs match the earlier source byte-for-byte; all 198 CLI
  fingerprints match the existing manifest.
- CLI surface normalization self-tests: 6/6 passed; the surface lifecycle
  self-test and `cargo fmt --check` passed.
- The complete `check_public_surface.py` run passed with
  `public surface and lifecycle check: ok`.
- The `--fast` iteration preflight on clean local commit `d3c17c0b` completed
  with 22 PASS and one FAIL. Strict Clippy, hook build, migrations, dependency
  direction, plugin scripts, surface/baseline and PR lifecycle checks passed.
  The sole failure is the committed-public benchmark authority gate: a separate
  actual Rust verdict confirms all 80 diagnostics are old snapshots missing
  `worker_dispatch_state`. The verifier is clean and source-equivalent to that
  local commit. This is an incomplete preflight awaiting the evidence refresh,
  not an overall PASS. New native Windows CI remains pending.

These local Linux checks reproduce the small-stack failure boundary and pass
with the CLI fix. The new native Windows worker/profile-path and local-embedding
CI results remain required.

### Remaining evidence and final verification

The CLI change modifies production inputs. The successful `4c7f0a1` bundles
will therefore be retained as historical evidence and will not be imported as
proof for the new source. The checked-in security snapshots still require a
fresh four-platform replacement before current public-claims and
production-security gates can pass.

- [x] Publish the CLI follow-up, record its remote source SHA, and complete
  the `--fast` iteration preflight with its 22 PASS / one old-evidence FAIL disclosed.
- [ ] Pass both Windows CI jobs and all four native evidence jobs plus their
  aggregate on the new source; authenticate the same-head/run bundle downloads.
- [ ] Stage and verify the new canonical root with the real Rust verifier, then
  import only the four evidence groups in an evidence-only commit. Preserve
  actual producer SHAs, platform identities, original receipts and payload bytes.
- [ ] Run the full preflight on that final evidence commit, using this PR body
  and the actual stacked base. This includes the production and integration
  suites, extraction baseline, native Web API and isolated SessionStart smoke,
  eval gates, and both constructed-regression rejection checks.
- [ ] Also run default `cargo check`, the public-claims self-test, and
  `cargo test --features eval --lib eval --test e2e_eval`, including
  `committed_public_fixture_passes`.
- [ ] Require final-head CI, both Windows jobs and the automatically triggered
  four-platform native workflow/aggregate to finish successfully.

The extraction baseline refresh changes exactly 27 observation request hashes
for the intentional input-budget metadata. Corpus, fixed outputs, candidate
request hashes, metrics and thresholds are unchanged; the native baseline gate
will be rerun in the final preflight.

Checks use synthetic fixtures, fixed model-output samples and offline
substitutes. They verify parser, evidence, lifecycle and request-fingerprint
behavior; they do not establish live-model extraction quality or production
recall improvements. No real model, Claude/Codex CLI or private memory database
is used for these checks.

## Review

- Current source: `5a9336898905a5d5d884c77bd457fddda609f1a0`; identical local preflight tree at `d3c17c0b36db589343753b235b838189470ce4ca`; new native evidence and final evidence commit pending
- Reviewer: independent implementation agents; normal maintainer review pending
- [ ] Actionable review findings and conversations are resolved
- [ ] Security review completed when the change affects a security boundary

## Merge Readiness

- [ ] Applicable ordinary CI checks are green on the final head
- [ ] Normal maintainer review is complete

Keep this PR as a draft while exact-head CI and maintainer review are pending.
The earlier `release.ready=true` security verdict does not authorize a release.
No release, deployment or merge is part of this change.

## Audit follow-up: f52d5ee6f8c0bdc7911c3ab6e3bea435df924b6a

This source update on the existing branch addresses REM-01, REM-02 and REM-03. It supersedes the earlier source SHA for final CI and native evidence; all earlier results above remain historical.

### Source integrity and protective constraints

- Prepared embeddings retain their exact canonical and effective-enrichment inputs. An atomic SQL compare-and-write accepts them only while those inputs and searchable lifecycle state still match. The sqlite-vec mirror changes only for accepted rows within the same savepoint.
- Pending/coverage selection compares the actual passage hash, including same-second and legacy-hash mismatches. Selected work consumes the bounded backfill budget, including a full stale/deleted batch.
- Narrow complete English/Chinese protective constraints can become **pending-review** candidates when every cited source is an actual matching user-authored message. Secrets, harmful affirmative instructions, quotes, conditions, double negatives and mixed/tool/file provenance retain their rejection behavior. Generated auto-promotion cannot bypass the pending-review boundary.
- The affected PRODUCT/TECH contracts are updated. No schema migration, persisted ledger format, release version, native-evidence payload or published surface baseline is changed by this follow-up.
- Source consistency scans may hash all eligible passage bytes. This fixes correctness; it does not claim a faster coverage query.

### Independent CI with the same final requirement

The original Linux job is split into independent `runtime_checks` and `benchmark_evidence` jobs. Both Windows jobs are unchanged. The original `check` status is now an `always()` aggregate requiring all four results to equal `success`; failure, cancellation, skipping or a missing result cannot pass. Ordinary tests can now produce a result even when checked-in evidence is stale. No failure is ignored.

### Validation of these changes

- Vector library tests: **46 passed**, 0 failed/ignored, including real migrated WAL SQLite races across independent connections, canonical/enrichment changes, deletion/quarantine, mirror integrity, legacy/same-second repairs and a full skipped 512-item batch followed by later work.
- Final user-context library tests: **144 passed**, 0 failed/ignored, including protective constraints, source provenance, forced pending review, 80 matching citations and rejection of an unsafe 81st source.
- Documentation-contract suite: **74 passed**. It executes the actual aggregate shell against **17 result combinations** and checks the independent job graph. Original Linux steps are retained; both Windows job definitions compare unchanged.
- Format, diff, file-size, active-memory-write, module-dependency, migration-boundary and documentation-contract checks: passed.
- Independent source review found no remaining blocker in the scoped changes.

These are focused `--no-default-features` Rust results plus static/workflow gates. The final default/local-ONNX production and integration suites, strict Clippy, full preflight, Windows runtime checks and fresh authenticated native evidence are still required on this source. The CI split does not turn old evidence into current evidence or establish live-model/cross-host acceptance. Keep the existing draft and merge-readiness boxes pending until those gates and maintainer review finish.

### Stacked-base integration and final source

PR #1101 advanced to `f5378f49c418d69659589cd806fd16d58a56ab9d`, leaving this stacked branch one base commit behind and preventing its pull-request CI from starting. An actual three-way merge found content conflicts in CHANGELOG and the surface manifest.

Final source `1692f1da92d51011d9f4fbbccf43ff45538ee354` is a non-force merge with parents `f52d5ee6f8c0bdc7911c3ab6e3bea435df924b6a` and `f5378f49c418d69659589cd806fd16d58a56ab9d`. Its tree remains exactly `a5175d276dee81da0e5ef6eddda6878a1987462a`.

Independent structural review proved that the current tree already includes all seven base-added retired enum identities, all eight identical staged replacement records, and the complete failure-lifecycle compatibility paragraph. The combined 0.6.102/0.6.103 changelog entry also includes the base change. Keeping these contents preserves the 75 superseded declarations already reconciled by this PR, all 198 CLI records, and the unchanged published v0.6.82 baseline. No Rust, test, workflow, evidence payload or source-tree bytes changed in this ancestry integration.

The focused results above therefore apply to the identical source tree; final CI and authenticated evidence must still be checked on this final head.


## CI follow-up: caller fingerprint and explicit Windows home

Commit: a345c8ba4098952154f5168b7e558439fceeffa3.

The independent ordinary CI introduced above exposed two actionable gates:

- The `entity-bfs` caller guard fingerprints the complete source file. REM-01's backfill accounting change updated `src/cli/actions/query/backfill.rs`, so its recorded SHA-256 needed regeneration. Only that fingerprint changes; the 18 caller paths, 5,280 surface records, experimental/default-path statuses, and baseline remain unchanged.
- Windows `dirs::home_dir` reads the native Known Folder profile rather than the test's process-local HOME/USERPROFILE. Claude diagnostics therefore missed the isolated fixture. An internal resolver now selects absolute HOME, then absolute USERPROFILE, then the existing native fallback on Windows; Unix retains its existing resolution. Claude install paths, availability and doctor discovery share the resolver. Cursor and Codex selection are unchanged. This does not introduce CLAUDE_CONFIG_DIR support.

The complete install-status regression retains every Hooks/MCP/Capture/no-store assertion and now includes the doctor's status/stdout/stderr in failures. Four pure selector regressions cover absolute, empty, relative and missing overrides.

Local verification of `a345c8ba`: five-file rustfmt and whitespace checks, file-size guard, all seven actual caller-guard builders, and the target-export scanner passed. All 25 previously published source files were verified unchanged. Independent review found no blocker. The no-default-feature Linux runs passed all **5 host-root library tests** and **7 install-status CLI tests**. The four Windows selector cases exercise pure selection logic on Linux; they do not substitute for the real Windows resolver branch. On the selected ordinary CI run, **Check public and lifecycle surfaces** and its published-baseline check passed. The actual Windows profile fixture passed its Claude/Codex diagnostic assertions and then exposed the persistent-logging issue described below.

### Remaining public-evidence gate

The prior ordinary run also failed because committed public benchmark snapshots at schema 93 lack `worker_dispatch_state`, which the current schema-95 security verifier requires. The same 80 errors across 105 run artifacts / 605 files are present in the older `5a933689` run. No snapshot, migration, validator, or historical result was edited to suppress that failure. Updated, actually generated evidence is still required.

Native evidence at `a345c8ba4098952154f5168b7e558439fceeffa3` passed on all four platforms plus its aggregate in run 37529853379. That result covers the source before the doctor change below. It does not establish current authenticated evidence for the changed production fingerprint.


## Doctor diagnostics preserve the read-only store boundary

The Windows profile regression reached its final `!data_dir.exists()` assertion and found that an embedding-provider warning had created `data/remem.log`. This was a pre-existing diagnostic side effect: schema and database checks correctly reported no database, but the ordinary logger still prepared a directory, lock and persistent log. The fixture assertion, Windows model-root restrictions and provider warning are retained.

Ordinary synchronous doctor reporting now runs inside an internal thread-local scope that suppresses file logging. The shared log preparation path returns before directory, lock, rotation, append, permission or sidecar writes. Existing stderr mirroring and `REMEM_STDERR_TO_LOG` semantics remain unchanged, so errors and warnings remain visible. The log-health reader still inspects the original configured path and existing files. RAII restores the previous per-thread state after normal return, returned errors, nested calls and unwinding; other threads retain their normal logging. The independent `doctor truth` entry point is unchanged.

README and the existing current log-rotation PRODUCT/TECH contract describe this boundary. No version, schema, public API, evidence snapshot or authority validator changes are included in this correction.

### Focused validation of the doctor correction

- `cargo test --locked --no-default-features --lib log::tests::`: **20 passed, 0 failed, 1 existing subprocess helper ignored**. The four real filesystem regressions cover missing and existing logs, both append entry points, restoration after nesting/error/unwind and thread isolation. Child invocations of the existing subprocess helper are not counted as additional test cases.
- `cargo test --locked --no-default-features --test install_status`: **8 passed, 0 failed, 0 ignored** on Linux. The original seven cases and every original diagnostic/no-store assertion remain intact. The added real CLI case deliberately makes an API embedding credential unavailable, then requires a visible JSON warning and stderr error while the entire data directory remains absent.
- Independent source review found no blocking issue in the synchronous scope, shared write gate, restoration or CLI regression. The six changed Rust files pass direct format checking; whitespace and file-size checks passed. All seven actual caller-guard builders and the target-export scanner match the unchanged surface manifest, so this correction needs no caller fingerprint update.

These focused Linux results do not establish a green full preflight, real Windows execution of the correction or current native evidence. The PR remains draft while those required results are pending or failing. Full local preflight was not started because the shared workspace had approximately 1 GiB of free disk while another Cargo job remained active. Its unexecuted stages are pending; the focused and pure static results above are recorded separately. Full preflight must run with this intended body and the current source when resources permit. After this production correction is published, all four native platforms must generate fresh authenticated evidence for its production fingerprint before a separate evidence-only update can claim current results. Historical snapshots are not repaired by adding empty tables or weakening validation.

## Align active embedding test fixtures with the current source-hash contract

Source `9ab293eed7f3d9471aacb2886a0480d798378a4c` changes only two existing test fixtures under `src` (**16 insertions, 4 deletions**). Both now compute the active embedding row's hash using the production `memory_index_hash` helper and the exact canonical memory fields already inserted by the test. No production implementation, validator, expected assertion, inactive embedding row, test timeout or CI gate changes.

The complete production library run on the earlier `a345c8ba` source actually finished with **4,151 passed, 2 failed, 1 ignored** in 1,991.14 seconds. Its two failures were the provider-returned-profile coverage fixture using the literal `hash`, and the pinned-prune fixture using `feature-hash` for the active vector. The new freshness logic correctly classified both placeholders as stale. The prune test therefore stopped at its coverage precondition before reaching the deliberately blocked DELETE. The corrected fixtures retain the normalized/requested model and dimension differences, every coverage assertion, inactive row, DELETE blocking, model activation contention, cleanup sequence and `pruned == 1` assertion.

[Actual production run and failures](https://github.com/majiayu000/remem/actions/runs/37529950364/job/112496470282)

Independent review verified the two exact fixture-only transformations against immutable `479590b`, and checked the remaining integration/binary/doc targets for the same stale-fixture conflict without finding another necessary correction. Static review does not claim those later suites executed: the failed library stage prevented the preceding CI invocation from reaching them.

### Actual results and remaining limits

- Direct formatting, file-size and whitespace checks passed for the two changed files. All seven actual caller-guard builders match the existing manifest, and the target-gated scanner remains empty. No surface manifest regeneration is required.
- Both Windows jobs on `479590b` completed successfully. The runtime job actually ran the complete no-default-feature `install_status` binary: **8 passed, 0 failed, 0 ignored**, preserving the original invalid-profile/no-store check and the degraded-doctor regression. [Windows runtime](https://github.com/majiayu000/remem/actions/runs/37535135257/job/112514024319) · [Windows local embedding security](https://github.com/majiayu000/remem/actions/runs/37535135257/job/112514024410).
- The attempted local exact doctor test with the same CI profile, `--locked --no-default-features --features local-onnx`, exited during dependency compilation because the `ort-sys` native ONNX download connection was refused. **No test case executed in that attempt, and the pinned-prune test was not started.** No dependency version, feature gate or assertion was altered to bypass this limitation. The existing remote production profile remains required.
- All four native platform jobs and their aggregate passed on `479590b`. The original ZIP digests, source/run identities, receipts and payload bytes were authenticated. A clean real Rust verifier then passed the mechanically relocated canonical candidate, replaying **80 native security runs with zero policy failures** and all four required targets. [Historical native run](https://github.com/majiayu000/remem/actions/runs/37535135322).
- That `479590b` candidate was prepared as Git object `ca8670fd265917fc51c2e27100ededf74496b2d7` but **was never published to this branch or PR**. It remains historical: the production pathspec conservatively hashes the complete source files, including these test fixtures, so the two fixture edits require genuinely fresh four-platform generation and a new actual Rust verification. No older producer or payload will be relabeled.
- Full default preflight, final production/integration/eval suites and final-head ordinary/native CI remain pending. The current checked-in old public snapshots still require the authentic evidence refresh. Any local dependency/resource interruption is recorded separately from a passing test or preflight.

The earlier PRODUCT/TECH contracts already describe the source freshness behavior; these fixture corrections add no new user-facing behavior or API requiring another documentation contract change. This PR stays draft, and its final merge-readiness boxes remain unchecked until the applicable results and normal maintainer review are complete.

## Authenticated four-platform evidence for the final source

Evidence commit `70b2de0d414a1adeaf2623e00843bdf1ee6d35dd` replaces the four current v2 evidence groups with outputs genuinely generated from clean source `9ab293eed7f3d9471aacb2886a0480d798378a4c` in [native run 37538980481](https://github.com/majiayu000/remem/actions/runs/37538980481). All four native platform jobs and the aggregate completed successfully. The five downloaded ZIP files were independently matched to GitHub artifact IDs, sizes, SHA-256 digests, workflow run and exact producer before use.

The exact update is **80 run.json files plus 80 SQLite snapshots**. All 763 public paths remain present; 603 files are byte-for-byte unchanged, with no additions or deletions. The 80 databases are original schema-96 native outputs, including their actual worker-dispatch state. They have not been migrated, patched or otherwise modified during import. No production source, schema migration, validator, threshold, workflow, published surface record or baseline changes in this evidence commit.

### Preserve native provenance and established public paths

The established bare `adversarial-policy-v2` group remains the macOS ARM row, and `adversarial-policy-v2-linux-x86_64` remains the Linux x64 row. Their corresponding native target-qualified paths are relocated mechanically; all affected JSON references reverse exactly to the original native JSON bytes. The other two target groups retain their canonical names. Manifest/report bytes match the existing consumer layout, and all 480 referenced payload hashes match the actual native bytes. The original native receipts retain their original logical path bindings; they are not presented as receipts for relocated paths.

### Actual Rust verification of the imported candidate

A new real binary was built from clean `9ab293eed7f3d9471aacb2886a0480d798378a4c` using `cargo build --locked --no-default-features --features eval --bin remem`. Its actual build metadata reports that source SHA, `SOURCE_DIRTY=false`, production fingerprint `8a67b532f470a27466a1433c80c104f1b8397e67bbdcc82fd07ebac691a7a862`, and unchanged production pathspec fingerprint. The binary SHA-256 is `24eb8e5526a833cb9a6b85ee7a46ce2aaf31afa8738be4ba08145f0a217d74f4`.

That binary actually executed `remem bench verify` against the relocated canonical candidate and exited **0**:

- All **8 manifests, 8 reports, 105 run artifacts and 605 payload files** passed structural verification.
- Security replay completed **80 native runs** with **0 policy failures**. These are fixture executions across four platforms, not 80 additional unique unit tests.
- Authority, security and release-readiness verdicts reported **PASS**, with all four required targets present and no missing or stale targets.
- All **734 consumed-byte bindings** were checked against actual files; all **763 public file hashes** were unchanged before and after the verifier.
- Build and runtime checkout were clean and source-equivalent. The separate evidence commit preserves those exact production input bytes and retains the real 9ab producer, rather than relabeling its run as generated from the evidence commit.

The actual relocated verdict SHA-256 is `f3921b42edec8b0d9ae60450e18476aa47921a6a26e06ca7a1fda0e731534d63`; the execution-result SHA-256 is `185bad11901f5c991b47dc512bd035e808159800d2dfd72cc2bd21939cf0d4b5`. Independent review re-read the remote base, five ZIPs/four tar files, payloads, receipts, mechanical references and the real Rust result without finding a blocking issue. Metadata assertions and hash counts are not added to the unit-test totals.

### Full-suite and environment boundaries

On source `9ab293e`, both actual Windows jobs completed successfully: **18 worker tests and 8 install-status tests** passed, and the local embedding security job passed `cargo check --locked --all-targets` plus **59 local-semantic tests**. [Windows runtime](https://github.com/majiayu000/remem/actions/runs/37538980498/job/112527033477) · [Windows embedding security](https://github.com/majiayu000/remem/actions/runs/37538980498/job/112527033392).

The previous local same-feature test attempt was blocked during the native ONNX dependency download before executing any test. Full default preflight must use this exact intended PR body, the actual stacked base and final evidence head, through the unmodified repository script without `--fast` or skip flags. Its actual dependency/resource interruptions and unexecuted stages must be recorded separately; the eval-only authority PASS above does not establish a full default preflight PASS.

Final-head ordinary CI, both Windows jobs and the automatically triggered four-platform native workflow/aggregate remain required merge gates. Their outcomes are tracked on this exact evidence head. These offline fixture and software checks do not establish live-model extraction/recall quality, cross-host continuation, real-account or physical-device acceptance. Normal maintainer review and release decisions remain separate.



</details>
