# Tech Spec：安全本地控制台证据、只读资源与可恢复治理

## Linked Issue

GH-880

## Product Spec

[`product.md`](product.md)

## Codebase Context

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| REST router / capability discovery | `src/api/server.rs`, `src/api/handlers/capabilities.rs`, `src/api/types.rs` | API 已受 loopback bearer token 保护；candidate list/review、memory list/detail 等 endpoint 已存在，capability 为平面布尔值和 endpoint map | 新能力必须逐项声明，旧客户端语义不能被重定义 |
| Candidate read/review | `src/api/handlers/candidates.rs`, `src/api/handlers/candidate_review.rs`, `src/memory_candidate/review.rs`, `src/memory_candidate/review/approval.rs` | 列表只返回 `evidence_count`；写请求不要求 reason/version/idempotency；approve/edit 已用 immediate transaction，reject 是单条条件更新 | 需要补详情、证据投影、乐观并发、幂等和同事务审计，同时复用现有 promotion 规则 |
| Capture / observations | `src/migrations/v001_baseline.sql`, `src/migrations/v006_capture_pipeline.sql`, `src/db/query/queries.rs`, `src/adapter/redaction.rs` | observations、sessions、captured_events、extraction_tasks 已持久化；通用敏感文本脱敏器已存在；当前 REST 未公开这些行 | 只读 API 应从现有真数据构建安全投影，不能直接序列化 DB model 或 payload |
| Workstreams | `src/workstream.rs`, `src/migrations/v001_baseline.sql`, `src/migrations/v053_workstream_identity_continuity.sql` | workstream 和 session 关系已稳定存储，且支持 canonical/merged identity | list/detail 必须只暴露 canonical 安全字段和安全引用 |
| Memory governance | `src/memory/governance.rs`, `src/api/handlers/list.rs`, `src/api/handlers/detail.rs` | CLI governance 能写 status 和 `events` audit，但动作集合不含 archive/restore；默认 memory 查询排除 archived | 新 Web 写能力应复用状态、事务、FTS 可见性和 audit 基础，不暴露 delete |
| Schema / migrations | `src/migrate/types.rs`, `src/migrate/schema_drift/invariants.rs`, `src/migrations/v069_job_queue_atomicity.sql` | 最新 schema 为 v69；candidate 和 memory 只有秒级 `updated_at_epoch`，不足以作为可靠并发版本 | 需要整数版本和通用 Web mutation 幂等记录 |
| Contract / smoke / release | `docs/specs/SPEC-web-api.md`, `scripts/smoke_native_web_api.sh`, `src/api/tests.rs`, release/version manifests | 当前 contract 只描述旧 candidate review；source capability 会被客户端立即发现 | 新写能力只有在测试、smoke、版本同步和发布边界一致时才能置 true |

## 设计方案

### 1. Schema v70：版本与幂等账本

新增单个 migration（实现时使用当时最新的下一个版本；若 main 已占用 v70，则顺延，不覆盖已有 migration）：

- `memory_candidates.version INTEGER NOT NULL DEFAULT 1`
- `memories.version INTEGER NOT NULL DEFAULT 1`
- `api_mutation_requests`
  - `idempotency_key TEXT PRIMARY KEY`
  - `request_hash TEXT NOT NULL`
  - `operation_id TEXT NOT NULL UNIQUE`
  - `resource_kind TEXT NOT NULL`
  - `resource_id INTEGER NOT NULL`
  - `action TEXT NOT NULL`
  - `response_json TEXT NOT NULL`
  - `audit_id INTEGER NOT NULL`
  - `created_at_epoch INTEGER NOT NULL`

`version` 每次受本契约管理的 mutation 加一，不用秒级时间戳代替。`request_hash` 对规范化后的 action、resource id 和业务请求体计算；不包含 bearer token。相同 key + 相同 hash 返回已持久化响应，相同 key + 不同 hash 返回 `409 idempotency_conflict`。

