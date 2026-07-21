# Task Plan

## Linked Issue

GH-851（Epic: GH-849）

## Spec Packet

- Product: [`product.md`](product.md)
- Tech: [`tech.md`](tech.md)

## 实现任务

- [ ] `SP851-T1` Owner: human maintainer and explicitly authorized evidence agent; Dependencies: explicit, scope-limited PoC authorization; Done when: the missing research report is restored and reviewed, the isolated PoC records exact model/config/hash, paired local A/B evidence, and separate cold/warm SessionStart budgets, and the maintainer grants `spec_approval` plus moves GH-851 to `ready_to_implement`; Verify: reviewable report path, PoC raw artifact hashes, written approval, and fresh GitHub label evidence — 关闭实现前置证据与人工 gate
  - Files: `docs/research/agent-memory-optimization-research-2026-07.md`, an explicitly authorized disposable PoC harness/report path, and no runtime wiring.
  - Before written PoC authorization, this task is evidence planning only: no model download, runtime edit, default change, schema change, or release asset change is allowed.
  - PoC completion does not itself grant `spec_approval`, `ready_to_implement`, implementation, merge, release, or default-on authorization.

- [ ] `SP851-T2` Owner: implementation agent; Dependencies: `SP851-T1`; Done when: a separate reranker preset/config and atomically verified local manifest inventory exist, runtime loading is network-free, and invalid top-N/top-k/input/deadline settings fail visibly; Verify: focused inventory/config tests, hash/path failure fixtures, and a network-deny model-load test — 建立本地模型资产与配置边界
  - Planned files: the existing embedding/model-management surfaces plus a separate reranker module and CLI action identified by `tech.md`.
  - The implementation must not reuse an embedding manifest as reranker evidence or download from search, API/MCP, SessionStart, or doctor paths.

- [ ] `SP851-T3` Owner: implementation agent; Dependencies: `SP851-T2`; Done when: standard curated-memory search sends the eligible, source-anchor-demoted fixed top-N baseline into one atomic rerank stage, applies the approved safety partition, returns one fixed top-k result set, and preserves the exact RRF baseline on off/failure paths; Verify: `cargo test rerank_shared_stage_top_n_membership`, `cargo test rerank_preserves_eligibility_and_source_anchor`, `cargo test rerank_fixed_result_pagination_contract`, and `cargo test rerank_off_is_baseline_equivalent` — 实现共享 rerank stage 与标准 search 接线
  - Planned files: `src/retrieval/rerank/**`, `src/retrieval/search/memory/**`, and `src/memory/service/search.rs`.
  - No new public field or API shape is declared outside the approved diagnostics contract.

- [ ] `SP851-T4` Owner: implementation agent; Dependencies: `SP851-T3`; Done when: SessionStart hybrid and recent candidates receive stable baseline ranks after dedupe/eligibility/policy, pass through the same final rerank stage, and cannot be reordered by a later branch/recent sort; Verify: `cargo test sessionstart_all_sources_use_shared_final_stage`, concurrency/cancellation fixtures, and off-baseline parity fixtures — 统一 SessionStart 与 search 的最终排序路径
  - Planned files: `src/context/query.rs`, `src/context/implicit_query.rs`, `src/context/hybrid_context.rs`, `src/context/memory_selection.rs`, and focused context tests.
  - Request-local cancellation, timeout, or inference failure must not publish a partial order or poison another request.

- [ ] `SP851-T5` Owner: implementation agent; Dependencies: `SP851-T2`, `SP851-T3`, `SP851-T4`; Done when: search/API/MCP/SessionStart expose one stable rerank outcome and timing contract, error paths log at error level, and doctor/status distinguish off, verified, missing, corrupt, and load-failed states; Verify: `cargo test rerank_diagnostics_contract`, `cargo test rerank_missing_or_corrupt_is_fail_visible`, doctor/API/MCP focused tests, and JSON contract snapshots — 接入诊断、timing、doctor 与公共调用面
  - Planned files: existing explain/perf, context diagnostics/stats, doctor, API, MCP, CLI, and service type surfaces listed in `tech.md`.
  - Diagnostics must not include query or memory content and must not be injected as memory text.

