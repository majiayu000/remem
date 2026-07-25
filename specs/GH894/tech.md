# Tech Spec

## Linked Issue

GH-894

## Product Spec

[`product.md`](product.md)

## Codebase Context

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| Local schema contract | `checks/schema_contract.py`, `checks/check_workflow.py` | remem validates a broad closed JSON Schema profile and returns stable path-prefixed error strings; runtime-profile schemas receive additional executable-subset checks | This local-owned layer defines repository and sync diagnostics and must remain stable across vendored updates |
| Vendored runtime validator | `checks/specrail_lib.py`; new `checks/schema_validation.py` from upstream | current main validates schema definitions and instances through the older `specrail_lib` implementation; upstream `0f903ab` moves strict two-phase validation into `schema_validation.py` and raises `SchemaDefinitionError` before instance evaluation | The upstream phase ordering is the source of the #892 compatibility failure and must remain upstream-owned |
| Schema regression matrix | `scripts/ci/test_schema_contract.py`, `scripts/ci/test_specrail_gate_wiring.py` | local tests prove exact local diagnostics and also encode which malformed definitions the old runtime validator happened to accept | The runtime-contrast expectations must describe the new upstream definition contract without weakening the local negative matrix |
| Vendored sync closure | `scripts/sync-specrail-checks.sh`, `checks/specrail-sync.lock.json`, synchronized `checks/*.py`, and synchronized `schemas/*.json` | main is pinned to upstream `75628b21`; #892 added ten modules and the corresponding lock entries for `0f903ab`, then #893 reverted them | The resync must be reproduced exactly and all imported upstream modules must be classified and locked |

## Proposed Design

Keep the two schema layers separate and make their ordering explicit:

1. `checks/schema_contract.py` remains remem-owned. It validates every
   repository schema during `check_workflow.py` and `sync --verify`, owns the
   broader adopted vocabulary, runtime-profile restrictions, ECMAScript/Python
   regex compatibility checks, and the existing exact local diagnostic text.
2. The resynced upstream `checks/schema_validation.py` remains vendored and
   unmodified. Runtime gate entry points call its `validate_instance`, which
   first validates the executable schema definition and raises
   `SchemaDefinitionError`; only a valid definition proceeds to instance
   evaluation, where data failures raise `InstanceMismatch`.
3. `checks/schema_contract.py` imports only upstream
   `schema_validation.SUPPORTED_KEYS` to define the executable runtime subset.
   It does not catch, translate, or replace upstream exceptions.
4. Update `scripts/ci/test_schema_contract.py` so each malformed runtime fixture
   records both contracts independently:
   - the local workflow/sync path must still fail with its established exact
     diagnostic;
   - the upstream runtime path either accepts the definition and evaluates the
     fixture instance, or rejects it with `SchemaDefinitionError` and a focused
     upstream diagnostic assertion.
5. Reapply the #892 synchronized file set and lock for the exact upstream
   commit `0f903abe1794899071a9f19a4c46af1ce81129d3`. Do not pull later
   unrelated gate changes into GH894, and do not hand-edit upstream-owned
   vendored modules after synchronization.

The assertion-update design is preferred over a translation adapter because
the two validators serve different callers and profiles. Translating upstream
exceptions in the local layer would couple an upstream runtime API to remem's
published-schema diagnostics, obscure the definition/instance phase boundary,
and require non-vendored wrapper plumbing in runtime call sites.

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| P1 | resynced checks, local contract, regression matrix | `python3 scripts/ci/test_specrail_gate_wiring.py` |
| P2 | upstream `schema_validation.py` phase ordering and focused fixtures | `python3 scripts/ci/test_schema_contract.py` asserts `SchemaDefinitionError` for unsupported scalar and union types |
| P3 | local `schema_contract.py` and workflow/sync assertions | exact local error substrings remain asserted by `assert_contract_failure` |
| P4 | runtime contrast matrix | focused exception-class and message assertions distinguish definition errors from instance mismatches |
| P5 | `SYNCED_FILES`, lock, import-closure validator | `scripts/sync-specrail-checks.sh --verify` |
| P6 | full `runtime_cases()` and published/locked-schema matrices | `python3 scripts/ci/test_schema_contract.py` |
| P7 | gate entry points and CI invocation | direct, unpiped commands return non-zero on injected failures |
| P8 | lock verification | run `scripts/sync-specrail-checks.sh --verify` twice and confirm a clean `git status --short` |

## Data Flow

```text
checked-in schema
  -> check_workflow.py
  -> local schema_contract.validate_schema_node
  -> stable list of local diagnostic strings

runtime schema + evidence instance
  -> vendored gate entry point
  -> schema_validation.validate_instance
  -> validate complete executable schema definition
       -> SchemaDefinitionError on malformed/unsupported definition
  -> evaluate evidence instance
       -> InstanceMismatch on data mismatch

sync verification
  -> lock/hash verification
  -> classified Python import-closure verification
  -> check_workflow.py
```

No application data, database persistence, network call, or user memory is
added by this change. GitHub is used only by existing evidence adapters during
their normal gate operation.

## Alternatives Considered

- Catch and translate `SchemaDefinitionError` into remem's local diagnostic
  strings. Rejected because the local validator is not the runtime caller,
  translation would blur upstream exception semantics, and the established
  local diagnostics already remain available through workflow/sync validation.
- Remove newly rejected cases from the runtime contrast set. Rejected because
  every malformed fixture must remain covered and its new phase outcome must be
  asserted explicitly.
- Patch `schema_validation.py` after vendoring. Rejected because it would break
  upstream hash parity and create untracked divergence.
- Keep the reverted `75628b21` gate stack. Rejected because it leaves remem
  without the required current SpecRail gate and evidence contracts.

## Risks

- Security: gate weakening could accept malformed review or runtime evidence.
  Mitigation: preserve the complete negative matrix and fail closed on
  definition errors.
- Compatibility: exact upstream exception text may change in a later resync.
  Mitigation: assert the exception class and focused semantic fragment for the
  upstream path, while keeping exact messages only for the remem-owned path.
- Performance: strict definition validation runs before each instance
  evaluation. The schemas are small local files; no new asymptotic work or
  external call is introduced.
- Maintenance: the two profiles can drift. Mitigation: import the upstream
  executable keyword set and keep explicit cross-profile fixtures.
- Sync integrity: a missing newly imported module can pass review but fail in
  CI. Mitigation: retain the strict classified import-closure check and lock
  every synchronized file.

## Test Plan

- [ ] Unit tests: run `python3 scripts/ci/test_schema_contract.py`, including
      focused assertions for unsupported scalar/union types and every existing
      negative fixture.
- [ ] Integration tests: run
      `python3 scripts/ci/test_specrail_gate_wiring.py`,
      `python3 checks/check_workflow.py --repo .`, and
      `python3 checks/check_workflow.py --repo . --spec-dir specs/GH894`.
- [ ] Sync verification: run
      `scripts/sync-specrail-checks.sh --verify` twice and confirm
      `git status --short` is unchanged after each run.
- [ ] Repository preflight: run
      `python3 scripts/ci/check_pr_preflight.py --base origin/main
      --pr-body-file /tmp/pr-body.md`.
- [ ] CI: run the required PR checks and wait with
      `gh pr checks <n> --repo majiayu000/remem --watch --fail-fast`.

## Rollback Plan

Revert the implementation PR as one unit, restoring the prior synchronized
files, lock revision, local import, and test expectations. Do not partially
revert only `schema_validation.py` or only the lock because that would leave
the vendored import closure inconsistent. The spec remains as the contract for
a corrected retry.