`operation_id` 由服务端对 idempotency key 和固定 namespace 计算稳定、不可逆的 SHA-256 标识，因此事务前错误和成功重放拥有同一诊断标识，又不需要在业务事务外预写半完成记录。成功时，业务状态、audit event 和 `api_mutation_requests` 在同一 immediate transaction 提交。账本中的 `response_json` 固定保存首次成功 envelope（`replayed=false`）；同 key/hash 重放时服务端解析该安全 JSON、保留原 `operation_id`/`audit_id`/时间/版本并仅把返回值的 `replayed` 派生为 true，不产生第二条 audit。

### 2. Candidate detail 与 evidence 安全投影

新增 `GET /api/v1/candidates/{id}`。handler 只查询显式列，不直接序列化 `memory_candidates` 或 `captured_events`：

- candidate：当前可编辑字段、route/risk/review 状态、`version`、时间戳；
- evidence：`source_kind`、稳定 source id、event type、role/tool、时间、server-generated summary、bounded preview、provenance 状态；
- decision：`can_review`、`blocked_reasons[]`。

`evidence_event_ids` 必须解析为正整数集合并按候选记录顺序解析。每项只允许关联同 project 的 `captured_events`；跨 project、缺失、被 retention 清理、policy suppressed、payload 无法安全投影或 candidate 已非 `pending_review|quarantined` 都产生稳定阻塞码。至少一项 evidence 不能验证时 fail closed：详情仍 200，但 `can_review=false`。

preview 先使用 `adapter::redaction::redact_sensitive_text`（或抽出的等价共享函数）再按 Unicode 字符边界截断；禁止读取 `event_blobs.content_bytes`、raw messages、环境变量和未分类 JSON payload。仅敏感内容时返回 `[redacted]`/空 preview 及 `redacted=true`，不能回退原文。

### 3. Candidate 安全审核写入

保留旧 endpoint 路径以兼容客户端，但新安全能力由 `features.candidate_review_safe` 控制。approve/reject/edit body 统一要求：

```json
{
  "reason": "human-readable reason",
  "expected_version": 4,
  "idempotency_key": "client-generated stable key"
}
```

approve 可额外携带 `acknowledge_pattern`；edit 可携带现有 editable fields。handler 在 immediate transaction 内按 `id AND version AND review_status IN (...)` 锁定/校验，再调用抽出的 transaction-scoped review primitive。旧 CLI wrapper 继续自行开启事务；Web wrapper 不嵌套事务。

`reason` 先 trim，要求 UTF-8 字节长度 `1..=1024`；规范化后的 reason 是 request hash 的一部分，并以原规范化值写入同事务 `events.detail.reason` 与首次 `response_json` 对应的 audit 语义。重放沿用首次 audit/reason，不新增或覆盖 reason。超长、空白或无效 reason 返回 `400 reason_invalid`。

审核前再次构建 evidence 决策，不能信任客户端传入的 `can_review`。动作资格固定为：approve 允许 `pending_review`（禁止 acknowledgement）或 `quarantined`（必须携带与服务端 `quarantine_pattern_id` 精确相同的 `acknowledge_pattern`）；reject 允许两种状态且禁止 acknowledgement；edit-and-approve 允许两种状态，但编辑后的文本必须重新扫描且不含 instruction pattern，禁止 acknowledgement。其它状态统一为 `candidate_not_reviewable`。事务内按顺序完成：幂等冲突检查、版本/状态/动作资格检查、evidence fail-closed 检查、promotion 或 discard、candidate version +1、含规范化 reason 的 `events` audit 插入、幂等响应插入、commit。任何一步失败全部回滚。

成功 envelope：`operation_id`、`audit_id`、`candidate_id`、`memory_id?`、`action`、`before_status`、`after_status`、`version`、`occurred_at_epoch`、`replayed`。结构化冲突至少区分 `version_conflict`、`candidate_not_reviewable`、`evidence_blocked` 和 `idempotency_conflict`。

