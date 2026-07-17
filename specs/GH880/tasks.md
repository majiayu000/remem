# Task Plan：安全本地控制台证据、只读资源与可恢复治理

## Linked Issue

GH-880

## Spec Packet

- Product: [`product.md`](product.md)
- Tech: [`tech.md`](tech.md)

## 实现任务

- [ ] `SP880-T1` Owner: backend worker; Dependencies: complete spec packet and allowed implement route; Done when: schema versions, mutation ledger, stable operation IDs, and typed cursor primitives pass migration and focused tests; Verify: 见 SP880-T1。
- [ ] `SP880-T2` Owner: backend worker; Dependencies: SP880-T1; Done when: candidate detail/evidence and safe review satisfy B-001–B-004 without weakening CLI compatibility; Verify: 见 SP880-T2。
- [ ] `SP880-T3` Owner: backend worker; Dependencies: SP880-T1 and serial ownership of shared API files; Done when: five independent read resources satisfy B-005–B-010 and redaction leak tests; Verify: 见 SP880-T3。
- [ ] `SP880-T4` Owner: backend worker; Dependencies: SP880-T1 and serial ownership of shared API files; Done when: archive/restore satisfy B-011–B-015 and delete remains unavailable; Verify: 见 SP880-T4。
- [ ] `SP880-T5` Owner: coordinator; Dependencies: SP880-T2, SP880-T3, SP880-T4; Done when: contract, smoke, release surfaces and full verification satisfy B-016–B-017; Verify: 见 SP880-T5。
- [ ] `SP880-T6` Owner: frontend worker; Dependencies: published remem capability release; Done when: remem-web consumes the safe capabilities and closes GH-3 only after its acceptance criteria pass; Verify: 见 SP880-T6。

### SP880-T1 — Schema、mutation ledger 与共享安全 primitive

- Owner：backend worker（单 lane）
- Dependencies：GH-880 spec packet 完整；实现 route gate 与 duplicate-work evidence 通过；基于最新 `origin/main` 选择未占用的 migration 版本。
- Files owned：`src/migrations/v*_web_console_governance.sql`、`src/migrate/types.rs`、`src/migrate/schema_drift/invariants.rs`、migration tests、`src/api/mutation.rs`、`src/api/cursor.rs`、共享模块声明。
- Work：添加 candidate/memory integer version、幂等账本、稳定 operation id/request hash helper、typed cursor codec；不注册业务 endpoint，不提前声明 capability。
- Done when：
  - fresh/upgrade DB 具有正确 default、unique/index/schema invariant；
  - same key + same payload 可读取同一 ledger 结果，不同 payload 明确 conflict；
  - cursor 对 kind/filter/version 绑定且 malformed input fail closed；
  - 没有 token、raw payload 或 secret 写入 request hash/audit fixture。
- Verify：
  - `cargo test migrate --locked`
  - `cargo test api::mutation --locked`
  - `cargo test api::cursor --locked`
  - `cargo fmt --check`
  - `cargo check --locked`

### SP880-T2 — Candidate detail/evidence 与安全审核

- Owner：backend worker（单 lane）
- Dependencies：SP880-T1。
- Files owned：`src/api/handlers/candidate_detail.rs`、`src/api/handlers/candidate_review.rs`、candidate API types/tests、`src/memory_candidate/review.rs`、`src/memory_candidate/review/approval.rs`、必要的 redaction 共享导出。
- Work：实现 detail 安全投影和 fail-closed decision；把 domain mutation 拆成 transaction-scoped primitive；为 Web approve/reject/edit 增加 reason/version/idempotency/audit envelope；保留旧 CLI wrapper 和兼容 endpoint 行为。
- Done when：
  - detail 返回 version、evidence provenance、`can_review` 和稳定 blocked codes；
  - missing/cross-project/suppressed/unsafe evidence 均阻止审核但不泄漏原文；
  - approve/reject/edit 的并发、重放、payload conflict 和失败回滚均有 E2E 证据；
  - `candidate_detail`、`candidate_evidence`、`candidate_review_safe` 只在 endpoint + tests 完整后为 true。
- Verify：
  - `cargo test api::tests::candidates --locked`
  - `cargo test api::tests::candidate_review_poisoning --locked`
  - `cargo test memory_candidate::review --locked`
  - `cargo fmt --check`
  - `cargo check --locked`

### SP880-T3 — 五类安全只读资源

- Owner：backend worker（单 lane）
- Dependencies：SP880-T1；可在 T2 合并后串行执行以避免共享 `api/types/server/capabilities` 冲突。
- Files owned：五类 resource handler/query/DTO、`src/api/handlers.rs`、`src/api/server.rs`、`src/api/handlers/capabilities.rs`、`src/api/types.rs`、read-resource tests。
- Work：实现 observations/sessions/workstreams/events/tasks 的独立 list/detail、keyset cursor、安全引用和服务端脱敏；禁止 raw blob、event detail、task payload/last_error 原文。
- Done when：
  - 每类 capability 与 endpoint 独立开关且 route 全受 bearer middleware 覆盖；
  - empty/data/not-found/auth/DB failure 不互相伪装；
  - cursor 重读、边界、并发插入、非法/跨 endpoint/filter mismatch 有回归；
  - secret/token/env/transcript/payload fixtures 不出现在任何 JSON 字段。
- Verify：
  - `cargo test api::tests::read_resources --locked`
  - `cargo test adapter::common::tests --locked`
  - `cargo fmt --check`
  - `cargo check --locked`

### SP880-T4 — Archive / restore 可恢复治理

