# Task Plan

## Linked Issue

GH-934

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`
- Current contract: `docs/specs/GH934/{PRODUCT,TECH}.md`
- Existing partial implementation: original PR #940,
  branch `codex/issue934-retrieval-router`

所有任务保持未勾选。当前 `write_spec` route 已通过，但 `implement` route 因 packet 尚未落地、
duplicate evidence、trusted default base/path evidence 与 `sensitive_enforcement` 缺失而 blocked；
readiness label 与 spec approval 仍是 human gates。

## 实现任务

- [ ] `SP934-T1` — 完成人工 readiness/spec approval、原 PR ownership、duplicate 与 sensitive gate — Owner: human maintainer + queue coordinator; Dependencies: none; Done when: 见下; Verify: 见下
  - Owner: human maintainer + queue coordinator
  - Dependencies: none
  - Done when:
    - maintainer 在 exact spec head 批准 `product.md`/`tech.md`，确认 GH-934 可进入
      `ready_to_implement`；
    - fresh duplicate evidence 明确原 PR #940 / 原分支
      `codex/issue934-retrieval-router` 是 GH-934 唯一 writable implementation lane，禁止新建替代 PR；
    - 刷新 origin/main、PR #939/#940 与 #932 的 merge/base truth，确认 #940 的 stacked base、冲突
      处理顺序和 trusted default base ref/SHA；
    - 提供 trusted changed-path evidence、sensitive registry classification 与
      `sensitive_enforcement`，人工确认涉及 public API/MCP、benchmark schema 或其它敏感路径时的
      enforcement 结论；
    - `route_gate implement` 在携带 packet、duplicate evidence、trusted base/path 与 sensitive
      evidence 后返回 `allowed`；若 gate 仍为 `blocked`/`needs_human`，后续任务不得开始。
  - Verify:
    - `python3 checks/check_workflow.py --repo . --spec-dir specs/GH934`
    - `python3 checks/github_duplicate_evidence.py --github-repo majiayu000/remem --issue 934 --json`
    - `python3 checks/route_gate.py --repo . --route implement --issue 934 --pr 940 --state ready_to_implement --artifact product_spec=specs/GH934/product.md --artifact tech_spec=specs/GH934/tech.md --artifact task_plan=specs/GH934/tasks.md --duplicate-evidence <trusted-duplicate-evidence.json> --evidence <trusted-route-evidence.json> --mode required --json`

- [ ] `SP934-T2` — 在原 PR #940 对齐 Phase A、base 与 current contract — Owner: original PR implementation agent; Dependencies: `SP934-T1`; Done when: 见下; Verify: 见下
  - Owner: original PR implementation agent
  - Dependencies: `SP934-T1`
  - Done when:
    - 只在原 PR #940 原分支更新；#932 合并后按 maintainer 认可的顺序 retarget/merge/rebase，
      无 force push，冲突全量暴露并解决；
    - Phase A 的 `ContextIntent`、`RetrievalPlan`、15-channel enum、mapping、high-risk policy、
      stable plan hash 与 `context-plan` 仍满足 `B-001`/`B-002`；
    - `docs/specs/GH934/{PRODUCT,TECH}.md` 与本 packet 明确标注 Phase A partial 和剩余 acceptance；
      PR body 保持 `Refs #934`，不以 Phase A 关闭 issue；
    - version surfaces 与最终 base 唯一、同步，不保留 stacked branch 的冲突版本号。
  - Verify:
    - `cargo test retrieval_router -- --test-threads=1`
    - `cargo test context_bundle -- --test-threads=1`
    - `python3 scripts/ci/check_plugin_version_sync.py`
    - `git diff --check`

- [ ] `SP934-T3` — 实现 DB-backed channel execution、fusion、scope 与 abstention — Owner: retrieval implementation agent; Dependencies: `SP934-T2`; Done when: 见下; Verify: 见下
  - Owner: retrieval implementation agent
  - Dependencies: `SP934-T2`
  - Done when:
    - `src/retrieval_router/executor*` 只消费 planner 产生并通过 hash/schema validation 的 plan；
    - enabled channels 复用现有 typed readers，disabled channel zero-call；limit/weight/trust/validity/
      max-contribution/timeout/degradation 全部执行；
    - canonical/projection identity 去重、稳定 tie-break、weighted fusion、source-anchor/
      suppression/trust/freshness gate、high-risk canonical Top 1 与 abstention 符合
      `B-003`/`B-004`/`B-006`/`B-007`/`B-015`/`B-016`；
    - 无 SQL 拼接、无 public `Any`、无新增 foreground LLM/network，error/timeout/cancel 不静默降级。
  - Verify:
    - `cargo test retrieval_router::executor -- --test-threads=1`
    - `cargo test retrieval::search -- --test-threads=1`
    - `cargo test context::tests -- --test-threads=1`
    - `cargo check`

