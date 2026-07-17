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
- `memories.web_archive_operation_id TEXT NULL`
- 将五个 Web cursor source（`observations`、`sessions`、`workstreams`、`captured_events`、`extraction_tasks`）的主键重建为 `id INTEGER PRIMARY KEY AUTOINCREMENT`，保留全部既有 id、外键、索引、trigger 和 FTS 联动，并把 `sqlite_sequence` 推进到各表迁移时现存的 `MAX(id)`；所有 production writer 禁止显式分配这些 id
- `api_mutation_requests`
  - `idempotency_key_hash TEXT PRIMARY KEY`
  - `request_hash TEXT NOT NULL`
  - `operation_id TEXT NOT NULL UNIQUE`
  - `resource_kind TEXT NOT NULL`
  - `resource_id INTEGER NOT NULL`
  - `action TEXT NOT NULL`
  - `response_schema_version INTEGER NOT NULL`
  - `response_json TEXT NOT NULL`
  - `audit_id INTEGER NOT NULL`
  - `created_at_epoch INTEGER NOT NULL`

`src/migrate/run.rs` 为该 migration 增加专用、fail-closed 协议：任何 migration 在 `BEGIN IMMEDIATE` 前都先把 `PRAGMA foreign_keys` 设为 ON 并读回验证，统一安全入口与返回后置条件；若检测到 vNext pending，再在 transaction 外临时设为 OFF 并读回验证。随后开启 transaction、重建五个被引用表，并在 commit 前运行 `PRAGMA foreign_key_check` 与 `PRAGMA integrity_check`。任一 migration/check 失败都先 rollback；COMMIT 失败也必须尝试 rollback并保留两条错误链。成功 COMMIT 或成功 ROLLBACK 结束后，都在 transaction 外无条件设回 FK ON 并读回验证，恢复失败与原错误一起上抛，绝不返回 FK OFF 的可继续使用连接。若 ROLLBACK 本身失败，返回复合 fatal error，所有 in-repo caller 必须立即丢弃该 connection，不能继续查询或写入，也不能声称 FK 已恢复。入口 FK OFF、成功、注入 migration/check/commit 失败及成功 rollback 路径都验证 schema 原子性与返回时 FK ON。只有迁移完成后发出的新 cursor 才依赖永不复用保证。两个 versioned 表各增加一个 `AFTER UPDATE OF <web-visible columns>` trigger：候选 trigger 覆盖 detail DTO 中的内容、route/provenance、风险、阻塞与审核状态字段；memory trigger 覆盖 detail DTO 中的内容、ownership/project、type/topic/scope/branch、status 和 `updated_at_epoch`。trigger 只执行 `version = version + 1`，不监听 `version` 自身，因而现有 Web、CLI、worker、lifecycle 和 batch writer 只要改变 Web 可见字段都会推进版本；仅访问计数等不可见字段不制造冲突。另一个 `AFTER UPDATE OF status` trigger 在 `OLD.status IS NOT NEW.status` 时无条件清空 `web_archive_operation_id`，使所有现有 CLI/worker/lifecycle/batch status writer 都会撤销旧 Web restore 权限；Web archive 在状态更新及该 trigger 执行后，才在同一事务写入当前 `operation_id`，Web restore 的状态更新则由 trigger 自动清空 marker。schema drift 测试必须把 DTO 字段 allowlist 与 version trigger 列集合绑定，并验证 status trigger 的清除语义。版本可在一个复合 domain mutation 中推进多次，客户端只依赖单调变化和响应中的最终值，不假设恰好 `+1`。`request_hash` 对规范化后的 action、resource id 和业务请求体计算；不包含 bearer token 或原始幂等标识。相同 key 摘要 + 相同 hash 返回已持久化响应，相同 key 摘要 + 不同 hash 返回 `409 idempotency_conflict`。