现有 `features.candidate_review` 保持旧含义，避免旧客户端被破坏；remem-web 只有同时看到 `candidate_detail`、`candidate_evidence`、`candidate_review_safe` 和声明 endpoint 才启用写 UI。

### 4. 五类独立只读资源

新增五个独立 handler module，而不是一个返回异构 payload 的通用 endpoint：

| Capability | List | Detail | 数据源 |
| --- | --- | --- | --- |
| `observations` | `/api/v1/observations` | `/api/v1/observations/{id}` | `observations` |
| `sessions` | `/api/v1/sessions` | `/api/v1/sessions/{id}` | `sessions` + 安全 summary/ref |
| `workstreams` | `/api/v1/workstreams` | `/api/v1/workstreams/{id}` | canonical `workstreams` + refs |
| `events` | `/api/v1/events` | `/api/v1/events/{id}` | `events`，显式排除 raw detail |
| `tasks` | `/api/v1/tasks` | `/api/v1/tasks/{id}` | `extraction_tasks`，不返回 payload/last_error 原文 |

所有 list 默认按 `(created_at_epoch DESC, id DESC)`（session 使用 `(last_seen_at_epoch DESC, id DESC)`）keyset 排序。opaque cursor 是版本化 base64url JSON，仅含 kind、sort epoch、id 和筛选 fingerprint；服务端严格解码并校验 endpoint/fingerprint。`LIMIT page_size + 1` 生成 `next_cursor`，空集返回 `data: []`、`next_cursor: null`。cursor 不承诺冻结快照，但同一 cursor 的排序边界稳定，页内不重复；非法、跨 endpoint、筛选不匹配或不支持版本返回 `400 cursor_invalid`。

每类使用专用 response DTO 和 SQL allowlist。observations/events 仅返回脱敏 summary/bounded preview；sessions/workstreams/tasks 的关联项只返回 `{kind,id,title?,status?}` 安全引用，detail 不递归展开。tasks 的 `last_error` 只返回服务端分类码和 `has_error`，不返回原文。

### 5. Archive / restore

新增：

- `POST /api/v1/memories/{id}/archive`
- `POST /api/v1/memories/{id}/restore`

请求要求 `reason`、`expected_version`、`idempotency_key`。reason 使用同一 `trim + 1..=1024 UTF-8 bytes` 规则，进入 request hash，并原值持久化到同事务 audit detail；重放沿用首次 audit/reason。archive 只允许当前非 deleted/rejected 的可治理 memory，目标状态为 `archived`；对已 archived 的同请求走幂等重放。restore 只允许 `archived -> active`，不存在/永久 deleted 返回 `404 memory_not_recoverable`，版本不符返回 `409 version_conflict`。

两动作复用 `memory::governance` 的 target validation 和 audit 逻辑，但新增 transaction-scoped 单项 primitive，避免 Web handler 与现有函数嵌套事务。成功时更新 status/version/updated_at、插入 `events` audit、插入幂等响应并一次提交。响应使用与 candidate review 相同 audit envelope：`operation_id`、`audit_id`、`memory_id`、`action`、`before_status`、`after_status`、`version`、`occurred_at_epoch`、`replayed`。

不注册永久 delete REST route。capabilities 明确返回 `memory_delete=false`，且 endpoint map 中不存在 delete 项。

### 6. Capability、兼容与发布

新增布尔 capability：

- `candidate_detail`, `candidate_evidence`, `candidate_review_safe`
- `observations`, `sessions`, `workstreams`, `events`, `tasks`
- `memory_archive`, `memory_restore`, `memory_delete`

每个 true 值必须同时满足 endpoint 已注册、token middleware 覆盖、契约测试和 native smoke 已完成。未完成 slice 保持 false 且不声明 endpoint。新字段对 JSON 客户端是 additive；旧 endpoint、offset list meta 和 candidate review response 不在本轮删除。