- Owner：backend worker（单 lane）
- Dependencies：SP880-T1；在 T3 后串行处理共享 router/capability 文件。
- Files owned：`src/api/handlers/memory_governance.rs`、`src/memory/governance.rs`、memory governance API/types/tests、router/capability 增量。
- Work：新增 archive/restore transaction primitive 与 Web endpoint；使用 version/idempotency/reason；同事务更新、audit、ledger；显式 `memory_delete=false`，不注册 delete route。
- Done when：
  - archive 保留内容且默认 read/search 不可见；restore 仅从 archived 恢复；
  - replay、different-payload、version race、not recoverable、DB failure rollback 有测试；
  - response 含完整 audit envelope；失败含稳定 operation id 且不泄漏 SQL/内容；
  - capability map 只声明 archive/restore，delete 为 false 且无 endpoint。
- Verify：
  - `cargo test api::tests::memory_governance --locked`
  - `cargo test memory::governance --locked`
  - `cargo test retrieval --locked`
  - `cargo fmt --check`
  - `cargo check --locked`

### SP880-T5 — Contract、smoke、release 与全量验证

- Owner：coordinator。
- Dependencies：SP880-T2、T3、T4 全部完成。
- Files owned：`docs/specs/SPEC-web-api.md`、README、CHANGELOG、`scripts/smoke_native_web_api.sh`、version/release manifests；只做与 GH-880 对应的发布面同步。
- Work：记录 endpoint/request/response/error/cursor/redaction/min-version 契约；扩展 native smoke；执行版本同步和 release gate；不宣称未发布 binary 已可用。
- Done when：
  - contract 和 capabilities 精确一致；
  - smoke 覆盖 candidate detail/safe review、五类 read、archive/restore、delete absence；
  - source/package/plugin/server manifests 同步且 version bump gate 通过；
  - product B-001–B-017 均有测试或可审计命令对应。
- Verify：
  - `bash scripts/smoke_native_web_api.sh`
  - `cargo fmt --check`
  - `cargo check --locked`
  - `cargo test --locked --quiet`
  - `cargo clippy --all-targets -- -D warnings`
  - `python3 checks/check_workflow.py --repo .`
  - `python3 checks/check_workflow.py --repo . --spec-dir specs/GH880`
  - `python3 scripts/ci/check_plugin_version_sync.py`
  - `python3 scripts/ci/check_version_bump.py <base-sha> HEAD`
  - `python3 scripts/ci/check_pr_preflight.py --base <base-sha> --head HEAD --pr-body-file <body-file>`
  - `git diff --check`

### SP880-T6 — remem-web 消费与 GH-3 closure

- Owner：frontend worker in `majiayu000/remem-web`。
- Dependencies：承载 GH-880 capabilities 的 remem release 已发布；remem-web GH-3 spec/contract 复核完成。
- Files owned：仅 remem-web repo 的 API types/client、capability gates、resource/candidate/governance pages 和 tests；不与 remem backend lane 共享文件。
- Work：按独立 capability 消费新 endpoint，完整展示 loading/empty/error/conflict/replay；永久 delete 不进入 UI；installed binary 继续受最低发布版本 gate。
- Done when：
  - candidate evidence 和安全审核 UI 只在三个 candidate capability 全 true 时启用；
  - 五类资源逐项 gate，archive/restore 有 reason/version/idempotency 与冲突反馈；
  - TypeScript typecheck/test/build/audit 通过；
  - GH-3 所有验收标准完成后使用 closing PR 并做 closure audit。
- Verify：
  - `npx tsc --noEmit`
  - project test command
  - project build command
  - project security audit command

## 并行拆分

本任务的核心 API 文件（`server.rs`、`types.rs`、`handlers.rs`、capabilities）会被 T2–T4 共同修改，因此默认串行，避免 W-14 共享写入。只有以下 lane 可并行：

- reviewer lane：全程只读 spec/diff，不修改文件；
- T6 frontend lane：仅在后端 release contract 稳定后写 `remem-web` repo；
- CI/closure lane：只读远端状态和证据，不处理 implementation review thread。

若后续确需并行 T2–T4，coordinator 必须先把共享 router/type/capability 适配集中到一个 owner，其余 worker 仅拥有互不重叠的新模块和测试文件。

## 验证

- route gate 使用 fresh duplicate-work evidence 且 decision allowed。
- 每个 PR 都运行 focused tests、`cargo fmt --check`、`cargo check --locked` 和 `git diff --check`。
- 最终后端 PR 运行 full test、clippy、native smoke、workflow/spec、version sync/bump/preflight。
- `specrail-check-impl-against-spec` 对 B-001–B-017 输出无缺口。
- 每个 merge-ready PR 有独立 native reviewer lane、当前 head SHA、绿色 CI、零 unresolved threads、clean merge state 和 serial PR gate。
- remem release 后再执行 remem-web T6 和 installed-binary smoke。
- 最终 closure audit 确认 remem#880、remem-web#3 无遗留 actionable acceptance criterion。

## Handoff Notes

- `implx auto` 是本轮 readiness-label 和满足全部证据后的 merge standing authorization，不是跳过独立审查或 CI 的授权。
- migration 文件名中的 v70 只是当前 main 的预期；实现前刷新 remote main，若版本已占用必须顺延并同步 planned changes。
- 现有用户工作树 `/Users/lifcc/Desktop/code/AI/tool/remem` 很脏，禁止复用或清理；所有后端实现使用新的 clean worktree。
- 旧 `candidate_review` capability/endpoint 保持兼容。新 remem-web 写 UI 必须看 `candidate_review_safe`，不能把旧 flag 解释成安全审核契约。
- 任何仅含敏感内容的 evidence/resource 都宁可显示 redacted/blocked，不得 fallback raw。
- 永久 delete、raw transcript/blob 和跨 repo 非同 owner 工作均不在授权范围。