所有安全写 endpoint 先 trim 幂等标识，再要求规范化结果为 `1..=128` ASCII bytes 且完全匹配 `[A-Za-z0-9._~-]+`；空白、超长、Unicode、控制字符或字符集外字符在事务、hash、持久化和日志之前返回 `400 idempotency_key_invalid`。该错误发生在 operation 建立前，不返回 `operation_id`，只可返回与幂等值无关的 request/trace id。服务端只持久化规范化值的 SHA-256 `idempotency_key_hash`；原始值和规范化明文均不得进入 DB、audit、日志、response 或错误 envelope。校验通过后，`operation_id` 由服务端对规范化幂等标识和固定 namespace 计算稳定、不可逆的 SHA-256 标识，因此后续事务前错误和成功重放拥有同一诊断标识，又不需要在业务事务外预写半完成记录。成功时，业务状态、audit event 和 `api_mutation_requests` 在同一 immediate transaction 提交。首次 envelope 和 ledger 都写 `response_schema_version=1`；账本中的 `response_json` 固定保存首次成功 envelope（`replayed=false`）。同 key hash/request hash 重放只接受当前二进制显式支持的 response schema，解析该安全 JSON、保留原 `operation_id`/`audit_id`/时间/版本并仅把返回值的 `replayed` 派生为 true，不产生第二条 audit；未知 schema 返回结构化 `idempotency_schema_unsupported`，不能猜测或丢弃旧结果。

### 2. Candidate detail 与 evidence 安全投影

新增 `GET /api/v1/candidates/{id}`。`candidate_detail` 与 `candidate_evidence` 两个 endpoint key 都显式映射到该 route；evidence projection 是 detail response 的组成部分，而不是隐含或未声明的第二个 URL。handler 只查询显式列，不直接序列化 `memory_candidates` 或 `captured_events`：

- candidate：当前可编辑字段、route/risk/review 状态、`version`、时间戳；
- evidence：`source_kind`、稳定 source id、event type、role/tool、时间、server-generated summary、bounded preview、provenance 状态；
- decision：`can_review`、`blocked_reasons[]`。

`evidence_event_ids` 必须解析为正整数集合并按候选记录顺序解析。每项只允许关联同 project 的 `captured_events`；跨 project、缺失、被 retention 清理、policy suppressed、payload 无法安全投影或 candidate 已非 `pending_review|quarantined` 都产生稳定阻塞码。至少一项 evidence 不能验证时 fail closed：详情仍 200，但 `can_review=false`。

summary/preview 只能从 allowlisted event metadata 或已批准的服务端派生安全投影生成，再经统一脱敏并按 Unicode 字符边界截断。DTO/query 禁止读取或复制 `captured_events.content_text`、`event_blobs.content_bytes`、raw messages、环境变量和未分类 JSON payload；通用 `redact_sensitive_text` 本身不构成把 raw content 公开给 Web 的安全投影。只有 raw content、`session_stop` payload 或无批准派生摘要的 evidence 返回空 preview、`redacted=true`、稳定阻塞码 `evidence_safe_projection_unavailable` 且 `can_review=false`，不能回退原文。

### 3. Candidate 安全审核写入

旧 `/api/v1/candidates/{id}/approve|reject|edit` endpoint、空 approve body 和旧 edit payload 原样保留给 `features.candidate_review`。新安全能力使用不冲突的 `/api/v1/candidates/{id}/review/approve|reject|edit` 路径，并由 `features.candidate_review_safe` 控制；三个 safe body 统一要求：

```json
{
  "reason": "human-readable reason",
  "expected_version": 4,
  "idempotency_key": "client-generated stable key"
}
```

approve 可额外携带 `acknowledge_pattern`；edit 可携带现有 editable fields。handler 完成 key/body 规范化、计算 request hash 并开启 immediate transaction 后，先按 `idempotency_key_hash` 查询当前请求 ledger：same key hash + same request hash + 支持的 response schema 直接返回首次 envelope（仅派生 `replayed=true`），不读取/锁定 candidate，也不重建 evidence；same key hash + different request hash 优先返回 `idempotency_conflict`。只有 ledger miss 才按 `id AND version AND review_status IN (...)` 锁定/校验并调用抽出的 transaction-scoped review primitive。旧 CLI wrapper 继续自行开启事务；Web wrapper 不嵌套事务。