- [ ] `SP934-T4` — 把 execution identity 与 channel evidence 写入 ContextBundle/ContextAudit — Owner: context-contract implementation agent; Dependencies: `SP934-T3`; Done when: 见下; Verify: 见下
  - Owner: context-contract implementation agent
  - Dependencies: `SP934-T3`
  - Done when:
    - 新 router entrypoint 复用现有 Context Bundle scope/budget/audit helpers，旧 GH-932
      `ContextPlan` entrypoint 保持兼容；
    - bundle top-level hash、audit retrieval hash 与 executor plan hash 完全相同；
    - audit 记录 mode、intent、policy、per-channel counts/degradation/reasons、latency、
      abstention、token estimate，且不包含 memory 正文；
    - section mapping、duplicate canonical stable key 与 blocked/canonical-only/full JSON shape 被
      schema snapshots 锁定。
  - Verify:
    - `cargo test context_bundle -- --test-threads=1`
    - `cargo test retrieval_router -- --test-threads=1`
    - `cargo check`

- [ ] `SP934-T5` — 增加 MCP/REST 显式 intent Context Bundle surfaces — Owner: adapter implementation agent; Dependencies: `SP934-T4`; Done when: 见下; Verify: 见下
  - Owner: adapter implementation agent
  - Dependencies: `SP934-T4`
  - Done when:
    - MCP `context_bundle` 与 REST `POST /api/v1/context` 使用同一 typed request/service/executor；
    - explicit intent 始终优先，invalid schema/intent/role/risk/budget/scope 返回稳定
      invalid-request/4xx；DB/channel fail-closed 错误不伪装空成功；
    - capability map 声明 endpoint、schema 与 policy version；默认 log 只含 safe metadata/hash；
    - 现有 MCP/REST/CLI search 的 response/default 与 pagination 回归测试保持兼容。
  - Verify:
    - `cargo test mcp::server -- --test-threads=1`
    - `cargo test api -- --test-threads=1`
    - `cargo test memory::service -- --test-threads=1`
    - `cargo check`

- [ ] `SP934-T6` — 接入 GH-933 enrichment 并完成 corrupted/duplicate signal 防线 — Owner: retrieval security/data-quality agent; Dependencies: `SP934-T3`, `SP934-T4`, GH-933 merged; Done when: 见下; Verify: 见下
  - Owner: retrieval security/data-quality agent
  - Dependencies: `SP934-T3`, `SP934-T4`, GH-933 canonical projection implementation merged and exact anchors reverified
  - Done when:
    - generated enrichment 复用 GH-933 source-bound projection，不创建第二套 projection/storage；
    - wrong canonical ID、cross-project、missing source、duplicate projection、同 projection
      FTS+vector 双命中、malicious high score、timeout/provider error 的全矩阵 fixtures 完成；
    - generated signal 保留 attribution、受 cap、不能双重 canonical 加权，也不能成为 high-risk
      Top 1；验证失败按 plan degrade/block 并留下 reason；
    - 所有 fixtures 使用 fake provider/temp DB，不依赖 live LLM/network 或真实 user memory。
  - Verify:
    - `cargo test retrieval_router::executor::tests -- --test-threads=1`
    - `cargo test retrieval_enrichment -- --test-threads=1`
    - `cargo test retrieval::search -- --test-threads=1`
    - `cargo check`

