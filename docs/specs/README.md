# remem Specs Index

This directory is mostly historical implementation evidence, not a raw backlog.

Many `SPEC-*.md` files were written before or during implementation. Some still say `proposed` or `active` in old frontmatter, but the current repository already contains the corresponding migrations, modules, tests, plugin runtime scripts, or README behavior. Always verify against current code before treating an old spec as work to do.

## Status Rules

| Status | Meaning |
|---|---|
| Current contract | Read before implementing related work; update as behavior changes. |
| Implemented reference | Keep for rationale and validation history; do not re-execute as backlog. |
| Superseded reference | Useful for background only; prefer the newer architecture/code path. |
| Strategy reference | Product or distribution direction; individual items may be partially done. |

For new substantial work, prefer `docs/specs/<id>/PRODUCT.md` and `docs/specs/<id>/TECH.md` instead of adding another long root-style `SPEC-*.md`.

## Spec Lifecycle

Specs are contracts, not proof that implementation is done.

Use this handoff for substantial behavior, API, DB, hook, plugin, or
cross-module architecture work:

1. Create an epic or feature issue for the user-visible capability.
2. Open a spec PR for `docs/specs/<id>/PRODUCT.md`, `TECH.md`, and this index.
   Spec-only PRs use `Refs #...`, not `Closes` / `Fixes` / `Resolves`, unless
   the linked issue is explicitly only about writing the spec.
3. Create or link implementation issue(s) with file scope, acceptance criteria,
   and test commands.
4. Implementation PRs may close implementation issues after code, tests, docs,
   and smoke checks land.
5. Close the epic only after all acceptance criteria are actually verified.

The CI lifecycle guard enforces the highest-risk parts of this flow. See
`spec-lifecycle-governance/` for the full contract.

## Current Reading Order

1. `README.md` for user-facing installation and command behavior.
2. `docs/ARCHITECTURE.md` for current module and data flow.
3. This index to decide whether an old spec is current, historical, or superseded.
4. The specific spec only if it matches the files you are changing.

## Current Spec Directories

| Directory | Status | Notes |
|---|---|---|
| `issue385-coding-agent-ab/` | Current contract | Product and technical contract for the flagship coding-agent A/B benchmark comparing no-memory, remem, and curated-file conditions. Refs #385. |
| `spec-lifecycle-governance/` | Current contract | Product and technical contract for separating epic, spec, and implementation issue lifecycles. Refs #592. |
| `status-health-performance/` | Current contract | Product and technical contract for splitting fast API liveness from cached aggregate status diagnostics. Refs #588. |
| `user-context-layer/` | Current contract | Product and technical contract for auditable user-level context: manual claims, editable profile summaries, suppression/feedback, on-demand recall, and guarded automatic extraction. Refs #574-#579. |

## Top-Level Specs

| File | Status | Notes |
|---|---|---|
| `SPEC-audit-remediation-2026-05-29.md` | Implemented reference with per-item reverify | Several requested fixes have current implementation evidence, including all-status FTS, per-session raw archive dedup, API auth, migration drift tests, and state-key handling. Reverify the exact finding before reopening it. |
| `SPEC-benchmark.md` | Implemented reference | `tests/benchmark.rs`, `eval/golden.json`, `src/eval/`, and `remem eval/eval-e2e/eval-local` provide the benchmark/eval surfaces. |
| `SPEC-core-refactor-2026-03-26.md` | Historical reference | Core boundaries have since evolved into `src/project_id.rs`, service modules, retrieval modules, and capture/extraction pipeline code. Use code as truth. |
| `SPEC-eval.md` | Implemented/reference | LoCoMo remains informational; deterministic golden and local/e2e evals are the active gates. |
| `SPEC-growth.md` | Strategy reference, mostly implemented | README now documents Homebrew, GitHub Releases, crates.io, source builds, and prepared npm wrapper. Treat remaining channel/community items as strategy, not core runtime backlog. |
| `SPEC-memory-library-hardening-2026-05-16.md` | Implemented reference | The file already marks itself implemented; keep as rationale for memory library UX and governance behavior. |
| `SPEC-memory-system-v2-no-compat-2026-05-08.md` | Superseded/absorbed roadmap | The no-compat rewrite did not remain a single pending rewrite. Capture ledger, extraction tasks, memory candidates, current-state keys, retrieval, and context compiler pieces have landed in the current schema incrementally. |
| `SPEC-memory-system-v2.1-revisions-2026-05-08.md` | Superseded/absorbed roadmap | Use as background for host identity and rollout decisions. Do not start a new v2 rewrite from this file without a fresh PRODUCT/TECH pair. |
| `SPEC-observation-drain-scheduler-2026-05-05.md` | Superseded reference | Absorbed by capture/extraction task work and worker behavior. Current code treats legacy observation jobs as legacy. |
| `SPEC-raw-archive-vs-curated-memory-2026-04-22.md` | Implemented reference | Raw archive and curated memory are now separate concepts in migrations and code. Verify current behavior in `src/memory/raw_archive.rs`, `src/migrations/v002_raw_messages.sql`, and later raw ingest migrations. |
| `SPEC-web-api.md` | Current contract | remem-web read-only REST API contract for local authenticated dashboard endpoints. Update this when API behavior changes. |

## Refactor Step Specs

`docs/specs/refactor-steps/` contains completed split contracts from the large module-splitting pass. Treat them as historical implementation references unless a current file has drifted back into the exact problem described by that step.

Useful examples:

- API handler split specs correspond to `src/api/handlers/` modules.
- Retrieval and temporal split specs correspond to `src/retrieval/`.
- Memory promote/search/service split specs correspond to `src/memory/` and `src/retrieval/`.
- Eval split specs correspond to `src/eval/` and `eval/`.

## When To Add A New Spec

Add a new spec only when the work changes user-visible behavior, migrations, hook contracts, plugin runtime behavior, or cross-module architecture. For bug fixes with a clear root cause, a focused regression test and a short PR explanation are usually enough.