`reason` 先 trim，要求 UTF-8 字节长度 `1..=1024`；规范化后的 reason 是 request hash 的一部分，并以原规范化值写入同事务 `events.detail.reason` 与首次 `response_json` 对应的 audit 语义。重放沿用首次 audit/reason，不新增或覆盖 reason。超长、空白或无效 reason 返回 `400 reason_invalid`。

仅 ledger miss 时再次构建 evidence 决策，不能信任客户端传入的 `can_review`。动作资格固定为：approve 允许 `pending_review`（禁止 acknowledgement）或 `quarantined`（必须携带与服务端 `quarantine_pattern_id` 精确相同的 `acknowledge_pattern`）；reject 允许两种状态且禁止 acknowledgement；edit-and-approve 允许两种状态，但编辑后的文本必须重新扫描且不含 instruction pattern，禁止 acknowledgement。其它状态统一为 `candidate_not_reviewable`。事务内按顺序完成：当前请求 ledger replay/conflict 检查；若 miss，再做版本/状态/动作资格与 evidence fail-closed 检查、promotion 或 discard、由 visible-field trigger 推进 candidate version、含规范化 reason 的 `events` audit 插入、幂等响应插入、commit。任何一步失败全部回滚。首次成功后即使 candidate version/status 已改变，原请求 replay 仍返回同一 `operation_id`、`audit_id` 和最终 version，不产生第二次 mutation/audit；different payload 即使当前 candidate 已不可审核也优先返回 `idempotency_conflict`。

成功 envelope：`response_schema_version`、`operation_id`、`audit_id`、`candidate_id`、`memory_id?`、`action`、`before_status`、`after_status`、`version`、`occurred_at_epoch`、`replayed`。结构化冲突至少区分 `version_conflict`、`candidate_not_reviewable`、`evidence_blocked`、`idempotency_conflict` 和 `idempotency_schema_unsupported`。

现有 `features.candidate_review` 保持旧含义，避免旧客户端被破坏；remem-web 只有同时看到 `candidate_detail`、`candidate_evidence`、`candidate_review_safe` 和声明 endpoint 才启用写 UI。

### 4. 五类独立只读资源

新增五个独立 handler module，而不是一个返回异构 payload 的通用 endpoint：

| Capability | List | Detail | 数据源 |
| --- | --- | --- | --- |
| `observations` | `/api/v1/observations` | `/api/v1/observations/{id}` | `observations` |
| `sessions` | `/api/v1/sessions` | `/api/v1/sessions/{id}` | `sessions` + 安全 summary/ref |
| `workstreams` | `/api/v1/workstreams` | `/api/v1/workstreams/{id}` | canonical `workstreams` + refs |
| `events` | `/api/v1/events` | `/api/v1/events/{id}` | `captured_events`，显式排除 blob/raw content |
| `tasks` | `/api/v1/tasks` | `/api/v1/tasks/{id}` | `extraction_tasks`，不返回 payload/last_error 原文 |

所有 list 默认按 migration 保证永不复用、单调递增的 AUTOINCREMENT `id DESC` keyset 排序；时间字段只用于展示和筛选，不进入 cursor，因此 session 的 `last_seen_at_epoch` 更新不会把已读行移过边界，legacy observation 的 nullable `created_at_epoch` 也不会丢行。五个 endpoint 共用 integer `page_size`：省略时为 50，解析成功后服务端以 `clamp(1, 100)` 得到 effective page size，并设置 `raw_scan_budget = max(100, effective_page_size * 10)`（最大 1000）。SQL 分批扫描 `id < resume_before_id`，直到收集恰好 effective page size 个通过 auth/project/suppression 的安全投影、raw rows 耗尽或达到预算；不消费未返回的 safe lookahead。填满页面时，continuation 的 `resume_before_id` 取最后一个已返回 safe row id；未填满且预算耗尽时取最后一个已扫描 raw row id；已耗尽时 `next_cursor=null`。因此 suppression 行会推进预算型 cursor，而任何未返回 safe row 都不会被越过。malformed、非整数或整数溢出返回 `400 page_size_invalid`。opaque cursor 是版本化 base64url JSON，仅含 kind、resume_before_id 和包含 effective page size 的筛选 fingerprint；服务端严格解码并校验 endpoint/fingerprint。预算耗尽可以返回 partial/空 `data` 加非空且严格前进的 cursor；只有 eligible raw scan 确认耗尽才返回终止 cursor。cursor 不承诺冻结快照，但同一 `id < resume_before_id` 边界稳定，页内不重复；清理当前最大行后重新插入得到更大 id，不会混入旧 cursor；非法、跨 endpoint、筛选不匹配或不支持版本返回 `400 cursor_invalid`。

