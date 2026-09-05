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

## Historical Issue Packets

Root-level `specs/GH<issue-number>/` packets are retained as historical product,
technical, and task-planning evidence. They are not an active workflow or an
execution prerequisite. Current Remem contracts live in this `docs/specs/`
directory and must be updated when the behavior they govern changes.
When the same `GH<number>/` exists in both trees, the structured handoff table
below is the machine-readable authority. The documentation contract check
requires exact current and historical paths and verifies that both endpoints
exist; the `docs/specs/` copy remains canonical.

### Current/Historical GH Packet Handoffs

| Contract | Canonical current packet | Historical packet |
|---|---|---|
| `GH931` | `docs/specs/GH931/` | `specs/GH931/` |
| `GH932` | `docs/specs/GH932/` | `specs/GH932/` |
| `GH933` | `docs/specs/GH933/` | `specs/GH933/` |
| `GH934` | `docs/specs/GH934/` | `specs/GH934/` |

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
| `associative-multihop-fixtures/` | Implemented evidence contract | Associative entity-hop golden fixtures used by the literal graph decision gate. GH-853 supplies the trusted `graph_edges` arm and same-head production decision. Refs #676, #853. |
| `cache-stable-injection/` | Current contract | Product and technical contract for a deterministic, prefix-cache-stable context block: byte-identical renders for unchanged memory state, additive prompt-time injection, and churn evals. Refs #673. |
| `context-budget-config/` | Current contract | SessionStart numeric budgets live in `config.toml` `[context]`; `REMEM_CONTEXT_*` remains an env escape hatch with the previous parse rules. |
| `pricing-config/` | Current contract | USD cost overrides live in `config.toml` `[pricing]`; `REMEM_PRICE_*` remains an env escape hatch. Init writes an empty table and does not pin compiled family rates. |
| `candidate-auto-promotion/` | Current contract | Closed candidate risk rubric, claim-level source support, phrase-specific secret blocking, and controlled observation-path promotion for directly supported failure lessons. Refs #955, #942. |
| `capacity-eval-axis/` | Current contract | Product and technical contract for the retrieval-quality-vs-store-size degradation curve: seeded corpus synthesis, per-channel metrics, and a regression budget wired into eval gates. Refs #675, #384. |
| `current-memory-contracts/` | Current contract | Product and technical contract for converging existing memory truth, temporal facts, injection audits, usage feedback, staleness labels, automatic lifecycle maintenance, observability, and host/app boundaries without a second rewrite. Refs #381, #383, #384, #385, #390, #945, #948. |
| `specs/GH823/` | Implemented historical reference (PR #922; Stop wiring consumed by PR #924) | Historical issue packet for the Cursor host runtime protocol: strict identity/event parsing, pre-capture PII removal, bounded nested tool-field decoding, verbatim generic capture, #825-gated stop transcripts, and the #822 real-host/MCP evidence gate. Refs #823, #821, #822, #825. |
| `specs/GH824/` | Implemented historical reference (PR #927; contract v1 ships MCP-only, hook capability gates closed) | Historical issue packet for Cursor install-host integration: exact owned hook entries, capability-gated registration, conflict-safe config mutation, runtime readiness, and doctor-visible partial support. Refs #824, #821, #822, #823, #825. |
| `specs/GH825/` | Implemented historical reference (PR #924) | Historical issue packet for lossless Cursor transcript capture: Stop-keyed immutable snapshots, physical-record ordinals, full-first arbitration, status-aware rollups, and current/history diagnostics without new schema. Refs #825, #821, #822, #823, #824. |
| `GH933/` | Current contract (bounded v1 production precursor released; v2 hardening pending) | Normative five-file contract for the `remem::truth` CurrentTruth projection, executable migration, rehearsal, and rollout. Version 0.6.81 has a focused `remem doctor truth` diagnostic plus bounded, fail-closed Context Bundle/SessionStart consumption with same-subject winner relations. This is a precursor, not completion of Phase B's broader shared-epoch/separate-output/history/rollback acceptance. Pending breaking v2 adds typed owner/scope identity, auditable reference epochs/replayability, Observation evidence and quarantine safety, external-content trust caps, owner-safe historical suppression, bounded relation/evidence reads, FK-safe v1→v2 migration, and the narrow request/result/seal plus route/lifecycle writer convergence required for exact history. Typed v2 integration, worktree/task scope, and general writer convergence remain later phases, so #933 stays open. Refs #933, #969. Historical planning packet: `specs/GH933/`. |
| `GH969/` | Current stabilization contract (repository-local close audit complete) | Canonical cross-cutting contract and [close audit](GH969/CLOSE-AUDIT.md) for active-memory activation safety, trust and surface lifecycle vocabulary, the current production/experimental/recovery inventory, distinct merge/release/default/public-claim gates, dependency-direction no-expansion enforcement, outcome scorecards, migration/rollback, and architecture/spec reconciliation. #1040/#1041, #1042/#1043, #1044/#1045, and #1049/#1052 implement the four runtime/governance slices; #1057 closes the documentation guard prerequisites; #1062 restores exact-main native evidence authority after the squash merge. The #1050 close audit records the explicit cyclic-component contract amendment and keeps #931/#933/#935 independent and parked. Refs #969, #1040, #1042, #1044, #1049, #1050, #1052, #1057, #1062, #931, #933, #935. |
| `GH932/` | Current contract (Context Bundle v1 implemented; API remains experimental) | Product and technical contract for Context Bundle v1: versioned ContextRequest/RetrievalPlan/ContextBundle/ContextAudit DTOs, deterministic plan/execute split, effective-loader-policy-bound plan hashes, executable trust/abstention policy, complete redacted poisoning-drop audit, title-aware budgets, DB-backed compilation, byte-compatible SessionStart consumption, payload-free doctor capability reporting, an experimental versioned MCP `context_bundle` tool with closed schema and legacy-text compatibility, durable hash-bound SessionStart audit rows, coding-bench plan/audit evidence, and bounded CurrentTruth v1 consumption. REST exposure is a v1 non-goal; breaking CurrentTruth v2 integration remains follow-up work under #933, not a #932 blocker. Refs #932, #933, #934. Historical planning packet: `specs/GH932/`. |
| `GH934/` | Current contract (planning/MCP projection landed; original acceptance partial) | The landed slices provide six task-routable `ContextIntent` variants, explicit lifecycle `SessionStart` planning, deterministic mappings, a versioned `RetrievalPlan`, `remem context-plan`, and an optional routed MCP `search` projection onto existing loaders. The original #934 acceptance still lacks the complete plan-controlled per-channel executor and execution-evidence propagation, per-intent goldens, static-vs-router ablation/default decision, and closure audit. GitHub issue #934 is currently closed, but that remote state is not completion evidence. Refs #934. Historical planning packet: `specs/GH934/`. |
| `GH935/` | Current contract (v1 infrastructure; completion unimplemented) | Product and technical contract for the Claude Code ↔ Codex cross-host continuity benchmark. The shipped surface is still 24 skeleton tasks plus offline schema/scanner/dry-run infrastructure with no live runs or public result. The completion contract adds one-source/many-condition sealing, same-path project identity, source-native import pairing, partial evidence, deterministic hash-bound reporting, and an explicit production user-identity prerequisite. Refs #935, #849, #852. |
| `GH953/` | Current contract (#953 closed after stage S1; deeper convergence remains future work) | Product and technical contract for converging SessionStart injection with the evaluated retrieval engine. The closed #953 slice centralizes scoring weights in `SearchWeights`, shares weighted fusion with retrieval search, and lets weight changes affect injection. Usage-channel calibration is also wired. The documented S2-S5 shared-engine/profile work is not implemented, and graph expansion remains explicitly gated until retrieval-side evidence justifies adding it to the latency-sensitive injection path. Refs #954, #953, #942. |
| `GH981/` | Current contract (extended by GH932) | Product and technical contract for truthful annotations on all 15 MCP tools, object-rooted output schemas and structured content on the 14 JSON tools, byte-for-byte legacy text compatibility, explicit Markdown handling, and fail-closed registry completeness. The experimental `context_bundle` extension is covered by GH932; Glama re-introspection remains post-release external evidence. Refs #981, #932. |
| `GH949/` | Current contract | Product and technical contract for centralized SQLite connection tuning: a 64 MiB per-connection cache target, fail-closed operator overrides, `FULL` durability by default with explicit WAL `NORMAL` opt-in, in-memory temporary storage, and an encrypted release-mode A/B harness. Refs #949, #942. |
| `GH931/` | Current contract (primary local adapters landed; governed execution pending) | Contract for the flagship E2E public proof: `remem_e2e` / `curated_file_budgeted` / `no_memory` official matrix. The local runner now has closed raw-event projection, transactional production capture/drain/SessionStart audit binding, budgeted curator input verification, and maintenance-cost reporting. Disjoint smoke identities, complete evaluation-clock propagation, reviewed live approval, independent scorer isolation, two-ruleset ledger protection with TUF/Rekor anchoring, deterministic memory-harm classification, official runs, and hash-bound public wording remain pending; PR #1052 makes the verifier verdict the local authority boundary. Refs #931, #384, #385, #928, #1052. Historical planning packet: `specs/GH931/`. |
| `failure-lifecycle/` | Current contract | Product and technical contract for failed pending-observation, extraction-task, replay-range, and job lifecycle: transient/permanent taxonomy, bounded auto-recovery, idle-only drain of actionable legacy pending rows into the current capture/extraction pipeline, doctor-visible admin-required archived legacy rows with exact transactional recovery, retention/archiving, cleanup-safe history, actionable-vs-history doctor split, exact range list/retry/quarantine, terminal replay evidence, dual explicit acknowledgement for archived quarantine recovery, and exact-profile worker dispatch. Refs #681, #864, #943. |
| `hook-binary-split/` | Current contract | Dedicated `remem-hook` entry compiles without `eval`/`local-onnx`; install prefers a sibling slim binary for hook commands and leaves `rules eval`/MCP on `remem`. |
| `host-native-memory/` | Current contract | Product and technical contract for host-native memory data sources: read-only Codex rollout-summary import into the candidate review queue (closed format set, plan-digest-bound apply, secret-blocked batches), Claude native topic-file ingestion via review candidates with self-ingest exclusion, and an explicitly no-go Claude `autoMemoryDirectory` bridge pending real-host PoC. Refs #852, #849. |
| `high-fidelity-episode-evidence/` | Current contract | Product and technical contract for opt-in preserved source slices that make public benchmark and debugging failures distinguish missing evidence from retrieval/ranking, policy, or downstream task failures. Refs #626, #384, #385. |
| `issue385-coding-agent-ab/` | Current contract | Product and technical contract for the coding-agent A/B benchmark harness. Condition naming and the claim-bearing primary matrix are superseded by `docs/specs/GH931/` (current `remem` → `remem_seeded_sessionstart`; historical full-body artifacts → `remem_preloaded`; `curated_file` → `curated_file_expert`). Refs #385, #931. |
| `legacy-observation-retirement/` | Current contract | Product and technical contract for inventorying legacy observation surfaces, deciding retire-vs-freeze per surface, draining residual pending rows through the bounded current-pipeline bridge, recovering one archived admin-required row by exact ID, and making the retained `events` compatibility projection transactionally linked and idempotent without reviving legacy enqueue/claim APIs or losing data. Refs #684, #943, #992. |
| `log-rotation-hardening/` | Current contract | Product and technical contract for cross-process-safe `remem.log` rotation, configurable retention, bounded lock fallback, worker stderr preparation, and doctor-visible log-health diagnostics. Refs #670. |
| `local-semantic-embedding/` | Current contract | Product and technical contract for a real local semantic embedding model, provider/fallback selection, same-model cosine and backfill rules, measured activation, and downstream dedup/preference adoption. GH-714/GH-715/GH-716/GH-717 are complete through PRs #719, #728, #731, #732, #733, #734, and #735. The refreshed GH-716/#946 release-mode report contains a verified local row at the default k=5: paraphrase evidence recall is 1.00 vs feature-hash 0.00, provider-comparison recall is 0.75 vs 0.00, abstention remains 10/10, warm local query-embedding p95 is 12 ms, and cold provider verification plus first profile probe is 5431 ms; the row records the exact model artifact SHA-256. The API row and cold-start budget remain blockers for an unconditional default flip. #946 instead conditionally resolves `Auto` to an already downloaded verified default local model before feature-hash and adds an explicit provider-switch backfill hint. GH-717 keeps downstream consumers in the active model space with model-specific thresholds. Refs #682, #946. |
| `memory-poisoning-defense/` | Current contract | Product and technical contract for write-time instruction-pattern quarantine, source trust classes on candidates and memories, injection-time re-scan with loud drops, Dream generated-output quarantine bound to exact source-memory provenance, the explicit v077 pre-v076 Dream active-memory backfill with immutable in-place restore binding, and production-path adversarial evaluation with database-measured governance state. Refs #672, #969, #990, #991. |
| `preference-rule-compilation/` | Current contract | Product and technical contract for compiling high-confidence, machine-checkable preferences into deterministic hook-evaluated rules with provenance, warn-first actions, and CLI overrides. Refs #671, #383. |
| `procedure-skill-export/` | Current contract | Product and technical contract for review-gated export of mature procedures to Claude skills, Codex prompts, and repo runbooks, with a doctor drift back-link and a hard no-background-writes guard. Refs #680. |
| `project-memory-pack/` | Current contract | Product and technical contract for deterministic git-committable project memory packs: export, provenance-aware merge import, pack trust class, round-trip integrity. Refs #678. |
| `public-memory-benchmark/` | Current contract | Product and technical contract for public benchmark evidence layers: memory-system capability proof, #385 coding-agent outcome proof, artifact schemas, reproducibility, claim levels, and stop-loss gates. Refs #384, #385, #629-#638. |
| `raw-session-ingestion/` | Current contract | Path-stable transcript identity, lossless occurrence ingestion, validated read-only raw queries, role counts, and privacy-safe fixed-window archive reconciliation. Refs #871, #720. |
| `retrieval-enrichment-budget/` | Current contract | P0 contract for deferring upgrade-time historical enrichment, bounding once/daemon AI batches, exhausting repeated failures, and avoiding false USD prices for GPT-5.6 Codex credit models. Extends GH-850. |
| `review-queue-throughput/` | Current contract | Product and technical contract for review-queue health metrics, block-reason aggregates and deadlock surfacing, batch review operations with previews, and a fast sequential review flow. Refs #683. |
| `refine-session-consumer/` | Current contract | Stable host, session reference, content fingerprint, and bounded raw-session listing for Refine without a second transcript store. |
| `session-observatory/` | Current contract | Rebuildable turn-level session activity derived from raw messages and captured events, with evidence-aware statistics and Remem app views. |
| `session-intent-display/` | Current contract (spec-only; phased implementation) | Session/workstream intent taxonomy separate from `memory_type`, Asia/Shanghai `MMDD` from created time, scannable `{MMDD}｜{INTENT}｜{topic}` labels, abstain-over-guess, and no host conversation title mutation. Refs #1065, #1066, #1067, #1068, #1069, #603. |
| `spec-lifecycle-governance/` | Current contract | Product and technical contract for separating epic, spec, and implementation issue lifecycles. Refs #592. |
| `status-health-performance/` | Current contract | Product and technical contract for splitting fast API liveness from cached aggregate status diagnostics. Refs #588. |
| `summary-candidate-promotion/` | Superseded reference | Original #674 survey contract for the summary-path promotion stall. Superseded by `summary-promotion-gate/`; keep for evidence and rationale only. Refs #674, #381, #383. |
| `summary-promotion-gate/` | Current contract | Product and technical contract for a source-path-aware auto-promote gate on summary-derived candidates: source_kind split, shadow-then-enforce rollout, doctor observability, and candidate-specific captured-event binding. Refs #674, #942. |
| `user-context-layer/` | Current contract | Product and technical contract for auditable user-level context: manual claims, editable profile summaries, suppression/feedback, on-demand recall, and guarded automatic extraction. Refs #574-#579. |
| `user-memory-policy-refinements/` | Current contract | Product and technical contract for profile Markdown snapshots, natural usage policy, and automatic extraction non-retention rules. Refs #617-#620. |
| `workstream-identity-continuity/` | Current contract | Product and technical contract for preserving canonical workstream identity across title drift, aliases, and rename chains. Refs #603. |

| `legacy-unverified-context/` | Current contract | Read-only trust/visibility projection, recovery labels, and CurrentTruth/SessionStart quarantine boundary. Refs #1017. |

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
