# Retrieval Enrichment Budget — Technical Contract

Status: Current contract
Date: 2026-08-13

## Data Contract

Migration v083 adds
`memories.search_context_enrichment_state TEXT NOT NULL DEFAULT 'pending'`
with the closed values `pending`, `ready`, `deferred`, and `exhausted`.

The migration is data-only with respect to memory content:

1. Existing rows with the current generator version, current security policy,
   and a non-null source hash become `ready`.
2. Every other existing row becomes `deferred` and has any stale claim/lease
   cleared.
3. Inserts after the migration retain the `pending` default.
4. The `memories_au` convergence trigger is rebuilt so canonical source
   changes set `pending`, clear claims, and reset failure metadata.

An index beginning with `search_context_enrichment_state` supports the due-row
scan. The due predicate includes `state='pending'`, so both candidate selection
and transactional claim enforce the same admission rule.

## Retry State Machine

| Current event | Next state | Retry time |
|---|---|---|
| Valid success | `ready` | NULL |
| Failure 1 or 2 | `pending` | exponential backoff, capped at 900s |
| Failure 3 | `exhausted` | NULL |
| Canonical source mutation | `pending` | NULL |

Every transition keeps the existing owner/attempt/source/version/policy CAS.
A stale completion changes zero rows.

## Worker Admission

`run_idle_sweep` returns counts rather than a success boolean so the caller
can distinguish attempted work from successful work. The worker owns a
process-local enrichment schedule:

- once mode: one sweep admission per process;
- daemon mode: one sweep admission every 60 seconds;
- sweep size: four rows;
- the admission is consumed even when all claimed rows fail, preventing a
  failure-only tight loop.

The once-worker also owns a process-local budget of four work items and a
180-second admission deadline. Extraction tasks, Compress/Dream jobs, and
attempted enrichment rows consume that shared budget. Purely local cleanup,
rule compilation, and retired-job transitions do not consume AI budget. The
check runs between items and does not cancel a provider request already in
flight. Daemon mode does not use the process-lifetime item budget; its
enrichment lane remains rate-limited independently.

The schedule remains after current extraction tasks and durable jobs, and
before embedding backfill. Automatic capture remains unchanged.

## Diagnostics

Retrieval-enrichment coverage records `ready`, `pending`, `exhausted`, and
`deferred` separately. Only actionable `pending` or `exhausted` rows prevent a
fully healthy result. Deferred historical rows are informational because the
deterministic retrieval context remains available and this hotfix deliberately
does not advertise an unimplemented backfill command.

## Pricing Truth

`pricing_breakdown_for_model` returns `None` for `gpt-5.6-*` Codex credit
models after checking the global explicit pricing override and before applying
the generic GPT-5 family fallback. v083 rewrites only existing matching events
whose source is remem static pricing; operator override rows are preserved.

## Verification

- Migration tests: v082 upgrade classification, trigger reset, new-row
  default, pricing correction, and schema invariants.
- Retrieval tests: state-gated claim, success, bounded retry exhaustion, and a
  batch-size regression with more than four rows.
- Worker tests: once and daemon schedule admission.
- Run-budget tests: four shared work items, 180-second admission deadline, and
  unlimited daemon lifetime.
- Doctor tests: deferred-only state is healthy and actionable states remain
  visible.
- Pricing tests: GPT-5.6 credit models are unknown without overrides and
  respect explicit overrides.
- Repository gates: `cargo fmt --check`, `cargo check`, `cargo test`.