每类使用专用 response DTO 和 SQL allowlist。observations/events 仅返回从 allowlisted metadata 或已批准派生数据生成的安全 summary/bounded preview；events query/DTO 绝不选择或复制 `captured_events.content_text`，即使它经过通用脱敏或截断。sessions/workstreams/tasks 的关联项只返回 `{kind,id,title?,status?}` 安全引用，detail 不递归展开。tasks 的 `last_error` 只返回服务端分类码和 `has_error`，不返回原文。该 raw transcript 禁止只覆盖 GH-880 新增 candidate detail/evidence 与五类 read-resource endpoint 及其关系展开；legacy `/api/v1/search.raw_hits[].preview` 保持当前兼容契约并由单独 regression test 锁定，本规格不把它误报为安全投影。

五类 list/detail 在投影前统一执行 fail-closed `resource_projection_policy`。所有 active pattern suppression 匹配每个将要公开的文本字段；可解析的 memory/topic/entity relation 还应用相应 active suppression，并省略/阻止包含被 suppress row/ref 的投影。list 不返回命中项，detail 返回 404 以避免 existence leak；policy DB 查询或评估错误返回结构化 5xx，不能降级为空集或继续返回文本。本轮不提供 bearer-only `include_suppressed`，未来 audit override 必须有独立 capability 与 auth contract。revoked suppression 恢复正常读取。

### 5. Archive / restore

新增：

- `POST /api/v1/memories/{id}/archive`
- `POST /api/v1/memories/{id}/restore`

请求要求 `reason`、`expected_version`、`idempotency_key`。reason 使用同一 `trim + 1..=1024 UTF-8 bytes` 规则，进入 request hash，并原值持久化到同事务 audit detail；idempotency key 使用第 1 节统一校验与摘要规则。完成规范化/request hash 并开启 transaction 后，首先查询“当前 archive/restore 请求 ledger”：same key hash + same request hash 直接返回首次成功 envelope（仅派生 `replayed=true`），不读取当前 memory；same key hash + different request hash 优先返回 `idempotency_conflict`。只有 ledger miss 才读取 memory 并执行 `expected_version`、status、marker 和 provenance 校验。archive 只允许 `active -> archived`；其它状态返回 `memory_not_archivable`。Web archive 先完成 status transition（由通用 trigger 清除任何旧 marker），再在同一事务把本次 `operation_id` 写入 `web_archive_operation_id`。restore 只允许仍为 archived、当前非空 marker 与同一 memory/action 的历史成功 `memory_archive` ledger/audit 精确匹配时执行 `archived -> active`；该“restore provenance lookup”与当前请求 replay lookup 不同，且只在当前请求 ledger miss 时执行。restore 的 status transition 自动清空 marker。历史 ledger 行本身永远不能授权当前 archived 状态：Web archive -> Web restore -> scope cleanup/stale cleanup/preference removal 等非 Web archive 后必须返回 `404 memory_not_recoverable`；随后新的成功 Web archive 可写入新 marker 并再次恢复。ledger miss 时，不存在、永久 deleted、没有当前合格 marker 或 marker/ledger 不匹配返回 `404 memory_not_recoverable`，版本不符返回 `409 version_conflict`；已完成请求即使资源后来删除或变化，same-key/same-hash replay 仍返回历史首次结果且不产生第二次 mutation/audit。

两动作复用 `memory::governance` 的 target validation 和 audit 逻辑，但新增 transaction-scoped 单项 primitive，避免 Web handler 与现有函数嵌套事务。成功时更新 status/updated_at（version trigger 自动推进）、插入 `events` audit、插入幂等响应并一次提交。响应使用与 candidate review 相同 audit envelope：`response_schema_version`、`operation_id`、`audit_id`、`memory_id`、`action`、`before_status`、`after_status`、`version`、`occurred_at_epoch`、`replayed`。