- [ ] `SP851-T6` Owner: evaluation agent; Dependencies: `SP851-T2`, `SP851-T3`, `SP851-T4`, `SP851-T5`; Done when: a fixed-hash rerank-off/on artifact proves all declared slice and combined non-regression gates, the preregistered combined metric improves by at least `0.05`, and separately approved cold/warm profiles pass their numeric budgets; Verify: `cargo test eval::rerank`, `cargo run -- eval-extraction --json --check-baseline`, `cargo run -- eval-gates --json-out /tmp/remem-eval-gates.json`, artifact schema/hash checks, and the approved benchmark command — 运行质量与性能批准门禁
  - The dataset, primary metric, slices, model/config, and budgets must be frozen before results are inspected.
  - A failed quality or p95 gate keeps rerank disabled and cannot be repaired by weakening an existing threshold.

- [ ] `SP851-T7` Owner: implementation coordinator; Dependencies: `SP851-T6`; Done when: focused and repository checks are fresh, version surfaces are synchronized if runtime/package behavior changed, a final implementation PR uses `Closes #851` only after every acceptance criterion is evidenced, and rerank remains default-off pending its separate gate; Verify: `cargo fmt --check`, `cargo check`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, plugin/version sync checks, full PR preflight, GraphQL review-thread evidence, and the SpecRail PR gate — 完成实现验证与独立交付 gate
  - The implementation PR must preserve human final review, merge, release, security, and default-on decisions.
  - GH-849 remains an umbrella epic and is not closed by this implementation slice.

## 并行拆分

`SP851-T1` 是所有实现工作的串行 gate。它完成且 maintainer 明确授予 `spec_approval` 和
`ready_to_implement` 后，`SP851-T2` 先固定模型/config/inventory 合同；`SP851-T3` 与
`SP851-T4` 共享最终候选边界，必须由同一 integration owner 顺序完成。`SP851-T5` 在两条调用路径
稳定后接入公共诊断面，`SP851-T6` 和 `SP851-T7` 继续串行消费固定实现 head。当前没有安全的并行
writable lane；只读 review/evidence lane 可以独立运行。

## 验证

- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir=specs/GH851`
- implementation 前以 fresh issue/duplicate evidence 运行
  `python3 checks/route_gate.py --repo . --route implement --issue 851 --state ready_to_implement --json`，
  结果不是 `allowed` 时停止。
- `B-001` 至 `B-018` 必须在 implementation PR 中逐项映射到 fresh diff、test、artifact 或 human-gate evidence。
- 不得删除或弱化现有 eval slice/threshold，不得以 warning-only fallback 隐藏模型或 rerank 故障。
- 本 spec PR 只运行文档/SpecRail focused checks；Rust build/test 属于未来 implementation PR。

## Handoff Notes

- GH-851 当前只有 `ready_to_spec`；本 task plan 只补齐可验证的 SpecRail packet，不代表
  `spec_approval`，不授权添加 `ready_to_implement`，也不授权 PoC、模型下载、runtime、schema、
  release asset 或 default-on 变更。
- 缺失的研究报告、exact model/config、本地配对 A/B、数字化 cold/warm p95 预算仍是显式 blocker。
- 当前 spec PR 使用 `Refs #851` 和 `Refs #849`；合并本 PR 不关闭任一 issue。
- future implementation 只有在 `SP851-T1` 全部 evidence/human gates 满足后才能开始，并应在独立 PR
  中以 `Closes #851` 表达最终 closure；GH-849 由其全部子项的独立完成状态管理。
- 选定 locale 为 `zh-CN`；稳定 IDs、路径、命令和 JSON keys 保持 English。