实现 PR 更新 `docs/specs/SPEC-web-api.md`、README/release guidance、CHANGELOG 和所有版本 manifest。只有包含这些能力的 remem release 发布后，remem-web 才将其视为 installed-binary 可用。

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| B-001–B-002 | candidate detail/evidence projector | detail contract tests；missing/cross-project/redaction/policy/quarantine fail-closed tests |
| B-003–B-004 | mutation ledger + transaction-scoped candidate review | version race、same-key replay、different-payload conflict、reason validation、rollback/audit E2E |
| B-005–B-006 | capability DTO + five handler modules | 每能力 true/false 与 endpoint 对应；list/detail empty/not-found/auth/server-error tests |
| B-007 | shared typed cursor codec + keyset SQL | concurrent insert、repeat cursor、boundary id、invalid version/filter/endpoint tests |
| B-008–B-010 | redacted read-model projectors | token/secret/env/raw transcript/payload fixtures；relationship non-expansion；true empty response |
| B-011–B-014 | memory archive/restore transaction primitive | success/replay/version conflict/not-recoverable/rollback/audit envelope；delete absence |
| B-015 | existing auth middleware + parameterized query builders | unauthenticated route matrix、malformed id/cursor、SQL metacharacter filters、error body leak assertions |
| B-016 | additive capability/route contract | existing API regression suite and legacy candidate review tests unchanged |
| B-017 | contract/smoke/version manifests | native API smoke, version sync, version bump and release-contract checks |

## 数据流

### Read

`bearer request -> auth middleware -> typed query/path validation -> parameterized DB read -> explicit safe DTO projection -> server-side redaction -> bounded JSON response`

任何 redaction/projector 错误返回结构化 5xx；不允许 warning + 原文 fallback，也不把错误变成空数组。

### Write

`bearer request -> strict body validation -> deterministic operation_id/request_hash -> BEGIN IMMEDIATE -> idempotency lookup -> version/status/evidence validation -> domain mutation -> audit event -> replay row -> COMMIT -> envelope`

事务错误执行 rollback，错误 envelope 返回 `operation_id` 和机器码，不返回 SQL、候选原文、event detail 或 token。

## 备选方案

- 用 `updated_at_epoch` 作为 version：拒绝。秒级精度会漏掉同秒并发更新。
- 在 Web 暴露 raw transcript/blob 并由前端脱敏：拒绝。会让浏览器成为敏感数据边界且无法阻止关系展开泄露。
- 为所有资源复用一个通用 rows endpoint：拒绝。无法逐能力 gate，也容易把未来列意外序列化。
- archive 复用既有 destructive `delete` action：拒绝。状态语义、恢复路径和 capability 都不清晰。
- offset pagination：只保留给旧 endpoint；高变动新资源使用 keyset cursor。

## 风险

- Security：events/observations 可能含 secret。使用显式列 allowlist、统一先脱敏后截断、泄漏回归 fixture，禁止 raw blob/detail。
- Compatibility：新增 `version` 和 capability 字段是 additive；旧 endpoint 不改变。migration 必须通过 legacy schema convergence/drift tests。
- Concurrency：嵌套事务会破坏原子性。domain mutation 分离为 transaction-scoped primitive 与 CLI wrapper，两条路径共测。
- Idempotency：响应 JSON schema 漂移会影响旧 replay。账本记录 schema/version，并在当前 API version 内保持可反序列化；不自动重写历史记录。
- Performance：五类 list 使用 `(epoch,id)` 索引和 `page_size + 1`；detail 关系查询设定硬上限，不做递归 N+1。
- Maintenance：共享 cursor、redaction、mutation ledger 只提供窄 helper；各资源仍保留专用 DTO/query，文件超过 800 行前拆分。
- Release race：source 完成但 binary 未发布时 web 不应误报。前端继续以 capability + 最低版本 release 证据双重 gate。

## 测试计划