canonical `/api/v1/memories` 的无 `status` 行为保持不变；`/api/v1/memories` list 和 `/api/v1/memories/{id}` detail 对治理读取以 additive 字段返回当前整数 `version`，包括显式 `status=active` 与 `status=archived` 的结果。客户端从该读取取得 `expected_version`；archive/restore 成功 envelope 返回 mutation 后最终 `version`，该值可直接驱动下一次 restore/archive。remem-web 的默认治理列表显式请求 `status=active`，audit/restore 视图显式请求 `status=archived`。默认 search 继续使用现有 current-memory filter 排除 archived。这样 archive 后从控制台默认 list/search 消失，而旧客户端的无 status list 不发生破坏性变化。

不注册永久 delete REST route。capabilities 明确返回 `memory_delete=false`，且 endpoint map 中不存在 delete 项。

### 6. Capability、兼容与发布

新增布尔 capability：

- `candidate_detail`, `candidate_evidence`, `candidate_review_safe`
- `observations`, `sessions`, `workstreams`, `events`, `tasks`
- `memory_archive`, `memory_restore`, `memory_delete`

endpoint map 必须显式包含 `candidate_detail -> /api/v1/candidates/{id}` 和 `candidate_evidence -> /api/v1/candidates/{id}`；两项 capability 共享 route 不允许省略任一 key。T5 contract 与 native smoke 同时断言这两个声明及其相同路径。

T2–T4 可以先合并 route/domain/tests，但所有新增 flag 保持 false、endpoint map 不声明；只有 T5 同一发布 slice 完成 contract、native smoke、版本同步和 release evidence 后才把对应 flag 置 true 并加入 endpoint map。新字段对 JSON 客户端是 additive；旧 endpoint、offset list meta、旧 candidate review payload/response 不在本轮删除。

实现 PR 更新 `docs/specs/SPEC-web-api.md`、README/release guidance、CHANGELOG 和所有版本 manifest。只有包含这些能力的 remem release 发布后，remem-web 才将其视为 installed-binary 可用。

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| B-001–B-002 | candidate detail/evidence projector | detail contract tests；missing/cross-project/redaction/policy/quarantine fail-closed；raw `content_text` sentinel 不进入响应，raw-only evidence 返回 `evidence_safe_projection_unavailable` |
| B-003–B-004 | mutation ledger + transaction-scoped candidate review | legacy payload compatibility、跨 CLI/batch writer version race、状态改变后的 stale-request same-key replay、different-payload-first conflict、unknown response schema、reason/key validation、raw key absence、rollback/audit E2E |
| B-005–B-006 | capability DTO + five handler modules + resource projection policy | 每能力 true/false 与 endpoint 对应；list/detail empty/not-found/auth/server-error；pattern 与 relation suppression、revocation、policy failure tests |
| B-007 | FK-safe AUTOINCREMENT migration protocol + shared typed cursor codec + keyset SQL | 五表 fresh/upgrade/rebuild/rollback/FK restore、delete-max/reinsert id growth、purge between pages、suppressed-row scan advancement、full/partial/empty budget pages、concurrent insert、repeat/boundary/page-size/error tests |
| B-008–B-010 | safe derived read-model projectors | GH-880 新 endpoint 的 token/secret/env/raw transcript/payload 与 non-secret raw sentinel fixtures；`content_text` 不进入 candidate/events DTO；relationship non-expansion；true empty response；legacy search raw-hit contract regression |
| B-011–B-014 | memory archive/restore transaction primitive + current archive marker | active/archived list/detail version、状态/marker 改变后的 current-request replay、different-payload-first conflict、version/not-recoverable/rollback/audit envelope；ledger-miss provenance enforcement；fresh Web archive 重建 marker；delete absence |
| B-015 | existing auth middleware + parameterized query builders | unauthenticated route matrix、malformed id/cursor、SQL metacharacter filters、error body leak assertions |
| B-016 | additive capability/route contract | existing API regression suite and legacy candidate review tests unchanged；candidate detail/evidence endpoint keys 都声明且映射同一路径 |
| B-017 | contract/smoke/version manifests | native API smoke, version sync, version bump and release-contract checks |

