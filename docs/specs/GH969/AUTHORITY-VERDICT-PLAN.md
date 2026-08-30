# GH969 / PR #1052 Authority-Verdict Convergence Plan

Status: local implementation and post-merge focused verification are complete
on `work/pr1052-authority-verdict`; the lifecycle manifest has now been
regenerated from current source. A fresh full preflight after regeneration,
hosted evidence, and review cleanup remain pending.

## Objective

Make one runtime Rust verifier the authority for public benchmark evidence. It
must consume the typed manifests, reports, suites, run artifacts, referenced
evidence files, and SQLite snapshots once, retain the exact bytes it consumed,
and emit one serializable `AuthorityVerdict`. The ship matrix, scorecard, and
Python public-claims checks will project that verdict and inspect wording; they
will not independently promote registry/report declarations to PASS.

This change is limited to the files authorized by the task. It does not create
unsupported macOS/Linux evidence, change package/release versions, or push.
README retains main's current #1051 navigation without superseded benchmark
material.

## Minimal architecture

1. `src/eval/bench_artifact` owns the verifier and authority boundary.
   `verify_benchmark_artifacts` remains the single entry point and returns a
   `BenchVerifyReport` containing the runtime-generated `AuthorityVerdict`.
   The verdict binds verifier/build identity, exact checkout/source identity,
   cleanliness, production-input tree, platform/config/model/condition
   identity, completeness, and hashes of every consumed byte.
2. Security policy is recomputed from the typed suite, typed memory runs,
   exact answer/retrieval evidence, and verified SQLite snapshot state. The
   verifier reuses the canonical memory-benchmark scorer/summarizer through a
   narrow crate-internal API; report aggregate metrics and run policy counters
   are evidence to compare, never authority.
3. GH931 is recomputed from the exact issue385-v1/official-v1 task matrix:
   16 tasks × 3 conditions × 3 run indices, with unique attempts and
   `target_started=true`. The existing task-cluster paired-bootstrap
   statistics and frozen registry thresholds, including memory-harm and
   stale-followed stop-losses, remain the calculation source.
4. Baseline generation, ship-matrix rows, and scorecard construction consume
   the already verified snapshot/verdict. Every row obtains model identity from
   its own covered runs/reports. Populations remain condition-specific;
   pre-target failures are excluded and an incomplete official matrix is
   `INSUFFICIENT`.
5. Registry data is policy/wording input only (comparisons, thresholds, and
   allowed/forbidden wording). Its `status` and `supporting_report` fields,
   and any duplicate claim contract, cannot authorize PASS. Synthetic latency
   remains unavailable and has no measured claim.
6. Release readiness is a closed four-target set: `x86_64-apple-darwin`,
   `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`, and
   `aarch64-unknown-linux-gnu`. Missing or stale genuine evidence makes the
   verdict false/unavailable. CI gets native-platform matrix scaffolding and
   must remain fail-closed until evidence exists. Legacy eval gates continue
   to participate.

## TDD slices

Each slice starts with a focused regression test, runs the test RED for the
intended reason, then implements the smallest production change and runs it
GREEN before refactoring:

- verifier/verdict serialization and one-pass snapshot ownership;
- report-path binding, exact consumed-source hashes, suite execution identity,
  source-equivalent history/build dirty state, real SQLite snapshots, and
  legacy-gate participation;
- recomputed security policy and aggregate stop-losses;
- exact GH931 matrix/attempt filtering, paired bootstrap, comparison/metric,
  and supporting-revision behavior;
- condition/population separation, per-row model identity, pre-target
  filtering, synthetic-latency unavailability, schema deduplication, and
  release fail-closed readiness;
- Python wording/claim comparison consumption of the emitted verdict and CI
  matrix structure.

## Implementation log

Record the exact command and outcome for each focused RED/GREEN cycle here.
No outcome is inferred from an unrun command or from CI; evidence supplied by
the commander is labeled separately from commands run in this implementation
session.

