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
- Files owned：`src/migrations/v*_web_console_governance.sql`、`src/migrate/types.rs`、`src/migrate/run.rs`、`src/migrate/schema_drift/invariants.rs`、migration tests、`src/api/mutation.rs`、`src/api/cursor.rs`、共享模块声明。
- Work：添加 candidate/memory integer version、`memories.web_archive_operation_id`、覆盖所有 Web 可见字段 writer 的 version triggers、任一 status transition 清除 archive marker 的 trigger；将五个 Web cursor source 主键重建为保留现有 id 的 AUTOINCREMENT；添加只存 `idempotency_key_hash` 且带 response schema version 的幂等账本、稳定 operation id/request hash helper 和 typed cursor codec；不注册业务 endpoint，不提前声明 capability。
- Done when：
  - fresh/upgrade DB 具有正确 default、unique/index/trigger/schema invariant，CLI/worker/lifecycle 可见字段更新也推进 version，任一非 Web status transition 都清除旧 Web archive marker；
  - run.rs 在 BEGIN 前保存/关闭/验证 FK；`observations`、`sessions`、`workstreams`、`captured_events`、`extraction_tasks` rebuild 保留既有 id/FK/index/trigger/FTS，commit 前 check，commit/rollback 后恢复并验证 FK ON；成功与注入 migration/check/rollback 失败均无部分 schema 或 FK-off 连接；
  - 每表 post-migration 删除当前 max 后插入的新 id 大于迁移时现存 MAX，production writer 不显式分配 id；
  - idempotency key trim 后必须为 `1..=128` ASCII bytes 且匹配 `[A-Za-z0-9._~-]+`；非法输入在 hash/事务/存储/日志前返回不含 `operation_id` 的 `idempotency_key_invalid`，只可携带独立 request/trace id；same key + same payload 可读取同一 ledger 结果，不同 payload 明确 conflict；
  - raw/normalized idempotency key 不进入 DB、audit、日志或响应，仅 SHA-256 摘要进入 ledger，sentinel 测试同时证明明文不存在且摘要存在；
  - cursor 对 kind/filter/version/`resume_before_id` 绑定且 malformed input fail closed；full page 以最后 returned safe id 续页，budget partial/empty page 以最后 scanned raw id 续页，任何未返回 safe row 不被越过；
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
  - missing/cross-project/suppressed/unsafe evidence 均阻止审核；raw-only/`session_stop` evidence 返回 `evidence_safe_projection_unavailable`，candidate query/DTO 不读取 `captured_events.content_text`，non-secret transcript sentinel 和 secret 均不泄漏；
  - 当前请求 ledger lookup 先于 candidate version/status/evidence；首次成功改变状态后，用原 stale request 重放仍返回同 operation/audit/final version 且无第二 mutation/audit，different payload 即使 candidate 已不可审核也优先 conflict；并发和失败回滚均有 E2E 证据；
  - 新 safe review route 不改变旧 route/payload；空白、超长、Unicode、控制字符及字符集外 idempotency key fail closed，UUID/ULID key 可安全重放且明文不进入 DB/audit/log/response；
  - `candidate_detail`、`candidate_evidence`、`candidate_review_safe` 在本 slice 保持 false，等待 T5 smoke/contract/release gate 后统一启用；contract test 预先固定 `candidate_detail` 和 `candidate_evidence` 两个 endpoint key 最终都映射 `/api/v1/candidates/{id}`。
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
- Work：实现 observations/sessions/workstreams/events/tasks 的独立 list/detail、AUTOINCREMENT keyset cursor、安全引用、fail-closed resource projection policy 和服务端脱敏；禁止 raw blob、event detail、task payload/last_error 原文。
- Done when：
  - 每类 route 全受 bearer middleware 覆盖，但 capability 和 endpoint map 在本 slice 保持未发布，等待 T5；
  - empty/data/not-found/auth/DB failure 不互相伪装；active pattern suppression 覆盖全部可见文本，memory/topic/entity relation suppression 移除对应 row/ref；list 省略、detail 404、policy failure 5xx、revocation 恢复，且没有 `include_suppressed` 绕过；
  - cursor 重读、边界、并发插入、每表 page1/page2 间 purge max/reinsert、safe/suppressed interleave、all-suppressed 多预算批次、full/partial/empty/terminal page、session last-seen 更新、null observation epoch、非法/跨 endpoint/filter mismatch 有回归；
  - 五类 list 的 `page_size` omitted=50，0/negative clamp 1，101/large integer clamp 100，malformed/overflow 返回 `page_size_invalid`，响应最多 100 行且 `next_cursor` 正确；
  - events/observations 只从 allowlisted metadata 或批准派生数据构造安全投影，query/DTO 不读取 raw `content_text`；secret/token/env/transcript/payload 与 non-secret raw sentinel 不出现在 GH-880 新 endpoint 的任何 JSON 字段或关系展开；legacy `/api/v1/search.raw_hits` 由兼容回归测试保持现有行为。