## 数据流

### Read

`bearer request -> auth middleware -> typed query/path validation -> parameterized DB scan -> fail-closed resource_projection_policy -> explicit safe DTO projection -> server-side redaction -> bounded JSON response`

任何 policy/redaction/projector 错误返回结构化 5xx；不允许 warning + 原文 fallback，也不把错误变成空数组。suppressed detail 返回 404，suppressed list row 被省略但推进扫描 cursor。

### Write

`bearer request -> idempotency-key validation -> deterministic operation_id -> remaining strict body validation -> deterministic request_hash -> BEGIN IMMEDIATE -> idempotency lookup -> version/status/evidence validation -> domain mutation -> audit event -> replay row -> COMMIT -> envelope`

key 校验通过、operation 已建立后的 validation 或事务错误返回同一 `operation_id` 和机器码；事务错误执行 rollback。operation 前的 `idempotency_key_invalid` 只返回独立 request/trace id。任何错误都不返回 SQL、候选原文、event detail、token 或幂等明文。

## 备选方案

- 用 `updated_at_epoch` 作为 version：拒绝。秒级精度会漏掉同秒并发更新。
- 在 Web 暴露 raw transcript/blob 并由前端脱敏：拒绝。会让浏览器成为敏感数据边界且无法阻止关系展开泄露。
- 为所有资源复用一个通用 rows endpoint：拒绝。无法逐能力 gate，也容易把未来列意外序列化。
- archive 复用既有 destructive `delete` action：拒绝。状态语义、恢复路径和 capability 都不清晰。
- offset pagination：只保留给旧 endpoint；高变动新资源使用 keyset cursor。
- 继续把普通 `INTEGER PRIMARY KEY` 当永不复用 cursor key：拒绝。清理最大 row 后 SQLite 可复用 rowid；五个 Web source 统一重建为 AUTOINCREMENT，而不是依赖时间/id 组合猜测顺序。

## 风险

- Security：candidate evidence/events/observations 可能含 raw transcript、secret 或 active suppression 命中内容。GH-880 新 endpoint 只从 allowlisted metadata/批准派生数据构造投影，先执行 fail-closed resource policy，再统一脱敏和截断；`captured_events.content_text`、raw blob/detail 不进入这些 query/DTO，并用 non-secret sentinel、secret 和 suppression fixture 证明。
- Compatibility：API/数据语义是 additive，旧 endpoint 不改变，legacy search raw-hit preview 由 regression test 保持；但 schema vNext 含五个 source table 的物理 rebuild 例外。rebuild 必须保留 id/FK/index/trigger/FTS，并通过 legacy schema convergence/drift/integrity、注入失败 rollback 与 FK 恢复测试。
- Concurrency：嵌套事务会破坏原子性。domain mutation 分离为 transaction-scoped primitive 与 CLI wrapper，两条路径共测。
- Idempotency：响应 JSON schema 漂移会影响旧 replay。账本和 envelope 都记录 `response_schema_version`，只解析显式支持版本；不自动重写或猜测历史记录。幂等标识先严格校验，账本仅保存 SHA-256 摘要，raw/normalized key 不进入 DB、audit、日志或响应。
- Performance：五类 list 使用 AUTOINCREMENT `id` keyset、缺省 50/服务端 clamp 1..100 和最多 1000 raw rows 的 bounded batched scan；suppression 稀疏命中时可以返回 partial/空页加严格前进 continuation，不能越过未返回 safe row、递归 N+1 或产生空页循环。
- Maintenance：共享 cursor、redaction、mutation ledger 只提供窄 helper；各资源仍保留专用 DTO/query，文件超过 800 行前拆分。
- Release race：source 完成但 binary 未发布时 web 不应误报。前端继续以 capability + 最低版本 release 证据双重 gate。

## 测试计划