| Slice | RED command/outcome | GREEN command/outcome |
|---|---|---|
| Plan/contracts | — | — |
| Verifier/verdict boundary | `cargo test eval::bench_artifact::tests::verifier_emits_runtime_authority_verdict_with_consumed_byte_hashes -- --nocapture` — RED: failed because serialized `BenchVerifyReport` had no `authority_verdict` field | Commander-provided fresh Sol evidence: the same command — PASS, 1 passed, 0 failed |
| Source/evidence identity | `cargo test eval::bench_artifact::tests::authority::security_snapshot_validation_uses_consumed_bytes_after_source_removed -- --nocapture` — RED: 0 passed, 1 failed because validation reopened the removed source path and reported `open security SQLite snapshot: unable to open database file` | Same command — PASS, 1 passed, 0 failed; after moving the focused source file while retaining the parent namespace, `cargo test eval::bench_artifact::tests::security_snapshot_validation_uses_consumed_bytes_after_source_removed -- --nocapture` — PASS, 1 passed, 0 failed |
| Runtime identity/exact-byte/non-persistence | See the source/evidence RED above; the identity and persistence compile/behavior REDs were recorded in the worker session | Commander-provided fresh Sol evidence: `cargo test mismatched_build_and_checkout_identity_cannot_authorize_release -- --nocapture` — PASS, 1/1; `cargo test baseline_serialization_omits_runtime_authority_but_verify_json_keeps_it -- --nocapture` — PASS, 1/1; `cargo test verifier_emits_runtime_authority_verdict_with_consumed_byte_hashes -- --nocapture` — PASS, 1/1; `cargo test coding_context_audit_uses_consumed_sqlite_bytes_after_source_removed -- --nocapture` — PASS, 1/1 |
| Security authority | `cargo test eval::bench_artifact::tests::security_verification::tampered_ -- --nocapture` — RED: tampered report aggregate was accepted with overall `passed=true`; after tightening the run-declaration assertion, `cargo test eval::bench_artifact::tests::security_verification::tampered_run_policy_declarations_cannot_authorize_security_pass -- --nocapture` — RED: semantic validation rejected the declaration, but no typed `security.status=FAIL` verdict existed | `cargo test eval::bench_artifact::tests::security_verification::tampered_ -- --nocapture` — PASS, 2 passed, 0 failed |
| GH931 authority | `cargo test eval::bench_artifact::tests::gh931_authority -- --nocapture` — RED: 6 passed, 3 failed because a declaration-only `memory_hurt=true` was counted as a 2.0833% measured harm rate instead of 0%, a stale-use/declaration mismatch left `report.passed=true`, and a smoke-only curated run was rejected for a missing curator artifact; `cargo test eval::bench_artifact::tests::gh931_authority::mixed_or_malformed_official_provenance_is_insufficient -- --nocapture` — RED: 0 passed, 1 failed because a malformed producing SHA yielded `PASS` instead of `INSUFFICIENT` | Commander-provided fresh Sol evidence: `cargo test eval::bench_artifact::tests::gh931_authority -- --nocapture` — PASS, 10 passed, 0 failed; `cargo test eval::bench_artifact::tests::paired_statistics_ -- --nocapture` — PASS, 3 passed, 0 failed; `cargo test eval::bench_artifact::tests::security_verification::tampered_ -- --nocapture` — PASS, 2 passed, 0 failed |
| Populations/claims/release | `cargo test eval::bench_artifact::tests::authority_verdict_has_closed_four_target_release_set -- --nocapture` — RED: `authority_verdict.release.required_targets` serialized as null | Same command — PASS, 1 passed, 0 failed; readiness remains false for missing genuine target evidence |
| Rust consumer convergence | `cargo test eval::ship_matrix::tests::consumer_convergence -- --nocapture` — RED: 0 passed, 2 failed because the legacy claim authority promoted an insufficient GH931 verdict to a coding-row PASS and report aggregate fields made a failed security verdict measured; a later compile RED showed the missing condition completion, report-local model/platform, and verifier-release projections | Commander-provided fresh Sol evidence: `cargo test eval::ship_matrix::tests::consumer_convergence -- --nocapture` — PASS, 5/5; `cargo test eval::ship_matrix::tests -- --nocapture` — PASS, 26/26 |
| Schema/Python/CI | Earlier RED: `cargo test eval::bench_artifact::tests::schema -- --nocapture` failed because adversarial-policy v2 did not require `artifact_sha256`. Current wording-guard RED: `python3 scripts/ci/check_public_claims.py --self-test` — failed because a valid recomputed verdict was rejected with `coding-outcome superiority claim lacks verified registry authority and a hash-bound supporting report`. `cargo test eval::bench_artifact::tests::gh931_authority::complete_exact_matrix_reuses_registered_paired_statistics -- --nocapture` — compile RED (`Gh931ClaimVerdict` had no `allowed_wording` or `forbidden_wording`). After that narrow Rust projection, `cargo test eval::bench_artifact::tests::schema -- --nocapture` — RED: 1 passed, 2 failed because v2 `artifact_sha256.minProperties` was null and the duplicate coding claim contract still existed. | `python3 scripts/ci/check_public_claims.py --self-test` — PASS (`public claims check self-test: ok`); `cargo test eval::bench_artifact::tests::gh931_authority -- --nocapture` — PASS, 11 passed, 0 failed; `cargo test --lib eval::bench_artifact::tests::schema -- --nocapture` — PASS, 3 passed, 0 failed. Commander-provided fresh Sol acceptance repeats those outcomes and reports `cargo fmt --check`, `cargo check`, `python3 scripts/ci/check_file_size.py`, and `git diff --check` — PASS. Python projects runtime verdict wording/report bindings and the published-v2 schema is strict. |
| Native CI evidence wiring | During exploration, the temporary `python3 scripts/ci/check_native_benchmark_workflow.py` guard first exited 1 with `missing native benchmark workflow: .github/workflows/native-benchmark-evidence.yml`, then passed after the workflow was added. Sol rejected that guard as an over-designed hardcoded validator, so it was removed before acceptance together with its CI invocation; this preserves the historical RED/GREEN without treating the mechanism as accepted. | Direct PyYAML structural parse — PASS for `.github/workflows/native-benchmark-evidence.yml` and `.github/workflows/ci.yml`; direct extraction and `bash -n` review — PASS for all 10 Bash `run` blocks; `python3 scripts/ci/check_public_claims.py --self-test` — PASS (`public claims check self-test: ok`); scoped reference search — PASS (`removed native workflow guard: no file or CI/runtime references`). Workflow assertions consume only the Rust verdict; no four-platform job was run locally and no generated evidence is checked in. |
| Full-suite correction #1 | Commander-provided fresh Sol evidence: `cargo test` completed in 3295.47s with 4083 passed, 5 failed, 1 ignored; four invocation-isolation fixtures declared the stale checked-in 20-run policy aggregate after selecting one run, and the missing coding test-log diagnostic regressed. Focused worker RED: `cargo test public_verifier_replays_identical_input_on_every_invocation -- --nocapture` — FAIL, 0 passed, 1 failed, with declared 20-run policy counts differing from the recomputed one-run summary; `cargo test verifier_rejects_missing_coding_test_log -- --nocapture` — FAIL, 0 passed, 1 failed, because the established `artifact file for test_log is missing` substring was absent. The first post-edit invocation test attempt had a compile-only RED because the new JSON conversion needed an explicit `Vec<String>` type. | The four exact invocation tests each PASS, 1 passed, 0 failed: `cargo test public_verifier_replays_identical_input_on_every_invocation -- --nocapture`; `cargo test parallel_identical_public_verifier_invocations_have_thread_local_probes -- --nocapture`; `cargo test parallel_public_verifier_invocations_cover_distinct_task_and_platform_keys -- --nocapture`; `cargo test public_verifier_replays_same_task_id_after_suite_semantics_change -- --nocapture`. `cargo test verifier_rejects_missing_coding_test_log -- --nocapture` — PASS, 1 passed, 0 failed; `cargo test eval::memory_bench::tests::invocation_isolation -- --nocapture` — PASS, 4 passed, 0 failed; `cargo test eval::bench_artifact::tests::security_verification::tampered_ -- --nocapture` — PASS, 2 passed, 0 failed. The fixture now declares the canonical one-run policy summary from its task, replay measurement, and referenced run answer/retrieval; existing artifact files still use exact-byte consumption after the narrow missing-path diagnostic check. |
| Security authority review gaps | `cargo test eval::bench_artifact::tests::security_verification::security_report_requires_exact_suite_task_coverage_under_remem_default -- --nocapture` — RED, 0 passed, 1 failed: an omitted task produced only the stale aggregate mismatch and no explicit exact-suite coverage diagnostic. `cargo test eval::bench_artifact::tests::security_verification::leaked_protected_answer_text_fails_recomputed_security_policy -- --nocapture` — RED, 0 passed, 1 failed: the verifier returned `passed=true` for an exact consumed answer containing `sk-benchmark-secret-0001`. `cargo test eval::security_snapshot_identity::tests::snapshot_identity_rejects_malformed_volatile_timestamp_values -- --nocapture` — RED, 0 passed, 1 failed: Blob/Text/out-of-range volatile values were normalized without inspection. `cargo test eval::ship_matrix::tests::consumer_convergence::stale_security_report_tree_cannot_pass_current_implementation_gate -- --nocapture` — RED, 0 passed, 1 failed: the row was `Pass` instead of `Incomplete`. `python3 scripts/ci/check_public_claims.py --self-test` — RED, exit 1: a security-FAIL verdict did not block an otherwise passing coding claim. | The same exact-suite and leaked-answer Rust commands — PASS, 1 passed, 0 failed each; the first post-edit exact-suite attempt had a compile-only E0425 because the report helper used the wrong existing DTO name, corrected to `PublicBenchmarkReport`. `cargo test eval::security_snapshot_identity::tests -- --nocapture` — PASS, 2 passed, 0 failed. The same targeted stale-tree command — PASS, 1 passed, 0 failed; `cargo test eval::ship_matrix::tests::consumer_convergence -- --nocapture` — PASS, 6 passed, 0 failed. `python3 scripts/ci/check_public_claims.py --self-test` — PASS (`public claims check self-test: ok`). Existing authority checks remain green: `cargo test eval::bench_artifact::tests::security_verification::tampered_ -- --nocapture` — PASS, 2 passed, 0 failed; `cargo test eval::bench_artifact::tests::committed_public_fixture_passes -- --nocapture` — PASS, 1 passed, 0 failed. |
| GH931 raw-evidence review gaps | `cargo test eval::bench_artifact::tests::gh931_authority::exact_non_inferiority_ci_margin_is_inclusive -- --nocapture` — RED, 0 passed, 1 failed: an exact CI-boundary result was `FAIL` instead of `PASS`. `cargo test eval::bench_artifact::tests::gh931_authority::missing_treatment_maintenance_evidence_is_insufficient -- --nocapture` — RED, 0 passed, 1 failed: curator-only evidence produced `PASS` because treatment maintenance was hard-coded to zero. `cargo test eval::bench_artifact::tests::gh931_authority::cross_condition_model_mismatch_is_insufficient -- --nocapture` — RED, 0 passed, 1 failed: mixed condition models produced `PASS`. `cargo test eval::bench_artifact::tests::gh931_authority::official_tree_must_match_current_runtime_implementation -- --nocapture` — RED, 0 passed, 1 failed: a stale official tree produced `PASS`. `cargo test eval::bench_artifact::verify::coding::tests::official_failing_test_evidence_cannot_authorize_declared_resolution -- --nocapture` — RED, 0 passed, 1 failed because the verifier accepted the declared resolution and returned `passed=true`. `python3 scripts/ci/check_public_claims.py --self-test` — RED, exit 1: missing security report summaries did not block a coding claim. Standalone RED was not captured before implementation for the combined explicit-zero/measured-maintenance calculation or malformed zero-work test. Resumed correction RED: `cargo test eval::bench_artifact::tests::gh931_authority::zero_and_measured_treatment_work_recompute_maintenance_reduction --no-run` failed to compile because the multi-command result type, `remem_sessions`, and maintenance `session_count` fields did not yet exist; the first namespace GREEN attempt then exposed the interrupted patch's omitted enum field pattern and moved test fixture path, and the second exposed the remaining moved path. | Earlier timed-out-run outcomes actually completed: `cargo test eval::bench_artifact::tests::gh931_authority -- --nocapture` — PASS, 15 passed, 0 failed; `cargo test eval::bench_artifact::verify::coding::tests -- --nocapture` — PASS, 5 passed, 0 failed; `cargo test eval::bench_artifact::verify::coding::tests::official_failing_test_evidence_cannot_authorize_declared_resolution -- --nocapture` — PASS, 1 passed, 0 failed; `python3 scripts/ci/check_public_claims.py --self-test` — PASS (`public claims check self-test: ok`). Resumed correction GREEN: `cargo test eval::bench_artifact::tests::gh931_authority -- --nocapture` — PASS, 15 passed, 0 failed; `cargo test eval::bench_artifact::verify::coding::tests -- --nocapture` — PASS, 5 passed, 0 failed; `python3 scripts/ci/check_public_claims.py --self-test` — PASS (`public claims check self-test: ok`). Official GH931 resolution now requires a nonempty bound list of required command results and passes only when every command completes without timeout at exit zero. Maintenance uses bound supervisor-timed or internally consistent zero-work observations with positive `session_count`, and normalizes summed minutes by summed observed sessions. The CI margin is inclusive, provenance binds the current implementation tree and one exact model while allowing distinct producing commits, and Python requires zero-leak PASS summaries from the Rust verdict. |
| Static gates | `cargo fmt --check` — failed only on formatting of the new duplicate-contract schema assertion; `cargo fmt` completed successfully. Full-suite correction #1: the first `cargo fmt --check` reported only rustfmt changes in the corrected invocation-isolation fixture; `cargo fmt` completed successfully. | Commander-provided fresh Sol evidence after Rust consumer convergence: `cargo fmt --check` — PASS; `cargo check` — PASS; `python3 scripts/ci/check_file_size.py` — PASS (`file size check: ok`). Current worker evidence after Python/schema convergence: `cargo fmt --check` — PASS; `cargo check` — PASS; `python3 scripts/ci/check_file_size.py` — PASS (`file size check: ok`); `git diff --check` — PASS. Current worker evidence after native CI wiring: `cargo fmt --check` — PASS; `cargo check` — PASS; `python3 scripts/ci/check_file_size.py` — PASS (`file size check: ok`); `git diff --check` — PASS. Native correction #1 evidence: `cargo fmt --check` — PASS; `cargo check` — PASS; `python3 scripts/ci/check_file_size.py` — PASS (`file size check: ok`); `git diff --check` — PASS. Full-suite correction #1 worker evidence: `cargo fmt --check` — PASS; `cargo check` — PASS; `python3 scripts/ci/check_file_size.py` — PASS (`file size check: ok`). Security authority review-gap worker evidence: `cargo fmt --check` — PASS; `cargo check` — PASS; `python3 scripts/ci/check_file_size.py` — PASS (`file size check: ok`); `git diff --check` — PASS. GH931 raw-evidence review-gap worker evidence: `cargo fmt --check` — PASS; `cargo check` — PASS; `python3 scripts/ci/check_file_size.py` — PASS (`file size check: ok`); `git diff --check` — PASS. Resumed correction worker evidence: `cargo fmt --check` — PASS; `cargo check` — PASS; `python3 scripts/ci/check_file_size.py` — PASS (`file size check: ok`); `git diff --check` — PASS. Sol-provided current full-suite evidence: all focused suites, formatting, check, file-size, and diff-check passed; the first full `cargo test` had 4101 passed, 1 failed, and 1 ignored because of one local AI-stub timeout, and the exact test rerun passed in 1.06s. The full suite is not claimed as passed. |

