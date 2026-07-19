# Task Plan

## Linked Issue

GH-894

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`

## Implementation Tasks

- [ ] `SP894-T1` Owner: coordinator; Dependencies: merged GH894 spec PR, fresh `origin/main`, and no covering open PR or matching remote branch for GH894; Done when: the #892 synchronized file set for upstream `0f903ab` is reapplied, every imported module is classified, synchronized blobs and modes match the lock, and no upstream-owned vendored module has a local-only patch; Verify: `scripts/sync-specrail-checks.sh --verify` and `git diff --check`. Covers: P1, P5, P8.
- [ ] `SP894-T2` Owner: coordinator; Dependencies: `SP894-T1`; Done when: the remem-owned `checks/schema_contract.py` consumes `schema_validation.SUPPORTED_KEYS` while preserving the local validator, vocabulary, exact diagnostic strings, and the absence of any `SchemaDefinitionError` translation; Verify: `python3 checks/check_workflow.py --repo .` and `python3 scripts/ci/test_schema_contract.py`. Covers: P2, P3, P4.
- [ ] `SP894-T3` Owner: coordinator; Dependencies: `SP894-T1`, `SP894-T2`; Done when: `scripts/ci/test_schema_contract.py` asserts local workflow/sync and upstream runtime outcomes independently, preserves every negative fixture, and proves malformed definitions newly rejected before instance evaluation raise `SchemaDefinitionError`; Verify: `python3 scripts/ci/test_schema_contract.py` and `python3 scripts/ci/test_specrail_gate_wiring.py`. Covers: P2, P3, P4, P6, P7.
- [ ] `SP894-T4` Owner: coordinator (exclusive verification owner); Dependencies: `SP894-T1`, `SP894-T2`, `SP894-T3`; Done when: the final implementation head passes focused tests, workflow checks, repeat sync verification, repository preflight, required Rust checks, PR CI, independent review, review-thread query, and PR gate with retained evidence; Verify: every command in the Verification section exits zero and the second sync verification leaves `git status --short` unchanged. Covers: P1-P8.

## Parallelization

Implementation tasks are serial because the resync changes the shared vendored
gate stack and the local contract/test updates depend on that exact upstream
revision. The coordinator owns all writes and all cargo commands in the current
worktree. Native planner and reviewer lanes are read-only, do not run cargo,
and do not perform GitHub writes.

## Verification

- `git diff --check`
- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH894`
- `scripts/sync-specrail-checks.sh --verify`
- `python3 scripts/ci/test_specrail_gate_wiring.py`
- `python3 scripts/ci/test_schema_contract.py`
- `cargo fmt --check`
- `cargo check`
- `cargo test`
- `python3 scripts/ci/check_pr_preflight.py --base origin/main --pr-body-file /tmp/pr-body.md`
- `gh pr checks <n> --repo majiayu000/remem --watch --fail-fast`
- Independent reviewer artifact, GraphQL review-thread evidence, allowed PR
  gate, and remote closure audit for only GH894.

## Handoff Notes

- #892 (`048d1c928f400488a887bd6287a6eefc4d72ab5e`) is the reviewed resync
  reference; #893 reverted it after CI exposed the schema-contract mismatch.
- The chosen contract is assertion update, not exception translation:
  `schema_contract.py` owns exact local diagnostics;
  `schema_validation.py` owns upstream `SchemaDefinitionError` and
  `InstanceMismatch` semantics.
- The initial reproduced CI failure was the old runtime-contrast expectation
  for the unsupported union member `["integer", "uint64"]`; strict upstream
  definition validation correctly rejects it before instance evaluation.
- This is a heavy-tier change. The spec PR must merge before implementation
  begins.