- [ ] `SP934-T7` — 建立六 intent golden、unknown fallback 与 ablation runner — Owner: eval implementation agent; Dependencies: `SP934-T4`, `SP934-T6`; Done when: 见下; Verify: 见下
  - Owner: eval implementation agent
  - Dependencies: `SP934-T4`, `SP934-T6`
  - Done when:
    - 六个 intent 各有独立离线 slice，覆盖 relevant/irrelevant、empty/abstain、stale/
      superseded、scope mismatch、channel error/timeout、generated/corrupted 与 unknown control；
    - 报告按 intent/overall 输出 Recall@k、nDCG、abstention、stale-followed、irrelevant
      injection、token estimate、p50/p95 latency、memory-hurt；
    - `static_fusion` 与 `intent_router` 读取同一 corpus/head/dataset fingerprint；
    - repeated run byte-stable，fixture/report verifier 拒绝缺样本、重复 ID、未知 intent、
      stale fingerprint 与缺失指标。
  - Verify:
    - `cargo test eval::retrieval_router -- --test-threads=1`
    - `cargo run -- eval-gates --json-out /tmp/remem-eval-gates.json`
    - router ablation command documented in current contract and emits `/tmp` report

- [ ] `SP934-T8` — 扩展 benchmark artifact、固定阈值并执行 default gate — Owner: eval/governance implementation agent; Dependencies: `SP934-T7`; Done when: 见下; Verify: 见下
  - Owner: eval/governance implementation agent
  - Dependencies: `SP934-T7`
  - Done when:
    - memory/coding artifacts携带 mode、intent、policy version、plan hash、audit digest、
      dataset/implementation/policy fingerprints、degradation/abstention；
    - verifier 对 missing/mismatched/stale hash/fingerprint 和伪 `intent_router` artifact
      fail closed；
    - maintainer 在 report 生成前批准 baseline/thresholds；完整 ablation 证明或否定每个目标/
      非目标 slice、memory-hurt、unknown fallback、p95 latency 与 corruption leak gate；
    - default decision 由同一 head 的机器报告输出；任一失败明确 `keep_static`，禁止手工挑样本改为
      default-on。
  - Verify:
    - `cargo test eval::bench_artifact -- --test-threads=1`
    - `cargo test eval::coding_bench -- --test-threads=1`
    - `cargo run -- bench verify --root eval/public --json-out /tmp/remem-bench-verify.json`
    - `cargo run -- eval-gates --json-out /tmp/remem-eval-gates.json`

- [ ] `SP934-T9` — 同步 rollout/status、文档、contract 与版本 surfaces — Owner: integration documentation agent; Dependencies: `SP934-T8`; Done when: 见下; Verify: 见下
  - Owner: integration documentation agent
  - Dependencies: `SP934-T8`
  - Done when:
    - typed `static_fusion`/`explicit_only`/`intent_router` mode 与 invalid/stale gate fallback
      被 status JSON/tests 覆盖；
    - README、Architecture、GH934 current contract、SpecRail packet、eval/public docs、
      CHANGELOG 准确描述实际 default decision、显式 API/MCP、audit 与 rollback；
    - Planned Changes Manifest 与实际 touched paths 对齐；未触及的规划 path 被删除或在批准的
      follow-up packet 中明确移交，不留未声明 production diff；
    - Cargo/plugin/npm/server version surfaces 完全同步。
  - Verify:
    - `python3 checks/check_workflow.py --repo . --spec-dir specs/GH934`
    - `python3 scripts/ci/check_plugin_version_sync.py`
    - `python3 scripts/ci/check_version_bump.py <trusted-base-sha> HEAD`
    - `git diff --check`

- [ ] `SP934-T10` — 完成原 PR 全量验证并 push 当前 head — Owner: original PR implementation agent; Dependencies: `SP934-T1`–`SP934-T9`; Done when: 见下; Verify: 见下
  - Owner: original PR implementation agent
  - Dependencies: `SP934-T1`–`SP934-T9`
  - Done when:
    - focused tests 先通过，随后 full Rust/JS/eval/artifact/preflight 在当前 head 通过；
    - `/tmp/pr-body.md` 与原 PR #940 实际 intended body 一致，保留 `Refs #934`，说明 default
      gate 的真实 enabled/keep-static 结论；
    - 只 push 原分支，不 force push；remote head SHA 与本地验证 head 完全一致；
    - CI 针对新 head 运行，失败与 review threads 全量暴露，不使用历史 green 结果。
  - Verify:
    - `cargo fmt --check`
    - `cargo check`
    - `cargo test`
    - `cargo clippy --all-targets -- -D warnings`
    - `node --test plugins/remem/scripts/remem-runtime.test.js plugins/remem/apps/remem/server.test.js npm/remem/scripts/install.test.js`
    - `python3 scripts/ci/check_plugin_version_sync.py`
    - `cargo run -- eval-extraction --json --check-baseline`
    - `cargo run -- eval-gates --json-out /tmp/remem-eval-gates.json`
    - `python3 scripts/ci/check_pr_preflight.py --base origin/main --pr-body-file /tmp/pr-body.md`