- [ ] migration vNext：fresh DB、v69 upgrade、version defaults/index/unique conflict、`idempotency_key_hash` schema；run.rs 在 BEGIN 前强制/验证 FK ON，vNext pending 时临时关闭/验证，五表 rebuild 保留 id/FK/index/trigger/FTS，commit 前 foreign_key/integrity check；入口 FK OFF、成功、注入 migration/check/commit 失败与成功 rollback 均验证无部分 schema且返回时 FK ON；rollback 自身失败返回要求丢弃 connection 的复合 fatal error；每表 post-migration delete current max 后新 id 大于迁移时现存 MAX，production writer 无显式 id；visible-field/status triggers 覆盖全部 writer。
- [ ] candidate detail：真实安全派生 evidence、缺失/跨 project/suppressed、malformed ids、raw-only/`session_stop` 返回 `evidence_safe_projection_unavailable`、non-secret transcript sentinel 和 secret 均不进入 JSON、not found。
- [ ] candidate review：旧 route/payload 回归、safe approve/reject/edit、reason、expected_version、跨 writer version conflict；首次成功后用原 stale version/已变化 status 重放仍返回同 operation/audit/final version 且无第二 mutation/audit，different payload 即使 candidate 已不可审核也先 conflict；unknown response schema、evidence recheck、rollback/audit；非法 key 与 raw-key absence。
- [ ] observations/sessions/workstreams/events/tasks：各自 list/detail、empty、not found、auth、server error、cursor repeat/invalid/concurrent insert；每表 page1/page2 间 purge current max/reinsert 无复用/重复/跳过；active pattern 与 relation suppression、revocation、policy DB failure=5xx；safe/suppressed interleave 不跳过 lookahead，all-suppressed 跨多预算批次返回严格前进 continuation，预算边界覆盖 full/partial/empty/terminal；page-size clamp/error 契约。
- [ ] redaction：Bearer/API key/cookie/env assignment/raw transcript/JSON secret 与 non-secret raw sentinel fixtures 在 GH-880 新 endpoint 的任何响应字段/关系展开中均不存在，events query/DTO 不读取 `content_text`；legacy `/api/v1/search.raw_hits` contract regression 保持现有兼容行为。
- [ ] archive/restore：active/archived list/detail version 与 success final version；archive 后 archived、restore 后 active/marker cleared 的 same-key replay 均返回首次 envelope且无第二 audit，different payload 优先 conflict；新 key/ledger miss 仍严格执行 version/status/marker/provenance，非 Web archive 不可恢复；非法 key、rollback、raw-key absence/hash present、default query exclusion/audit lookup。
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

实现拆为三个默认关闭能力的后端 slice：candidate 安全审核、五类只读资源、archive/restore；T5 完成 smoke/contract/release 后才统一发布对应 capability。发生回归时先在 patch release 将对应 capability 置 false 并移除 endpoint map 声明，再回退 handler/domain 改动。

schema vNext 的 API/数据语义是 additive，但五个 cursor source 会物理 rebuild；旧 binary 会因 newer-schema guard fail closed，不能直接读取已升级 DB。migration 内失败必须原子 rollback；成功 commit 或 rollback 结束后必须恢复/验证 `foreign_keys=ON`，rollback 本身失败则调用方立即丢弃连接。升级完成后的版本回滚必须继续使用理解 vNext 的兼容 binary，或从迁移前备份恢复整个数据库，不得手工降 `user_version`、反向 rebuild 或删除列/trigger/账本。已 archive 的 memory 保持 archived，回滚不能自动 restore；已成功审核的 candidate 也不逆转。账本和 audit event 保留供诊断。任何 rollback 重新运行 FK/integrity、schema convergence、API regression、version sync 和 native smoke，不能靠吞异常或返回空数据降级。

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
    "src/migrate/run.rs",
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

本规格包只定义实现与验证要求，不能自行授予合并权限。合并必须由当前对话中的用户明确授权并记录到 gate evidence；即使已有授权，仍须满足 route gate、独立 reviewer、当前 HEAD CI、零 unresolved threads、clean merge state 和串行 PR gate。release 发布不在本规格授权范围内。