- Verify：
  - `cargo test api::tests::read_resources --locked`
  - `cargo test adapter::common::tests --locked`
  - `cargo fmt --check`
  - `cargo check --locked`

### SP880-T4 — Archive / restore 可恢复治理

- Owner：backend worker（单 lane）
- Dependencies：SP880-T1；在 T3 后串行处理共享 router/capability 文件。
- Files owned：`src/api/handlers/memory_governance.rs`、`src/memory/governance.rs`、memory governance API/types/tests、router/capability 增量。
- Work：新增 archive/restore transaction primitive 与 Web endpoint；让 memory list/detail additive 返回治理所需 version；使用 version/idempotency/reason；同事务更新、audit、ledger；显式 `memory_delete=false`，不注册 delete route。
- Done when：
  - archive 仅允许 active，保留内容且 remem-web active list/search 不可见；Web archive 在 status trigger 清除旧 marker 后写入当前 `operation_id`，restore 只接受 marker 与成功 ledger/audit 精确匹配的当前 archived 行并在状态转换时清除 marker；canonical 无 status list 保持兼容；
  - active/archived list/detail 返回当前整数 version；客户端读取值可作为 expected_version，archive 成功返回的最终 version 可直接驱动 restore；
  - 当前 archive/restore 请求 ledger lookup 先于 memory version/status/marker/provenance；archive 后 archived、restore 后 active/marker cleared 的 same-key replay 均返回首次 envelope且无第二 audit，different payload 优先 conflict；
  - 新 key/ledger miss 才执行 version/status/marker 及 restore provenance lookup；非 Web archive、Web archive -> restore -> scope cleanup 均不可恢复，后续 fresh Web archive 写入新 marker 后可恢复；非法 key 在 operation 建立前返回不含 `operation_id` 的 400，DB failure rollback 有测试；
  - response 含最终 version 和完整 audit envelope；key 校验通过后的失败含稳定 operation id 且不泄漏 SQL/内容；archive/restore E2E 证明 raw/normalized idempotency key sentinel 不进入 DB/audit/log/response，且 ledger 中对应 SHA-256 hash 存在；
  - 本 slice 的 archive/restore flag 仍为 false，delete 为 false 且无 endpoint；T5 通过后才发布 archive/restore map。
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
  - contract、smoke 与 routes 精确一致，并在本 slice 首次把已完成能力 flag 置 true、加入 endpoint map；`candidate_detail` 和 `candidate_evidence` 两个 key 均显式映射 `/api/v1/candidates/{id}`；
  - smoke 覆盖 candidate detail/safe review、五类 read/suppression/cursor、archive/restore、delete absence；legacy search raw-hit contract regression 证明现有兼容面未被本 spec 静默改变；
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
  - candidate evidence 和安全审核 UI 只在三个 candidate capability 全 true 且 detail/evidence endpoint key 均声明时启用；
  - 五类资源逐项 gate，archive/restore 从 list/detail 获取 version，并有 reason/version/idempotency 与冲突反馈；
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
- 每个 merge-ready PR 有当前对话中的明确 human merge authorization evidence、独立 native reviewer lane、当前 head SHA、绿色 CI、零 unresolved threads、clean merge state 和 serial PR gate。
- remem release 后再执行 remem-web T6 和 installed-binary smoke。
- 最终 closure audit 确认 remem#880、remem-web#3 无遗留 actionable acceptance criterion。

## Handoff Notes

- 本 task packet 只定义 implementation/review orchestration 的工作范围，不能自行授权 readiness-label progression 或 merge；两者都必须满足仓库 human gate，每次 merge 还必须引用当前对话中的明确用户授权 evidence，并继续满足独立审查、当前 HEAD CI、零 unresolved threads、clean merge state 和串行 PR gate。
- migration 文件名中的 v70 只是当前 main 的预期；实现前刷新 remote main，若版本已占用必须顺延并同步 planned changes。
- 现有用户工作树 `/Users/lifcc/Desktop/code/AI/tool/remem` 很脏，禁止复用或清理；所有后端实现使用新的 clean worktree。
- 旧 `candidate_review` capability/endpoint 保持兼容。新 remem-web 写 UI 必须看 `candidate_review_safe`，不能把旧 flag 解释成安全审核契约。
- 任何仅含敏感内容的 evidence/resource 都宁可显示 redacted/blocked，不得 fallback raw。
- 永久 delete、raw transcript/blob 和跨 repo 非同 owner 工作均不在授权范围。
