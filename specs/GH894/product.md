# Product Spec

## Linked Issue

GH-894

## User Problem

remem needs the current vendored SpecRail gate stack, but the previous resync
made the repository's gate-wiring CI fail because the new upstream
`schema_validation` definition checks and remem's local published-schema
contract did not share an explicit compatibility contract. Maintainers cannot
safely resync while valid and invalid schema behavior is ambiguous across the
two validators.

## Goals

- Restore the SpecRail checks resync to upstream `0f903ab` or a newer reviewed
  revision without leaving main red.
- Preserve deterministic rejection of malformed JSON Schema definitions at
  both the vendored runtime boundary and remem's local workflow/sync boundary.
- Preserve remem's established local diagnostic contract for published and
  runtime-profile schema checks.
- Keep the vendored import closure and sync lock complete and reproducible.

## Non-Goals

- Expand either validator to the full JSON Schema 2020-12 vocabulary.
- Change application runtime, memory storage, API, hook, or migration behavior.
- Weaken negative fixtures or accept malformed schemas to make the resync pass.
- Fork or silently modify upstream-owned vendored files after synchronization.

## Behavior Invariants

1. A repository whose checked-in schemas satisfy both declared validator
   profiles passes workflow validation, schema-contract regression tests, and
   SpecRail sync verification after the resync.
2. A schema definition containing an unsupported JSON type, including
   `uint64` as a scalar or union member, is rejected before instance
   evaluation; the failure is never swallowed or treated as an instance
   mismatch.
3. remem's local workflow and sync validation retain their established,
   path-specific diagnostic text for malformed published or runtime-profile
   schema definitions.
4. The vendored runtime validator may expose its upstream
   `SchemaDefinitionError` diagnostic contract; regression coverage must
   distinguish that definition failure from an `InstanceMismatch`.
5. Every upstream-owned Python module required by the resynced checks is listed
   in the synchronized file set and lock, so a clean `--verify` run reports no
   missing or unclassified import.
6. Negative fixtures continue to exercise the complete local schema-contract
   matrix, including invalid types, duplicate type arrays, invalid keyword
   shapes, regex incompatibilities, and recursive child schemas.
7. A failed gate command exits non-zero and surfaces the complete actionable
   failure; no pipe, fallback, or warning converts a failed gate into success.
8. Re-running the sync verification from an unchanged checkout is deterministic
   and does not modify tracked files.

## Acceptance Criteria

- [ ] The vendored SpecRail checks and lock match upstream `0f903ab` or a newer
      reviewed revision, including the complete import closure.
- [ ] `python3 scripts/ci/test_specrail_gate_wiring.py` passes.
- [ ] `python3 scripts/ci/test_schema_contract.py` passes with all negative
      schema-definition cases still covered.
- [ ] `python3 checks/check_workflow.py --repo .` passes.
- [ ] `scripts/sync-specrail-checks.sh --verify` passes without changing the
      worktree.
- [ ] Focused assertions prove that `uint64` and other malformed definitions
      are rejected at the correct validation phase with the intended local or
      upstream diagnostic contract.
- [ ] The final implementation PR passes the repository's required CI checks.

## Edge Cases

- A type array that is empty, contains duplicates, or mixes a supported type
  with an unsupported type.
- Boolean child schemas that are permitted by the local published-schema
  profile but are not executable in a runtime profile.
- Schemas that are valid locally but deliberately outside the smaller upstream
  runtime subset.
- ECMAScript-only and Python-only regular expressions at the runtime boundary.
- A synchronized module added upstream without a corresponding lock or
  `SYNCED_FILES` entry.

## Rollout Notes

This is a heavy-tier gate-contract change. Land the complete spec packet first,
then land the resync and compatibility tests in a separate implementation PR.
Do not merge either PR without current green CI, independent reviewer evidence,
resolved review threads, and an allowed PR gate.
