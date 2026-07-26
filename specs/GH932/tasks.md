# Task Plan

Status: Planning only；当前 implementation gate blocked
Date: 2026-07-26

## Linked Issue

GH-932

- Merged partial implementation: PR #938 / merge
  `284cdf94406dbbe2583e6ee31f23e2a48af561bf`
- Current issue state: open；GitHub labels 尚不包含 SpecRail readiness label
- Selected locale: `zh-CN`

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`

本文件只固定人工门、依赖、文件 ownership、done-when 与 fresh verification。所有任务均未完成；
PR #938 只作为 Phase A baseline，不把本计划中的任一 checkbox 自动标为完成。

2026-07-26 在当前 packet 上执行：

```bash
python3 checks/route_gate.py --repo . --route implement \
  --issue 932 --state ready_to_implement --json
```

结果为 `blocked`：缺少 trusted default-base/path evidence、
`duplicate_evidence` 与 `sensitive_enforcement`，并仍要求 `readiness_label` 与
`spec_approval`。该命令只是本地 planning 诊断；调用方自报 `--state` 不能替代 live human
readiness/spec approval，也不授权 production edit。

## 实现任务

- [ ] `SP932-T0` — 人工 readiness、spec approval 与 implementation gate — Owner: maintainer / SpecRail coordinator；Dependencies: none；Covers: `B-001`–`B-027`；Done when: 见下；Verify: 见下
  - Writable ownership: 仅 gate/evidence artifacts；不得修改 production、tests、version 或
    release files。
  - Done when:
    1. maintainer 审查并批准 `product.md` / `tech.md` exact diff；
    2. GH-932 获得 trusted `ready_to_implement` readiness state；
    3. live issue/duplicate/PR/head/default-base/sensitive evidence 完整、fresh、同仓库，原 PR/
       remote branch ownership 冲突已有人工决定；
    4. repo-local prospective sensitive implementation wrapper 在 implementation 开始前返回
       schema-valid `allowed`，并绑定 approved spec revision、current implementation PR、
       exact head 与完整 planned paths。
  - Verify:
    - `python3 checks/check_workflow.py --repo . --spec-dir=specs/GH932`
    - `python3 checks/github_issue_evidence.py --repo . --github-repo majiayu000/remem --issue 932 --json > /tmp/gh932-issue-evidence.json`
    - `python3 checks/github_duplicate_evidence.py --github-repo majiayu000/remem --issue 932 --remote origin --json > /tmp/gh932-duplicate-evidence.json`
    - `python3 scripts/ci/run_sensitive_implement_gate.py --repo . --github-repo majiayu000/remem --issue 932 --pr <IMPLEMENTATION_PR> --head-sha <EXACT_HEAD_SHA> --output /tmp/gh932-sensitive-implement-gate.json`
    - 检查 output 的 repository/PR/head/base/readiness/spec/duplicate/path evidence 均绑定且
      `decision=allowed`、`missing=[]`。
  - Blocking rule: T0 未完成时，`SP932-T1` 至 `SP932-T11` 全部 blocked；只能继续
    spec/task review，不能预写 production 或以 feature flag 隐藏实现。

- [ ] `SP932-T1` — schema v2、closed DTO 与 canonical hashes — Owner: context-domain agent；Dependencies: `SP932-T0`；Covers: `B-001`–`B-004`, `B-005`, `B-007`；Done when: 见下；Verify: 见下
  - Writable ownership: `src/context_bundle.rs`,
    `src/context_bundle/{domain,planner,policy,hash,audit}.rs`,
    `src/context_bundle/tests/{mod,planner,schema}.rs`。
  - Done when: schema/policy upgrade、完整 semantic sections、role/risk/worktree/as-of filters、
    exact plan-hash revalidation、canonical audit hash 与 stable ordering 全部落地；schema v1/
    unknown enum 明确拒绝。
  - Verify:
    - `cargo test context_bundle::tests::planner -- --nocapture`
    - `cargo test context_bundle::tests::schema -- --nocapture`
    - tampered/empty/mismatched hash、reverse insertion、clock/env independence fixtures 全绿。

- [ ] `SP932-T2` — strict DB snapshot executor 与 scope/temporal eligibility — Owner: DB/context adapter agent；Dependencies: `SP932-T1` frozen domain interface；Covers: `B-003`, `B-004`, `B-006`, `B-009`–`B-014`, `B-017`–`B-020`；Done when: 见下；Verify: 见下
  - Writable ownership: `src/context_bundle/{executor,db_executor}.rs`,
    `src/context_bundle/tests/{executor,db_executor}.rs`,
    `src/context/{render_inputs,query,types}.rs`；T1 停止写入并记录 frozen handoff 后，T2
    还独占接收 `src/context_bundle.rs` 与 `src/context_bundle/tests/mod.rs`，仅用于注册
    executor/db_executor 模块和 focused tests。T2 完成后再次冻结并移交，禁止与 T1/T3/T4
    共享写入这两个 registry 文件。
  - Done when: production path 只从同一 read transaction 读取真实 remem candidates；所有
    loader error 原子失败；project/worktree/branch/role/risk/as-of/supersession 在 query 和
    executor 双层执行；caller-provided seam 仅测试可见；无 runtime DB write。
  - Verify:
    - `cargo test context_bundle::tests::executor -- --nocapture`
    - `cargo test context_bundle::tests::db_executor -- --nocapture`
    - real migration fixtures；same-snapshot concurrent writer；future/expired/invalidated/
      branch/worktree mismatch；loader error/cancel no-partial 矩阵全绿。

- [ ] `SP932-T3` — semantic classification、conflict/abstention、provenance/freshness — Owner: bundle policy agent；Dependencies: `SP932-T2`；Covers: `B-005`–`B-008`, `B-012`–`B-014`, `B-017`, `B-018`；Done when: 见下；Verify: 见下
  - Writable ownership: T2 完成后接收
    `src/context_bundle/{domain,executor,db_executor,audit}.rs` 与其 focused tests；移交前不得
    与 T1/T2 并行写 shared files。
  - Done when: memory/current-state/lesson/session/workstream 进入正确 semantic section；
    generated/graph-derived attribution 不冒充 canonical；每个 candidate exactly-once
    selected/dropped/abstained/conflict；freshness rollup 可重算。
  - Verify:
    - `cargo test context_bundle -- --nocapture`
    - unique-winner/two-active conflict、missing backing ref、high-risk、unknown temporal
      provenance、audit terminal-state property tests 全绿。

- [ ] `SP932-T4` — segmented renderer 与 strict rendered budget — Owner: rendering agent；Dependencies: `SP932-T1` frozen DTO（原型）, `SP932-T3` frozen shared registries（最终 wiring）；Covers: `B-003`, `B-015`, `B-016`, `B-018`, `B-027`；Done when: 见下；Verify: 见下
  - Writable ownership: `src/context_bundle/render.rs`,
    `src/context_bundle/tests/render.rs`；不修改 legacy SessionStart renderer。原型阶段不得
    修改 shared registry；T3 停止写入并记录 frozen handoff 后，T4 独占接收
    `src/context_bundle.rs` 与 `src/context_bundle/tests/mod.rs`，仅完成 render module/test
    wiring，随后冻结移交给 T5/T6。
  - Done when: versioned UTF-8 upper-bound estimator 计算 header/title/body/ref/separator 的完整
    rendered segments；section/total budget 永不超限；tiny budget、multibyte、item-boundary、
    rendered hash 与 audit/body parity 全部固定。
  - Verify:
    - `cargo test context_bundle::tests::render -- --nocapture`
    - property cases 断言任意生成 item 下 `rendered_token_estimate <= token_budget`，且 section
      counts、drop reasons、UTF-8 都合法。

- [ ] `SP932-T5` — SessionStart Bundle bridge、single-path gate 与 rollback — Owner: context integration agent；Dependencies: `SP932-T2`, `SP932-T3`, `SP932-T4`；Covers: `B-015`–`B-022`, `B-026`, `B-027`；Done when: 见下；Verify: 见下
  - Writable ownership: `src/context.rs`, `src/context/bundle_bridge.rs`,
    `src/context/{render,audit}.rs`, `src/context/tests/{mod,bundle_bridge,gate_pipeline,render,truncation}.rs`,
    `src/runtime_config.rs`。
  - Done when: selector 默认 legacy、显式 bundle_v2；单次只 load/render/inject 一条路径；
    semantic parity、strict gate、audit persistence、error visibility 与 rollback 都通过；
    `src/context/render.rs` 不因集成超过 800 行。
  - Verify:
    - `cargo test context::tests::bundle_bridge -- --nocapture`
    - `cargo test context::tests::gate_pipeline -- --nocapture`
    - `cargo test context::tests::render -- --nocapture`
    - legacy/bundle same-snapshot golden、double-injection sentinel、invalid config、rollback、
      empty/error/tiny-budget fixtures 全绿。

- [ ] `SP932-T6` — MCP/REST experimental surfaces 与 parity/auth — Owner: transport agent；Dependencies: `SP932-T2`, `SP932-T4` frozen API；Covers: `B-001`, `B-004`, `B-009`–`B-020`, `B-023`, `B-026`, `B-027`；Done when: 见下；Verify: 见下
  - Writable ownership: `src/mcp/server.rs`,
    `src/mcp/server/context_bundle_tools.rs`,
    `src/mcp/server/tests.rs`,
    `src/mcp/server/tests/{context_bundle,tool_metadata}.rs`, `src/mcp/types.rs`,
    `src/api/{server,handlers,types}.rs`, `src/api/handlers/{context_bundle,capabilities}.rs`,
    `src/api/tests.rs`, `src/api/tests/context_bundle.rs`, `tests/api_public.rs`。这些 transport
    registry 与 focused test files 由 T6 单一 owner 串行写入，不与其他 lane 共享。
  - Done when: MCP `context_plan`/`context_bundle` 与 REST POST plan/bundle 使用同一 DTO/
    executor；capability 标 experimental/schema/policy；stable errors 一致；REST unauthorized
    在 DB read 前拒绝。
  - Verify:
    - `cargo test mcp::server -- --nocapture`
    - `cargo test api::tests::context_bundle -- --nocapture`
    - `cargo test --test api_public -- --nocapture`
    - cross-transport golden equality、tool schema、HTTP/MCP error mapping、auth read-sentinel、
      deny-network fixtures 全绿。

- [ ] `SP932-T7` — doctor capability/degraded report without payload — Owner: diagnostics agent；Dependencies: `SP932-T5`, `SP932-T6`；Covers: `B-017`, `B-022`, `B-024`, `B-026`, `B-027`；Done when: 见下；Verify: 见下
  - Writable ownership: `src/doctor.rs`, `src/doctor/context_bundle.rs`,
    `src/doctor/{report,types}.rs`, `src/doctor/tests/context_bundle.rs`；shared `tests.rs` 只做最小
    module wiring。
  - Done when: human/JSON 报告 schema/policy/estimator/DB readiness/configured+effective
    SessionStart path/MCP+REST capability/degraded reason；doctor JSON schema 提升且不输出 memory
    text、query、path、evidence 或 secret。
  - Verify:
    - `cargo test doctor::tests::context_bundle -- --nocapture`
    - no-DB/current-DB/newer-schema/legacy/bundle/degraded/blocked 矩阵与 payload-leak sentinel 全绿。

- [ ] `SP932-T8` — coding-bench same-run plan/audit evidence — Owner: eval agent；Dependencies: `SP932-T5`；Covers: `B-002`–`B-004`, `B-015`–`B-019`, `B-025`, `B-026`；Done when: 见下；Verify: 见下
  - Writable ownership: `src/eval/coding_bench/{artifact,condition,runner,types,tests}.rs`,
    `eval/coding-bench/README.md`。
  - Done when: 每个 remem-backed run 从实际 production bridge 保存 schema/policy/estimator/
    plan/audit/render hashes、degraded mode、预算、head、fixture；validator fail closed，control
    arm 明确 not-applicable。
  - Verify:
    - `cargo test eval::coding_bench -- --nocapture`
    - tampered plan/audit/render hash、budget overflow、wrong head/fixture、missing evidence、
      synthetic-other-run evidence 负例全绿。
    - `cargo run -- eval-coding-bench --fixture eval/coding-bench/fixtures/tasks.json --runs-per-condition 1 --dry-run --json-out /tmp/gh932-coding-bench.json`

- [ ] `SP932-T9` — current contracts、user docs、changelog 与 version sync — Owner: docs/version agent；Dependencies: `SP932-T5`, `SP932-T6`, `SP932-T7`, `SP932-T8`；Covers: `B-001`, `B-021`–`B-027`；Done when: 见下；Verify: 见下
  - Writable ownership: `docs/specs/GH932/{PRODUCT,TECH}.md`, `docs/specs/README.md`,
    `README.md`, `docs/ARCHITECTURE.md`, `CHANGELOG.md`, `Cargo.toml`, `Cargo.lock`,
    `plugins/remem/.codex-plugin/plugin.json`, `plugins/remem/runtimes/remem-releases.json`,
    `npm/remem/package.json`, `server.json`。
  - Done when: 文档明确 Phase A history 与 current complete behavior、experimental surfaces、
    default gate/rollback、budget estimator/degraded semantics；一次 unreleased version staging
    全部同步，不发布 release。
  - Verify:
    - `python3 scripts/ci/check_plugin_version_sync.py`
    - `python3 scripts/ci/check_version_bump.py <LIVE_ORIGIN_MAIN_SHA> HEAD`
    - `git diff --check`

- [ ] `SP932-T10` — full verification 与 independent review handoff — Owner: verification agent；Dependencies: `SP932-T1`–`SP932-T9`；Covers: `B-001`–`B-027`；Done when: 见下；Verify: 见下
  - Writable ownership: verification/review artifacts only；不得修改 production 或放宽 tests。
  - Done when: product-to-test matrix 每项有 current-head fresh output；完整 planned paths 与实际
    diff 集合核对；independent reviewer 在 exact head 审查，无 unresolved actionable finding；
    full preflight/CI green。
  - Verify:
    - `cargo fmt --check`
    - `cargo check`
    - `cargo test`
    - `cargo clippy --all-targets -- -D warnings`
    - `python3 checks/check_workflow.py --repo . --spec-dir=specs/GH932`
    - `python3 scripts/ci/check_pr_preflight.py --base origin/main --pr-body-file /tmp/pr-body.md`

- [ ] `SP932-T11` — PR gate、merge 与 issue closure audit — Owner: SpecRail coordinator + human final reviewer；Dependencies: `SP932-T10`；Covers: `B-001`–`B-027`；Done when: 见下；Verify: 见下
  - Writable ownership: remote PR/closure evidence only；禁止 force push、release 或权限变更。
  - Done when: current PR head、CI、independent review artifact、GitHub review threads、
    merge state 与 PR gate 全部绑定且 green；human final review/merge authorization 有效；
    merge 后 closure audit 证明所有 acceptance criteria 已实现，才可关闭 GH-932。
  - Verify:
    - live GitHub PR/head/check/review-thread evidence（GraphQL review threads）
    - `python3 checks/pr_gate.py ... --json` 使用 current-head trusted evidence返回允许结论
    - merge commit 在 `origin/main`，closure audit 无 missing follow-up。

## 并行拆分

- T1 完成并冻结 domain/hash interface 后，T2（DB adapter）与 T4 的 estimator/renderer
  原型可在 disjoint files 并行；T4 只能使用 frozen DTO，不能反向改 T1 文件。shared
  `src/context_bundle.rs` / `src/context_bundle/tests/mod.rs` 先由 T2 在 T1 handoff 后独占
  完成 executor wiring，再移交 T3；T4 原型阶段不写这两个文件。
- T3 需要接收 T1/T2 shared-file ownership，因此默认串行；T3 冻结后再把两个 shared
  registry 独占移交 T4 完成 render wiring，禁止 T2/T3/T4 并发写。
- T6 transport、T7 doctor、T8 eval 可在 T5 bridge frozen 后并行，文件 ownership 如各任务所列，
  不得同时写 `src/context_bundle.rs`、`src/context.rs` 或 shared test modules；T6 独占
  `src/mcp/server/tests.rs` 与 `src/api/tests.rs` 作为 transport test registries。
- T9 docs/version 在功能行为稳定后串行；T10/T11 必须最后执行。
- 任一 shared file 需要跨任务修改时，先停止原 owner、记录 handoff，再由单一 lane 写；禁止两个
  agent 共享 writable file。

## Draft Packet 验证

以下只验证 planning packet，不授权实现：

```bash
git diff --check
PYTHONPATH=checks python3 -c 'from pathlib import Path; from sensitive_enforcement import parse_planned_changes_manifest; m=parse_planned_changes_manifest(Path("specs/GH932/tech.md").read_bytes()); assert m["version"] == 1 and m["issue"] == 932 and m["complete"] is True'
python3 checks/check_workflow.py --repo .
python3 checks/check_workflow.py --repo . --spec-dir=specs/GH932
python3 checks/route_gate.py --repo . --route write_spec --issue 932 --state ready_to_spec --json
```

## Handoff Notes

- 当前最高优先级是人工审查 product/tech 与解决 duplicate branch/PR ownership，再收集 trusted
  live evidence 完成 `SP932-T0`；在此之前不得实现。
- PR #938 是已合并 partial implementation，不回退、不重复创建同名基础设施，也不能被用作
  complete issue 的 closing evidence。
- 本 packet 使用 `Refs #932`。只有 T11 closure audit 证明完整 issue acceptance 后，
  implementation PR 才能使用 closing semantics。
- 若实现发现 planned manifest 外路径是正确性所必需，必须先更新 tech manifest、重新运行 packet
  checks 并重新获得 exact-diff spec approval；禁止 implement gate 后临时扩 scope。