- [ ] migration vNext：fresh DB、v69 upgrade、version defaults/index/unique conflict、schema drift/convergence。
- [ ] candidate detail：真实 evidence、缺失/跨 project/suppressed、malformed ids、only-sensitive preview、not found。
- [ ] candidate review：approve/reject/edit、reason、expected_version、idempotent replay、payload conflict、evidence recheck、transaction rollback、audit envelope。
- [ ] observations/sessions/workstreams/events/tasks：各自 list/detail、empty、not found、auth、server error、cursor repeat/invalid/concurrent insert。
- [ ] redaction：Bearer/API key/cookie/env assignment/raw transcript/JSON secret fixtures 在任何响应字段中均不存在。
- [ ] archive/restore：success、replay、same-key conflict、version race、already archived、not recoverable、rollback、default query exclusion、audit lookup。
- [ ] `cargo fmt --check`
- [ ] `cargo check --locked`
- [ ] focused `cargo test` suites for migration/API/governance/redaction
- [ ] `cargo test --locked --quiet`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `bash scripts/smoke_native_web_api.sh`
- [ ] `python3 checks/check_workflow.py --repo . --spec-dir specs/GH880`
- [ ] `python3 scripts/ci/check_plugin_version_sync.py`
- [ ] `python3 scripts/ci/check_version_bump.py <base-sha> HEAD`
- [ ] `python3 scripts/ci/check_pr_preflight.py --base <base-sha> --head HEAD --pr-body-file <body-file>`
- [ ] `git diff --check`

## 回滚方案

实现拆为三个可独立回退的后端 slice：candidate 安全审核、五类只读资源、archive/restore。每个 slice 的 capability 只在该 slice 完成时置 true；发生回归时先在 patch release 将对应 capability 置 false 并移除 endpoint map 声明，再回退 handler/domain 改动。

schema vNext 为 additive，不 down-migrate 或删除 `version`/幂等账本；旧 binary 会忽略新列。已 archive 的 memory 保持 archived，回滚不能自动 restore；已成功审核的 candidate 也不逆转。账本和 audit event 保留供诊断。任何 rollback 重新运行 schema convergence、API regression、version sync 和 native smoke，不能靠吞异常或返回空数据降级。

<!-- specrail-planned-changes
{
  "version": 1,
  "issue": 880,
  "complete": true,
  "paths": [
    "specs/GH880/product.md",
    "specs/GH880/tech.md",
    "specs/GH880/tasks.md",
    "src/migrations/v070_web_console_governance.sql",
    "src/migrate/types.rs",
    "src/migrate/schema_drift/invariants.rs",
    "src/migrate/tests_schema.rs",
    "src/api/server.rs",
    "src/api/types.rs",
    "src/api/handlers.rs",
    "src/api/handlers/capabilities.rs",
    "src/api/handlers/candidates.rs",
    "src/api/handlers/candidate_detail.rs",
    "src/api/handlers/candidate_review.rs",
    "src/api/handlers/observations.rs",
    "src/api/handlers/sessions.rs",
    "src/api/handlers/workstreams.rs",
    "src/api/handlers/events.rs",
    "src/api/handlers/tasks.rs",
    "src/api/handlers/memory_governance.rs",
    "src/api/cursor.rs",
    "src/api/mutation.rs",
    "src/api/tests.rs",
    "src/api/tests/candidates.rs",
    "src/api/tests/read_resources.rs",
    "src/api/tests/memory_governance.rs",
    "src/memory_candidate/review.rs",
    "src/memory_candidate/review/approval.rs",
    "src/memory/governance.rs",
    "src/adapter/redaction.rs",
    "scripts/smoke_native_web_api.sh",
    "docs/specs/SPEC-web-api.md",
    "README.md",
    "CHANGELOG.md",
    "Cargo.toml",
    "Cargo.lock",
    "plugins/remem/.codex-plugin/plugin.json",
    "plugins/remem/runtimes/remem-releases.json",
    "npm/remem/package.json",
    "server.json"
  ],
  "spec_refs": [
    "specs/GH880/product.md",
    "specs/GH880/tech.md"
  ]
}
-->

本规格在 `implx auto` 授权下可继续生成 tasks 并进入实现；这不豁免 route gate、独立 reviewer、CI、PR gate 或 release 证据。