### Current regeneration status

The final accepted corrections are present: security uses exact report
condition/task coverage and canonical safe-abstention binding; security and
GH931 bind the current implementation tree; official GH931 evidence uses typed
commands and maintenance observations with explicit session counts; all three
GH931 conditions share one model; the CI boundary is inclusive; and Python
consumes the Rust zero-leak summary.

The repository-owned `python3 scripts/ci/surface_lifecycle_discovery.py
--write-manifest` regenerated 5,226 records. Its only manifest delta is the
known `coding-public-benchmarks` implementation-caller fingerprint change;
published release, baseline, and lifecycle decisions were preserved.

Fresh full-preflight diagnosis: every listed gate passed except `Run eval
regression gates`. Its full `cargo test` passed with 4102 passed, 0 failed, and
1 ignored in the library suite; all integration and documentation tests passed.
`cargo run -- eval-gates --json-out
/private/tmp/remem-pr1052-eval-gates.json` reproduced exit 1 while every
deterministic metric row passed. The only required gate was
`production_security_e2e`, which was incomplete because the working tree/build
was dirty and not executable-source-equivalent; the JSON reported
`source_clean=false`, `build_source_dirty=true`,
`executable_source_equivalent=false`, and
`security report is not bound to the current verifier implementation tree`.
This is intended fail-closed authority behavior on an uncommitted production
tree, not a metric regression. A fresh full preflight is required after the
commit makes the source tree clean.

Earlier accepted verification: implementation commit `ac6676e7` passed complete
preflight before current `origin/main` at `0bf31e98` (including merged PR #1051)
was merged. README is byte-for-byte equal to `origin/main`, without restoring
superseded benchmark material. Post-merge focused documentation, preflight,
and public-claims tests passed, as did `--fast` preflight.

Current post-regeneration state: the first full preflight attempt had no
terminal result after session exit; one actionable lifecycle-manifest drift was
observed. A fresh full preflight remains required.

## Honest constraints and risks

- Fresh full preflight, commit/push, full hosted four-platform evidence, the
  aggregate hosted verdict, latest pushed CI, independent review, and
  review-thread cleanup remain pending. No missing platform evidence will be
  fabricated.
- PR #1051 is merged through main SHA `0bf31e98`; README now follows main's
  current navigation content without restoring superseded benchmark material.
- Existing historical report/spec material may describe earlier authority
  behavior. Current typed verifier code and tests are the source of truth for
  the completed local implementation.
