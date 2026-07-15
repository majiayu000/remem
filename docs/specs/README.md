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

## SpecRail Issue Packets

This repository also carries a repo-local SpecRail workflow. New
issue-first/spec-first SpecRail packets use `specs/GH<issue-number>/product.md`,
`tech.md`, and `tasks.md` as declared in `workflow.yaml`.

Keep this `docs/specs/` directory as the remem implementation-contract index
and historical spec archive. When a SpecRail issue changes behavior already
covered by a current contract below, update the relevant `docs/specs/` contract
as part of the implementation or spec handoff.

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
| `associative-multihop-fixtures/` | Current contract | Product and technical contract for associative entity-hop golden fixtures and the two graph eval arms required before the graph decision gate can be re-evaluated. Refs #676. |
| `cache-stable-injection/` | Current contract | Product and technical contract for a deterministic, prefix-cache-stable context block: byte-identical renders for unchanged memory state, additive prompt-time injection, and churn evals. Refs #673. |
| `capacity-eval-axis/` | Current contract | Product and technical contract for the retrieval-quality-vs-store-size degradation curve: seeded corpus synthesis, per-channel metrics, and a regression budget wired into eval gates. Refs #675, #384. |
| `current-memory-contracts/` | Current contract | Product and technical contract for converging existing memory truth, temporal facts, injection audits, usage feedback, staleness labels, observability, and host/app boundaries without a second rewrite. Refs #381, #383, #384, #385, #390. |
| `specs/GH823/` | Draft, needs human approval | SpecRail packet for the Cursor host runtime protocol: strict identity/event parsing, pre-capture PII removal, bounded nested tool-field decoding, verbatim generic capture, #825-gated stop transcripts, and the #822 real-host/MCP evidence gate. Refs #823, #821, #822, #825. |
| `failure-lifecycle/` | Current contract | Product and technical contract for failed pending-observation, extraction-task, replay-range, and job lifecycle: transient/permanent taxonomy, bounded auto-recovery, retention/archiving, cleanup-safe history, actionable-vs-history doctor split. Refs #681. |
| `high-fidelity-episode-evidence/` | Current contract | Product and technical contract for opt-in preserved source slices that make public benchmark and debugging failures distinguish missing evidence from retrieval/ranking, policy, or downstream task failures. Refs #626, #384, #385. |
| `issue385-coding-agent-ab/` | Current contract | Product and technical contract for the flagship coding-agent A/B benchmark comparing no-memory, remem, and curated-file conditions. Refs #385. |
| `legacy-observation-retirement/` | Current contract | Product and technical contract for inventorying the legacy observation surfaces, deciding retire-vs-freeze per surface, and converging readers/writers onto the capture ledger without data loss. Refs #684. |
| `log-rotation-hardening/` | Current contract | Product and technical contract for cross-process-safe `remem.log` rotation, configurable retention, bounded lock fallback, worker stderr preparation, and doctor-visible log-health diagnostics. Refs #670. |
| `local-semantic-embedding/` | Current contract | Product and technical contract for a real local semantic embedding model, the provider/fallback config contract, same-model cosine and backfill rules, an eval-gated default flip, and downstream dedup/preference adoption. GH-714/GH-715/GH-716/GH-717 are complete through PRs #719, #728, #731, #732, #733, #734, and #735. GH-716 evidence: `eval/provider-comparison/report.json` keeps the default unchanged because local/API comparison rows are unavailable in the reference run. GH-717 keeps downstream consumers in the active model space with model-specific thresholds. Refs #682. |
| `memory-poisoning-defense/` | Current contract | Product and technical contract for write-time instruction-pattern quarantine, source trust classes on candidates and memories, and injection-time re-scan with loud drops. Refs #672. |
| `preference-rule-compilation/` | Current contract | Product and technical contract for compiling high-confidence, machine-checkable preferences into deterministic hook-evaluated rules with provenance, warn-first actions, and CLI overrides. Refs #671, #383. |
| `procedure-skill-export/` | Current contract | Product and technical contract for review-gated export of mature procedures to Claude skills, Codex prompts, and repo runbooks, with a doctor drift back-link and a hard no-background-writes guard. Refs #680. |
| `project-memory-pack/` | Current contract | Product and technical contract for deterministic git-committable project memory packs: export, provenance-aware merge import, pack trust class, round-trip integrity. Refs #678. |
| `public-memory-benchmark/` | Current contract | Product and technical contract for public benchmark evidence layers: memory-system capability proof, #385 coding-agent outcome proof, artifact schemas, reproducibility, claim levels, and stop-loss gates. Refs #384, #385, #629-#638. |
| `review-queue-throughput/` | Current contract | Product and technical contract for review-queue health metrics, block-reason aggregates and deadlock surfacing, batch review operations with previews, and a fast sequential review flow. Refs #683. |
| `spec-lifecycle-governance/` | Current contract | Product and technical contract for separating epic, spec, and implementation issue lifecycles. Refs #592. |
| `status-health-performance/` | Current contract | Product and technical contract for splitting fast API liveness from cached aggregate status diagnostics. Refs #588. |
| `summary-candidate-promotion/` | Superseded reference | Original #674 survey contract for the summary-path promotion stall. Superseded by `summary-promotion-gate/`; keep for evidence and rationale only. Refs #674, #381, #383. |
| `summary-promotion-gate/` | Current contract | Product and technical contract for a source-path-aware auto-promote gate on summary-derived candidates: source_kind split, shadow-then-enforce rollout, doctor observability. Refs #674. |
| `user-context-layer/` | Current contract | Product and technical contract for auditable user-level context: manual claims, editable profile summaries, suppression/feedback, on-demand recall, and guarded automatic extraction. Refs #574-#579. |
| `user-memory-policy-refinements/` | Current contract | Product and technical contract for profile Markdown snapshots, natural usage policy, and automatic extraction non-retention rules. Refs #617-#620. |
| `workstream-identity-continuity/` | Current contract | Product and technical contract for preserving canonical workstream identity across title drift, aliases, and rename chains. Refs #603. |

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