- [ ] `SP934-T11` — independent review、serial PR gate、merge 与 closure audit — Owner: independent reviewer lane + queue coordinator + human merge authority; Dependencies: `SP934-T10`; Done when: 见下; Verify: 见下
  - Owner: independent reviewer lane + queue coordinator + human merge authority
  - Dependencies: `SP934-T10`
  - Done when:
    - independent read-only reviewer 在 exact remote head 对 issue/product/tech/tasks/diff、security/API、
      corrupted enrichment、eval/default gate 与 verification 做完整 review，blocking findings 为 0；
    - GraphQL unresolved review threads 为 0，current-head CI green，merge state clean；
    - SpecRail PR gate 串行返回 allowed；只有具备有效 merge authorization 时才 merge；
    - merge 后 remote main 包含 exact head，closure audit 逐项验证 `B-001`–`B-017`。若 PR 仍是
      Phase A/partial 或 default/eval acceptance 未完成，GH-934 保持 open 并记录 follow-up，不伪关闭。
  - Verify:
    - independent review artifact bound to exact head SHA
    - `python3 checks/runtime_ledger_gate.py --checkpoint <runtime-checkpoint> --json`
    - SpecRail `pr_gate` command with current PR/head/CI/review-thread evidence
    - fresh `gh pr view 940`、GraphQL review-thread query 与 post-merge issue/branch closure audit

## 并行拆分

默认由一个 integration owner 串行执行 `SP934-T1 → T2 → T3 → T4 → T5 → T6 → T7 → T8 → T9 →
T10 → T11`。理由是原 PR #940 与 #932/#939 共享 Context Bundle、enrichment、eval、docs 和 version
surfaces，盲目并行会违反 W-14 并制造不可审计冲突。

只有在 `SP934-T4` 的 public DTO 已冻结后，才可有限并行：

- adapter lane：只写 `src/mcp/**`、`src/api/**`，负责 `SP934-T5`；
- eval fixture lane：只写 `eval/retrieval-router/**`，不得修改 `src/eval/**`，准备 `SP934-T7`
  fixture 草案；
- coordinator 保留 `src/retrieval_router/**`、`src/context_bundle/**`、`src/eval/**`、docs/version
  surfaces。

每个 lane 必须使用独立 worktree 和显式、不重叠 file ownership。`src/eval/**`、current contract、
packet、CHANGELOG 与版本 surfaces 始终由 coordinator 串行集成。

## 验证

- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH934`
- `git diff --check`
- focused router/context/enrichment/MCP/API/eval/artifact tests
- `cargo fmt --check`
- `cargo check`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- required JS tests、version sync、eval gates、bench verifier
- full `check_pr_preflight.py` on exact original PR head
- current-head CI/review threads/merge state、independent review、serial PR gate 与 closure audit

## Handoff Notes

- 本 plan 不授权 implementation；`SP934-T1` 是不可绕过的人工/duplicate/sensitive gate。
- 当前本地 implement gate rejection 包含：packet artifacts absent、duplicate evidence missing、
  trusted default base ref/path evidence missing、`sensitive_enforcement` missing。packet 写入后仍需用
  fresh trusted evidence重跑，不能把 write_spec allowed 当 implement allowed。
- PR #940 是唯一原 PR；它当前 stacked on #932 且 dirty。任何修复必须在原分支完成，禁止新建替代
  PR 或 force push。
- GH-933 enrichment 来自原 PR #939；`SP934-T6` 开始前必须以合并后 main 的实现 truth 更新 exact
  anchors，禁止根据本 spec 发明第二套 projection。
- Phase A 或保持 static 的 gate 结论都不自动关闭 issue。只有全部 invariant 有当前证据，closure
  audit 才能决定 GH-934 是否完成。
