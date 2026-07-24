# Task Plan

Status: Draft, needs human approval before implementation

## Linked Issue

GH-850（Refs #850；Epic #849）

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`

## Implementation Tasks

- [ ] `SP850-T1` Owner: human maintainer (evidence + security gate); Dependencies: none; Done when: a human reviews and explicitly adopts `docs/research/agent-memory-optimization-research-2026-07.md` at immutable commit `5492dc96` (PR #905) — or maintainer-approved equivalent evidence bound to an immutable revision — as the `B-001` prerequisite for this packet, and an independent human security review approves the enrichment generation, poisoning, redaction, and external-call boundary defined by `B-014`/`B-015` and tech §3 (SEC-11 mandatory-review area); Verify: GitHub comment or review on GH-850 naming the adopted revision and the security-review verdict.
- [ ] `SP850-T2` Owner: human maintainer (spec approval gate); Dependencies: `SP850-T1`; Done when: a human approves `product.md`/`tech.md` at the exact spec head (including the closed output shape in tech §3, the compatibility floor/epoch design in tech §2, the golden paraphrase minimum-gate design in tech §7, and the Planned Changes Manifest boundary), confirms the implementation-time next-free migration numbering policy, and explicitly moves GH-850 to `ready_to_implement`; Verify: GitHub `spec_approval` evidence plus GH-850 label/state change at the approved head.
- [ ] `SP850-T3` Owner: implementation agent; Dependencies: `SP850-T2`; Done when: the next-free additive migration adds the enrichment identity/claim/lease/failure columns and the `retrieval_enrichment_compatibility` singleton with monotonic floor/epoch triggers, production canonical writers reset identity plus deterministic fallback in the same statement, the canonical-convergence and FTS triggers persist fallback first and rebuild FTS only from the final persisted row, and `open_db`/retrieval/worker gates enforce the policy floor exactly as tech §2; Verify: migration/schema-drift/idempotency tests including the raw-canonical-UPDATE → persisted-fallback → unrelated-access_count-UPDATE → old-term-MATCH-0/new-term-hit/FTS-integrity-clean sequence, `cargo fmt --check`, `cargo check`.
- [ ] `SP850-T4` Owner: implementation agent; Dependencies: `SP850-T2`; Done when: `memory::retrieval_enrichment` implements snapshot loading, redacted single-memory prompt construction over the existing memory AI profile, the strict closed-JSON parser (bounds, Unicode/control/bidi, duplicate/empty keyword rejection), output secret redaction plus poison re-scan with whole-output rejection, and authoritative `search_context` composition under the existing 4000-char bound per tech §3; Verify: `cargo test memory::retrieval_enrichment` output-contract and poisoning positive/negative fixtures, log-redaction assertions (no payload/secret, closed error codes only).
- [ ] `SP850-T5` Owner: implementation agent; Dependencies: `SP850-T3` and `SP850-T4`; Done when: the idle worker lane runs `run_idle_retrieval_enrichment` before the existing embedding backfill with batch bound 16, durable conditional claim/lease/attempt before any AI call, hard timeout below lease duration, success and failure committed through the same source/generator/security/attempt/lease CAS with exponential backoff and closed error codes, and hooks/save/foreground writes never wait on generation or backfill per tech §4; Verify: `cargo test worker::tests` plus dual-worker single-AI-call, lease-takeover, late-success/late-failure-zero-rows, cancel/crash, and no-foreground-blocking tests.
- [ ] `SP850-T6` Owner: implementation agent; Dependencies: `SP850-T3`; Done when: FTS and the enabled embedding channel consume the same authoritative snapshot (versioned `memory_index_text` + index hash equal to `search_context_index_hash`), vector candidate/load applies the source identity gate, provider=off is an explicit diagnosable branch, and the new `Retrieval enrichment coverage` doctor check reports eligible/ready/pending/failed, versions, drift, and floor/epoch/state with DB errors failing closed per tech §5–6; Verify: focused FTS/vector channel tests (enrichment-only term hits index but never content/DTO bytes), `cargo test retrieval::memory_search retrieval::vector doctor`.
- [ ] `SP850-T7` Owner: implementation agent with explicit human authorization for the artifact lane; Dependencies: `SP850-T4` and `SP850-T6`; Done when: the human-authorized offline lane freezes `eval/retrieval-enrichment/generator-artifact.json` (production prompt/parser/executor, exact model revision, corpus/output hashes), CI replays the artifact with zero live AI and hash fail-closed, `min_value` thresholds make `golden.slice.paraphrase.hit_at_k`/`evidence_recall_at_k`/`mrr_at_10` strictly positive against the exact-main zero baseline, and `eval/retrieval-enrichment/report.json` records base/head SHAs and all gate results per tech §7; Verify: `cargo test eval::golden`, `cargo run -- eval-extraction --json --check-baseline`, `cargo run -- eval-gates --json-out /tmp/gh850-eval-gates.json`.
- [ ] `SP850-T8` Owner: verification agent; Dependencies: `SP850-T3` `SP850-T4` `SP850-T5` `SP850-T6` `SP850-T7`; Done when: every `B-001`..`B-018` row of the Product-to-Test Mapping passes, canonical byte-invariance is proven (DB `hex(content)`, render/API/MCP/pack snapshots), the implementation diff stays inside the Planned Changes Manifest, and no test weakening or secret is introduced; Verify: `cargo test`, `cargo clippy -- -D warnings`, `python3 scripts/ci/check_plugin_version_sync.py`, `python3 checks/check_workflow.py --repo . --spec-dir specs/GH850`, `git diff --check origin/main...HEAD`, manifest diff review.

No implementation task (`SP850-T3` or later) may start until `SP850-T1` and
`SP850-T2` have fresh human approval at the exact spec head. Autonomous agents
may not infer approval from this packet, from the merged research report, from
documentation, or from the existence of any spec PR.

## Parallelization

- `SP850-T1` and `SP850-T2` are serialized human gates owning evidence/state,
  not repository implementation files.
- After `SP850-T2`, `SP850-T3` and `SP850-T4` may run in isolated worktrees with
  disjoint ownership: T3 owns `src/migrations/**`, `src/migrate/**`,
  `src/memory/store/write.rs`, `src/db/core.rs`; T4 owns
  `src/memory/retrieval_enrichment.rs` and its prompt/parser tests.
- `SP850-T5` serializes after both; `SP850-T6` may overlap `SP850-T4` (owns
  `src/retrieval/**`, `src/doctor/**`) but not `SP850-T5`'s worker files.
- `SP850-T7` requires explicit human authorization for the artifact generation
  lane and must not run from hooks or CI.
- `SP850-T8` is read-only verification after all implementation tasks finish.

## Verification

- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH850`
- `git diff --check`
- Implementation-only after the human gates: `cargo fmt --check`, `cargo check`,
  focused tests per task, then `cargo test` and the full test plan in
  `tech.md` §测试计划.

## Handoff Notes

- The research report referenced by GH-850/#849 was missing at the original
  spec base `2dc41cb3`; it now exists at
  `docs/research/agent-memory-optimization-research-2026-07.md` (immutable
  commit `5492dc96`, merged via PR #905). `B-001` still requires a human to
  adopt that revision explicitly; this packet does not self-certify it.
- The migration number is deliberately unassigned (`v0NN`): implementation
  re-reads main at start time and takes the next-free slot, then syncs the
  manifest and schema tests before runtime changes.
- Quality evidence must come from the frozen production generator artifact;
  hand-written `search_context` fixtures prove channel wiring only and must
  never enter the paraphrase quality gate.
- Spec-only PRs use `Refs #850` / `Refs #849` and never close the
  implementation issue; final review, merge, and release stay human gates.
