# Tech Spec

Status: Draft，等待 #822/PR #914 evidence 人工采用、GH-823/GH-825 人工规格批准；
不批准实现。

## Linked Issue

GH-825（Refs #825；关联 #821；依赖 #822、#823；安装态验证关联 #824）

## Product Spec

[`product.md`](product.md)

## Planned Changes Manifest

<!-- specrail-planned-changes
{
  "version": 1,
  "issue": 825,
  "complete": true,
  "paths": [
    "README.md",
    "docs/specs/README.md",
    "docs/specs/SPEC-web-api.md",
    "docs/specs/raw-session-ingestion/PRODUCT.md",
    "docs/specs/raw-session-ingestion/TECH.md",
    "specs/GH825/product.md",
    "specs/GH825/tech.md",
    "specs/GH825/tasks.md",
    "src/api/handlers/sessions.rs",
    "src/api/handlers/capabilities.rs",
    "src/api/types.rs",
    "src/api/tests.rs",
    "src/cli/actions/query/raw.rs",
    "src/cli/query_types.rs",
    "src/cli/tests_raw.rs",
    "src/context/query.rs",
    "src/context/tests/sessions.rs",
    "src/db/capture.rs",
    "src/db/capture/extraction_task.rs",
    "src/db/capture_drop.rs",
    "src/db/extraction/lifecycle.rs",
    "src/db/extraction/tests.rs",
    "src/db/query/stats.rs",
    "src/db/query/stats/tests.rs",
    "src/doctor/capture_liveness.rs",
    "src/doctor/database.rs",
    "src/doctor/tests.rs",
    "src/extraction_worker.rs",
    "src/ingest/sessions.rs",
    "src/ingest/sessions/tests.rs",
    "src/ingest/session_identity.rs",
    "src/ingest/session_identity/tests.rs",
    "src/memory/raw_archive.rs",
    "src/memory/raw_archive/tests.rs",
    "src/memory/raw_occurrence.rs",
    "src/memory/raw_query.rs",
    "src/memory/raw_reconcile.rs",
    "src/memory/raw_reconcile/tests.rs",
    "src/session_rollup/cursor_snapshot.rs",
    "src/session_rollup/cursor_transcript.rs",
    "src/session_rollup/mod.rs",
    "src/session_rollup/persist.rs",
    "src/session_rollup/prompt.rs",
    "src/session_rollup/raw_identity.rs",
    "src/session_rollup/side_effects.rs",
    "src/session_rollup/tests.rs",
    "src/session_rollup/tests/side_effects.rs",
    "src/session_rollup/transcript_evidence.rs",
    "src/summarize/summary_job/hook.rs",
    "src/summarize/summary_job/hook/tests.rs",
    "tests/api_public.rs"
  ],
  "spec_refs": [
    "docs/specs/SPEC-web-api.md",
    "docs/specs/raw-session-ingestion/PRODUCT.md",
    "docs/specs/raw-session-ingestion/TECH.md",
    "specs/GH825/product.md",
    "specs/GH825/tech.md"
  ]
}
-->

该清单不包含 migration：GH-825 明确禁止新表、列、索引或 schema version。GH-823 的协议实现
文件由 GH-823 自己的 packet 管理；本清单只列 GH-825 reader、既有 ledger/rollup/doctor 接点和
文档。若 #822 证明 parser 需要不同模块边界，必须先更新本 manifest 并重新人工批准。

## PR #874 / PR #914 真实证据修正

PR #874 exact head `34c7cad5f40c6ac1dae519461ad237fb7f549cb4` 记录了 Cursor IDE
3.6.31 的一次 foreground sessionStart→Stop：Stop 提供可读 JSONL 路径，文件恰有两行，顶层均
只有 `role` 与 `message`，文本位于 `message.content[].text`。证据明确没有逐消息 ID、
timestamp、sequence 或 token metadata。该单次样本不足以冻结 tool、长会话、错误/取消、压缩等
完整 grammar，相关实现门仍保持 blocked；但 exact fixture 是 v1 parser 的有效正例，不能因缺少
未观察的逐消息字段而标为 `format_invalid`，也不得合成这些字段。

PR #914 exact head `c0802c42c3fc22770aecb0b7b2eec88f117f795c` 记录了 Cursor IDE
3.12.17 的 foreground、subagent、MCP、manual compact 与 cancelled response。transcript
仍以 `{role,message}` 为 message record，assistant content 可含 `tool_use` block；另有独立
`{type:"turn_ended",status:"success"}` 或 cancelled error boundary record。未观察到独立
`tool_result` record。completed/aborted Stop 的 `loop_count` 均为 JSON number `0`；
completed 有 token fields，aborted 无 token fields。该 evidence 仍未冻结可信 root、platform
path、transcript max、非零/缺失/null loop、`status:error` 或跨版本完整 grammar。

## Codebase Context

以下锚点已在 2026-07-20 以当前 `origin/main` `2dc41cb332ead83ff39f234444fc76fc50713f43`
重新核对；该提交包含 v071 occurrence identity、#862 streamed raw-ingest rollback 和此前
#843 job atomicity。

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| Capture ledger | `src/db/capture.rs:15`, `src/db/capture.rs:115`, `src/db/capture/extraction_task.rs:7`, `src/db/capture/extraction_task.rs:31` | `CaptureEventInput` 写入 `captured_events`；超过 16 KiB 的脱敏内容进入 `event_blobs`；固定 event ID 幂等；同一 savepoint 可原子 coalesce pending extraction task，后到 event 会推进任务 high watermark | Cursor Stop 可与 durable SessionRollup task 原子落账；后到 IR/degraded companion 无需新字段即可推进同一任务范围 |
| Transaction primitives | `src/db/job/state.rs:292`, `src/db/capture.rs:115`, `src/db/capture_drop.rs:26`, `src/memory/raw_archive.rs:458` | job lifecycle 已示范 `TransactionBehavior::Immediate`；capture event/task只有自己的savepoint，drop目前是独立调用，尚无Cursor companion仲裁/bundle API | GH-825必须新增production helper把重查、companion、recoverable drop和task high-watermark包进同一immediate transaction；不得为 Cursor 调用无 recovery link 的 raw-failure writer |
| Capture schema | `src/migrations/v006_capture_pipeline.sql:44`, `src/migrations/v006_capture_pipeline.sql:54`, `src/migrations/v006_capture_pipeline.sql:76` | 已有 `event_blobs`、`captured_events`、`extraction_tasks`，`event_type` 是开放字符串，event ID 有 host/session 唯一约束 | 支持内部 snapshot/outcome event 与现有任务游标 |
| Drop diagnostics | `src/db/capture_drop.rs:26`, `src/db/capture_drop.rs:85`, `src/migrations/v036_capture_drop_events.sql:3` | `capture_drop_events` 已保存 reason/detail/recovered_event_id；stats 区分 actionable 与 recovered spill | transcript degradation 可写稳定 reason/locator，并用现有 recovery link 清除当前告警 |
| Raw failure history | `src/memory/raw_archive.rs:444`, `src/migrations/v028_raw_ingest_failures.sql:3` | `raw_ingest_failures` 是没有 recovery link 的 append-only legacy raw-ingest 失败史 | Cursor path/parser degradation 不写该表；Full raw insert 失败回滚整个 bundle并重试，避免制造永久 actionable 的半成品 failure row |
| Transcript identity | `src/ingest/session_identity.rs:164`, `src/session_rollup/raw_identity.rs:24` | v071 identity 以 `(source_root, transcript_path)` 稳定复用；Stop drain 先 resolve identity，再把其 ID 传给 raw archive；identity conflict 不允许 fallback | Cursor stable snapshot 必须映射到同一既有 path identity，Full companion 记录 Stop→identity 关联；不新增 message ID/schema |
| Raw occurrence/archive | `src/memory/raw_occurrence.rs:27`, `src/memory/raw_archive.rs:258`, `src/memory/raw_archive.rs:330`, `src/migrations/v071_raw_session_identity.sql:62` | `transcript_identity_id` 存在时唯一键是 identity+物理 record ordinal；ordinal 在每个 JSONL record 分类前递增，重复同文不同 ordinal 被保留；content-hash 唯一键只约束 identity 为 NULL 的行；streamed drain 在 savepoint 中失败回滚 | Cursor ordered role+message records 必须走 identified occurrence insert；完整验证先于 drain，ordinal/stable-field 冲突回滚整包 |
| Stop preparation | `src/summarize/summary_job/hook.rs:310` | `summary_payload_with_cwd()` 会在 Stop capture 前对 transcript path 调用 `metadata()`；缺失文件直接返回错误 | Cursor 必须走 Stop-first host branch，不能复用该 metadata-before-capture 顺序 |
| SessionRollup | `src/session_rollup/mod.rs:81`, `src/session_rollup/mod.rs:112` | 新范围先 drain raw、构造 bounded evidence、调用 AI、持久化 summary，再完成 checkpointed side effects；失败由 extraction task 重试 | Cursor 必须增加按 Stop/status 的 work planner 和写入前 authority-checked publish transaction；不得沿用混合 range 的单次 prompt 或在 terminal 前暴露 degraded side effects |
| Transcript evidence | `src/session_rollup/transcript_evidence.rs:206` | 当前按 Stop payload 的 path/byte_len 读取 Claude/Codex JSONL，再压到 64 KiB/128 message prompt budget | Cursor 必须 host-dispatch 到 ledger IR；prompt budget 不是外部 transcript 上限 |
| Stop payload/raw effects | `src/session_rollup/side_effects.rs:23`, `src/session_rollup/side_effects.rs:33` | `StopHookPayload` 没有 Cursor status/loop contract；raw archive reader仍以 path 为输入，并在 read failure 时用 last assistant fallback | GH-825 需消费 GH-823-normalized Stop 与 Cursor IR，禁止 Cursor 走旧 path reader |
| Follow-up checkpoint | `src/session_rollup/side_effects.rs:625` | Compress/Dream enqueue 已使用 transaction-aware API 并以 session summary checkpoint 幂等 | 不再设计 outcome revision/side-effect supersede；复用当前 checkpoint |
| Worker lifecycle | `src/extraction_worker.rs:18`, `src/extraction_worker.rs:47`, `src/db/extraction/lifecycle.rs:70`, `src/db/extraction/lifecycle.rs:182` | SessionRollup 成功后以 lease owner mark Done；`wait_extraction_task` 可把 processing 任务无 attempt 增量地放回 pending；mark Done 时若数据库 high watermark 已推进则重新排队 | worker 早于 IR companion 抢到 Stop task 时可走既有 Waiting；terminal/Done crash window靠 stable IDs 与既有幂等 checkpoint 收敛 |
| Doctor | `src/doctor/capture_liveness.rs:34`, `src/doctor/database.rs:200`, `src/doctor/database.rs:241` | doctor 已查询 capture drops/raw failures 和 capture heartbeat；历史 drop 当前可能一直 actionable | 增加 Cursor semantic-priority outcome 聚合和 recovered link，而非新状态表；不得用latest-row覆盖full |
| GH-823 contract | `specs/GH823/product.md:113`, `specs/GH823/product.md:143`, `specs/GH823/product.md:197`, `specs/GH823/tasks.md:16` | GH-823 独占 bounded tool decode、canonical identity/status 和 Stop wiring；SP823-T5 等 GH-825 reader | GH-825 只接收 approved normalized invocation，实施顺序必须打破旧“等待 GH-823 全部实现”的环 |

## 设计方案

### 1. 依赖门与实施顺序（P-001、P-002、P-003）

实现能力由三层 gate 组成：

1. PR #874/PR #914 exact-head 真实 PoC + 人工决定冻结 transcript format/version、可信根、
   平台 path 规则和 `CURSOR_TRANSCRIPT_MAX_BYTES`；PR #914 的 `loop_count: 0`
   completed/aborted 证据必须随后进入 GH-823 packet amendment，
   由 GH-823 在 exact head 人工批准字段类型、缺失/null、规范化和 canonical Stop key。
2. GH-823 SP823-T3/T4 提供 canonical `cursor` host、session/workspace identity、outer payload
   parser、PII removal、bounded tool decode 和 observe capture。GH-825 不修改这些语义。
3. GH-825 reader/outcome 先以未接线 capability 合入；随后 GH-823 SP823-T5 把 normalized Stop
   接到该 capability，联合测试后才启用。

GH-823 的 `CURSOR_TOOL_FIELD_MAX_BYTES` 是唯一 tool-field 限额。encoded/decoded malformed 或
over-limit 在 outer parser non-zero + zero writes，不能产生 capture/drop/spill。GH-825 只消费已
验证的通用 capture events，未知合法 tool name 原名保留。

GH-825 入口接收类似以下已验证值；字段名最终以 GH-823 人工批准为准：

```rust
struct ValidatedCursorStopInvocation {
    canonical_stop_key: String,
    session_id: String,
    project: String,
    cwd: String,
    status: CursorStopStatus,
    loop_key: CursorLoopKey,
    transcript_path: Option<String>,
}
```

### 2. status / loop_count 决策（P-003、P-010、P-011）

PR #914 已观察 `completed | aborted`；`error` 未观察，只有 GH-823 amendment 人工批准后才可
进入 accepted set。GH-825 消费已合入的 canonical 决策：每种已批准
状态均保留 Stop、IR/payload evidence；`completed` 和 `aborted` 有 usable evidence 时运行 grounded
SessionRollup，无 evidence 就 blank。若 GH-823 批准 `error`，它始终跳过 LLM、保持内容为空并写
error-level log；若未批准，outer parser 必须对 exact `status:error` non-zero、zero writes、
AI=0。这样中止会话仍可提取未完成工作，而 host 报错不会触发与 GH-823 相冲突的生成。

PR #914 证明 completed/aborted 的 `loop_count` 为 JSON number `0`，但没有证明非零、
缺失或 null。完整原始类型集合、是否必填、null/缺失和顺序继续由 #822 实测；规范化与是否参与 host event
identity 由后续 GH-823 packet amendment 独占定义并在 exact head 人工批准。未完成 amendment 时
GH-825 implementation route blocked。GH-825 只接收已批准的 `CursorLoopKey`，不得把缺失猜成 `0`
或自行冻结 key。canonical Stop key 驱动：

- 新 key：在同一 capture savepoint 原子写新 `session_stop` 与 pending SessionRollup task；随后
  snapshot/degraded companion 推进同一 task high watermark。
- 相同 key/status/evidence：复用同一 stable event ID，zero duplicate writes/LLM。
- 相同 key 但 status/evidence 冲突：outer integration non-zero、error log、zero new writes。
- 新的合法 loop key：是新的 Stop；同 session extraction task 可继续用现有 high-watermark
  coalescing 作为 scheduling envelope，但 worker 必须在 prompt/LLM 前按 canonical Stop key
  和 status 切分 work item。一个 coalesced range 含多个已批准状态时分别执行各自策略，禁止一次
  LLM 覆盖混合状态；只有 GH-823 批准 `error` 时，它才可进入 range 并执行 AI=0 policy，否则
  outer parser 在 task 创建前拒绝它。

### 3. Stop-first 与规范 transcript IR（P-004、P-005、P-006）

Cursor 分支不能调用当前 `summary_payload_with_cwd()` 的 metadata-before-capture 路径。顺序为：

1. 以 canonical Stop key 调用现有 capture API，并传 `task_kind = SessionRollup`；既有 capture
   savepoint 必须原子提交 `session_stop` 与 pending/coalesced task。事务失败时二者都不留下且
   outer integration non-zero；不得出现已提交 Stop 没有 durable task 的状态。
2. 对 `transcript_path` 执行 #822 批准的 absolute/trusted-root/platform/no-follow/regular-file
   检查。先从 descriptor 检查长度是否超过 `CURSOR_TRANSCRIPT_MAX_BYTES`，不能先无界读入。
3. 在同一短生命周期 hook invocation 内完整读取；复制期间检查 identity/length/mtime，计算 hash。
   worker 后续绝不重开原路径。
4. 在任何 transcript-derived DB write 前全量解析 format/version/encoding/records/tail。任一错误
   整体失败，不返回 prefix。
5. 对 PR #874/PR #914 已观察的 grammar，`{role,message}` record 即使没有逐消息
   ID/timestamp 也可验证成功；message content 接受批准的 `text` 与 assistant `tool_use`
   block。PR #914 的独立 `turn_ended` record 作为内部 boundary/status record 验证和保留，
   不伪装成 message；parser 不要求未观察到的独立 `tool_result`。所有 record 按完整稳定 JSONL
   snapshot 文件行序赋零基物理 ordinal，再做 record/role/usability projection；不得按 role、
   文本或 hash 排序，也不得合成 source message ID、timestamp 或 sequence。后续 host
   versioned grammar amendment 不能反向改变这些正例。
6. 将成功结果转换为受限、递归脱敏的 `ValidatedCursorTranscript` JSON，并连同安全 reader
   在同一 descriptor/boundary 下固定的 `trusted_source_root`、规范
   `normalized_transcript_path` 和完整稳定 snapshot bytes 组成
   `ValidatedCursorSnapshotInput`。此时它还不是 Full candidate。production helper 在自己的
   serialized transaction 内先以这组可信 locator 解析/复用 `transcript_identity_id`，再构造
   携带 `(trusted_source_root, normalized_transcript_path, transcript_identity_id)` 的
   `ResolvedCursorFullCandidate`；任何 Full companion/bundle write 只能接受该已解析类型，且
   不得从原始 Stop payload 重新解析 path/identity。Full companion 的
   stable event ID 由 canonical `(host_id, project_id, session_row_id, session_id)`、source
   Stop 的 stable `captured_events.event_id`、canonical Stop key 与 snapshot hash 共同派生；
   不得只用 Stop key/hash，因为同名 session/Stop 可出现在不同 project。以 validated input
   调用下节新增的 production helper `commit_cursor_snapshot_bundle`；16 KiB以上自然进入
   `event_blobs`。helper 在同一 transaction 内解析/复用既有 transcript identity、构造 Full、
   写 occurrence、仲裁 companion 并推进
   同一任务 high watermark；它是 Cursor raw occurrence insertion 的唯一 owner。SessionRollup
   worker 后续只读该 IR/raw evidence，不得再次 drain transcript 或写 raw。不新增 task state、
   preparing字段或专用表。

```rust
struct ValidatedCursorTranscript {
    source_stop_event_id: i64,
    canonical_stop_key: String,
    status: CursorStopStatus,
    loop_key: CursorLoopKey,
    snapshot_hash: String,
    snapshot_byte_len: u64,
    format_version: String,
    records: Vec<ValidatedCursorRecord>,
}

struct ValidatedCursorSnapshotInput<'a> {
    trusted_source_root: String,
    normalized_transcript_path: String,
    stable_snapshot_bytes: &'a [u8],
    transcript: ValidatedCursorTranscript,
}

struct ResolvedCursorFullCandidate<'a> {
    trusted_source_root: &'a str,
    normalized_transcript_path: &'a str,
    transcript_identity_id: i64,
    transcript: &'a ValidatedCursorTranscript,
}
```

`stable_snapshot_bytes` 只在本次 hook/helper 调用内用于 parser 与既有 boundary 的 prefix-hash
验证，不得进入 companion JSON、event/blob、spill、日志或错误；持久化的仍只有递归脱敏 IR、
hash/length 和可信 identity locator。helper 返回后立即释放该 buffer。

`ValidatedCursorRecord` 是 versioned enum：`Message` 保留批准 grammar 中已观察并验证的
role、text/tool_use blocks；`TurnEnded` 保留内部 status/error boundary，不进入 raw/prompt。
每个 variant 都携带从完整稳定 JSONL 顺序派生的零基 `transcript_record_ordinal`；该 ordinal
不是 host 提供或合成的 message ID，且在 record 类型、role/usability 过滤前递增，因此 raw
message ordinal 允许空洞但不能重排/压缩。

identity 分两层且不得混用：

- Full companion 层：canonical Stop key + status + stable snapshot bytes hash/length 确定同一
  evidence，并绑定下面解析出的 `transcript_identity_id`；同 key/status/identity/hash/length 重放
  no-op。同 key 的 path 解析成不同 identity 时，即使 bytes hash 相同也冲突；status/hash/length
  不同同样冲突并 zero new writes。
- Raw occurrence 层：使用现有 `SOURCE_ROOT_LOCAL` + 已批准规范可信 path 解析/复用
  `raw_session_identities.id`。Full companion JSON 记录 `transcript_identity_id` 与 source Stop
  关联；snapshot hash 不替代该 ID。同一路径后续 Stop 的 appended snapshot 复用该 ID，既有
  ordinal 重放 no-op、新 ordinal 插入；同 identity+ordinal 的 role/text/content hash、
  event-time provenance 或 source root 与已存行不同则沿用 `RawIdentityConflict`，整个 bundle
  回滚。content hash 对这种 identified row 不是唯一键；不得调用 identity=NULL 的 content-dedup
  路径，也不得增加 `stable_message_id`、列、索引或 migration。

`raw reconcile` 必须使用同一 parser boundary，而不是给 Cursor 再写第二套 grammar。
`raw_reconcile.rs` 对每个 identity 先从现有 ledger 查询绑定该
`transcript_identity_id` 的全部 authoritative Full companions，并读取批准的 host/format
version、可信 source root/normalized path、source Stop event time、snapshot hash/length。
同一 identity 有多个 growing snapshots 是合法 append history：按 byte length 升序后，每个相邻
boundary 必须同 host/format/trusted identity，长度单调增加，并且当前稳定 captured bytes 在每个
approved length 处的 prefix hash 精确等于对应 companion hash。相同 length/different hash、
长度回退、任一旧 prefix 变异或 path/source-root fork 都返回结构化 conflict。链合法时选择最长的
approved companion，按其 boundary 调用 `cursor_transcript.rs` 的完整验证/physical-ordinal
projection；当前 bytes 可在该 boundary 后还有尚未批准的 append suffix，但 reconcile 不消费该
suffix。当前 bytes 短于最长 boundary、任一 boundary hash 不符、metadata 缺失/malformed 或
bindings 不能组成唯一 prefix chain 时 reconciliation 返回结构化错误/parity false，不猜 host、
不退化为 unsupported-record exclusion。不得调用
`raw_transcript::classify_transcript_line`。非 Cursor identity 继续使用既有 classifier；整个
reconcile 路径保持 read-only、aggregate-only。

IR event 是现有 capture ledger 证据，不是新 schema。它必须从普通 prompt event formatter、tool
classification、candidate extraction 和 recursive task coalescing 中排除，只能由 Cursor transcript
reader 按严格 versioned schema 加载。

### 4. production bundle transaction 与 full-first 仲裁（P-004、P-006、P-008、P-013）

现有 `record_captured_event*` 只保证 event/task savepoint，`record_capture_drop` 与
`record_raw_ingest_failure` 仍是独立写入；因此实现必须新增
`src/session_rollup/cursor_snapshot.rs`，不能把现状描述成已有整包原子性：

```rust
pub(crate) fn commit_cursor_snapshot_bundle(
    conn: &Connection,
    key: &CursorSnapshotSemanticKey,
    candidate: CursorSnapshotCandidate<'_>,
) -> Result<CursorSnapshotSelection>;

struct CursorSnapshotSemanticKey {
    host_id: i64,
    project_id: i64,
    session_row_id: i64,
    session_id: String,
    source_stop_event_key: String,
    canonical_stop_key: String,
}

enum CursorSnapshotCandidate<'a> {
    Validated(&'a ValidatedCursorSnapshotInput<'a>),
    InterruptedDegraded(&'a CursorInterruptedDiagnostic),
}

enum CursorSnapshotSelection {
    Full { event_row_id: i64 },
    Degraded { event_row_id: i64 },
}
```

`commit_cursor_snapshot_bundle` 使用 `Transaction::new_unchecked(conn,
TransactionBehavior::Immediate)`，并在同一 serialized writer transaction 中完成：

1. 按 `(host_id, project_id, session_row_id, session_id, source_stop_event_key,
   canonical_stop_key)` 重查 source Stop、full companion、degraded companion和相关
   diagnostics；`source_stop_event_key` 必须精确等于 source Stop 的 stable
   `captured_events.event_id`，candidate 内 `source_stop_event_id` row id 也必须指向同一行。
   Stop不存在/不唯一或candidate绑定错误必须回滚并报错。
2. 对 `Validated` input，先在该 transaction 内从其可信
   `(trusted_source_root, normalized_transcript_path)` 通过现有 session-identity path
   resolve/upsert `transcript_identity_id`，再构造 `ResolvedCursorFullCandidate`，并查询该
   identity 跨 Stop semantic keys 的全部既有 Full boundaries。后续仲裁、occurrence、
   companion 和 task primitive 只接收该已解析类型；若 locator 与 captured
   descriptor/boundary 不一致则整包回滚，不能从 raw Stop payload 补算 identity。
3. 执行固定 companion 优先级 `full > degraded`，不按 row id、created time 或最后写入选择：
   - full 已存在：同一 Stop key/status/identity/hash/length 的 Full replay为no-op。不同 Stop
     对同一 identity 提供 growing snapshot 时不是 replay conflict；新 candidate 的 stable bytes
     必须在每个既有 approved byte-length boundary 上产生相同 prefix hash，且 length 不缩短，
     才可扩展唯一 append chain。相同 length/different hash、截断、旧 prefix 变异、path/source-root
     fork 或其他稳定 evidence 冲突都使整包zero-write。InterruptedDegraded 返回现有 Full，
     绝不插degraded/drop。
   - 尚无full、已有degraded、Full后到：写full companion；把相关 `capture_drop_events` 用现有
     `recovered_event_id` 链到full event；选择Full。
   - 尚无任何companion、InterruptedDegraded到达：条件写stable degraded companion及该reason
     要求的 `capture_drop_events`。已有同一degraded replay不得重复诊断。
4. `ResolvedCursorFullCandidate` 按 IR 中物理 ordinal 逐条调用新增的 Cursor exact-content
   occurrence primitive。该 primitive 可由 `raw_occurrence.rs` 的共享 SQL/refactor 实现，但
   必须保留 leading/trailing whitespace 与完整 UTF-8 text bytes，禁止调用当前会在 hash/存储前
   `trim()` 的 `insert_transcript_occurrence` 入口。相同 occurrence no-op；任一
   `RawIdentityConflict` 或 insert failure 使 identity/raw/event/blob/diagnostic/task 整包回滚。
   只有全部 role+message occurrences 成功后才可提交 full companion。
5. 对本次 authoritative companion 调用/refactor现有transaction-aware capture/task primitive，
   advance/coalesce SessionRollup high watermark。所有 identity/raw occurrence、blob/event、
   drop与task写入必须处于该immediate transaction；内部 savepoint只能嵌套，不能
   提前commit。
6. 最后commit并返回 authoritative selection。任一步失败全部rollback；不得出现companion无
   diagnostics、diagnostics无companion或companion已提交但task watermark未推进。

所有 loader 使用同文件中的 `load_authoritative_cursor_snapshot`，以显式 full-first query选择同一
semantic key：有full就完全忽略degraded内容，只把degraded/drop呈现为历史诊断。coalesced
task range 先按 canonical Stop key/status 生成独立 work plan，禁止混合状态共享 prompt/LLM。
每个 work item 可在内存中完成 prompt 与 LLM，但 worker 不写 raw，且不得提前写 summary 或
side effects。

Terminal 读取面另统一调用 `load_authoritative_cursor_outcome`，其 SQL/typed reducer 对同一
semantic key 固定按 `full > degraded > blank` 排序，禁止使用 latest row。full、degraded 和
blank 的重放顺序都不得改变这一语义：blank 后迟到的 approved payload evidence 可以原子写
degraded outcome 并成为当前态；degraded/blank 在 full 后到或 full 已存在时只能保留为历史。
反向到达的 blank 不得覆盖 degraded，反向到达的 degraded/blank 不得覆盖 full，也不得重复
summary、diagnostic 或 memory side effects。late-evidence test 必须覆盖
`blank -> degraded`、`degraded -> blank`、`blank -> degraded -> full` 及三种 outcome 的 replay。

`src/session_rollup/persist.rs` 必须开启 immediate publish transaction，先复用同一
`select_authoritative_cursor_snapshot` 重查 authority，再在该 writer lock 内提交 authoritative
summary、side effects 与 terminal outcome。该路径不得调用 raw insert/drain primitive。若本轮
按degraded准备但此时full已存在，整项zero-write并返回retry-full。degraded/blank 路径禁止
candidate/workstream/follow-up side effects；
其 summary 只有在 authoritative outcome 仍指向它时可被 session/context/API 读取。若degraded outcome
先完成、full后来提交，则full task/range产生full outcome与唯一记忆侧side effects，统一 selector
隐藏旧degraded summary，旧 outcome/diagnostics 仅保留审计。由此degraded不能在任何提交顺序中覆盖或
与full并列成为可见记忆。

### 5. transcript failure 与 payload-only/blank（P-004、P-008、P-009）

缺失/null/blank/path/reader/parser 失败时：

1. 保留已写的 Stop 和此前 GH-823-approved `postToolUse`、获准的
   `postToolUseFailure` 与 after-only `afterMCPExecution` ordinary capture events；
   `beforeMCPExecution` 是 pre-result event且不注册为 capture owner，`afterFileEdit` 未被
   PR #914 观察或 GH-823 批准，不得作为来源。
2. 由 `commit_cursor_snapshot_bundle(InterruptedDegraded)` 写 stable
   `capture_drop_events.reason = cursor_transcript_<reason>`；`detail` 只含 canonical Stop
   locator、安全计数和 `sha256(canonical_path)[0..16]`，不含完整路径。Cursor 的 path/
   reader/parser degradation 不写 `raw_ingest_failures`：该表没有 recovery link，而这些失败已由
   可链接的 `capture_drop_events` 完整表达。Full raw occurrence insert 的任何错误回滚
   identity/raw/companion/drop/task 整个 bundle并进入现有重试，也不留下 partial
   `raw_ingest_failures` row。
3. 同一bundle写内部 `cursor_transcript_degraded` event，内容含 semantic key、source Stop、
   status/loop key、reason 和可用 evidence IDs，并推进 SessionRollup task high watermark。
4. worker 仅从 exact range 中已脱敏、受限的普通 capture events构造 payload-only evidence。内部
   snapshot/outcome events不作为普通 evidence。
5. 有 usable evidence 时 prompt 明示 degraded reason/source IDs 并调用 AI；没有时不调用 AI、
   不插 session summary/topic segment，只准备 blank terminal outcome。

永久失败不会后来重读原路径升级。若完整 IR 已入 ledger 但 DB/AI/lease 等 worker 步骤瞬态失败，
现有 extraction-task retry 始终复用同一 IR，并可最终产生 full。

Stop/task 原子提交后，hook 准备 companion 与 worker claim 存在受控竞态：

1. worker 加载 task range 时必须按 canonical Stop key 重新查询当前 ledger，不能只相信 claim 瞬间
   的旧结果。若 IR/degraded companion 已存在但 claimed high watermark 尚未包含它，使用现有
   `Waiting` transition 回到 pending；companion 的 coalesce 已推进数据库 high watermark。
2. 若 companion 尚不存在且从 Stop `created_at_epoch` 计算仍在人工批准、测试固定的
   `CURSOR_IR_PREPARATION_GRACE_SECS` 内，使用同一 `Waiting` path，不增加 attempt、不写 outcome。
   该常量是实现常量而非新数据库字段；数值由 #822 Stop-time 延迟证据纳入 GH-825 exact-head批准。
3. 若期限已过，worker 调用
   `commit_cursor_snapshot_bundle(InterruptedDegraded(snapshot_prepare_interrupted))`。absence check
   不是写入依据；helper取得immediate writer lock后必须重新查询。若hook已在两者之间提交full，
   返回Full且零degraded/diagnostic写入；否则原子提交degraded bundle。
4. `Waiting` 保持在bundle transaction之外：只有helper commit成功并且claimed watermark未覆盖返回
   companion时才调用。helper失败时不Waiting，错误进入现有defer/backoff；若helper已commit但
   Waiting状态更新失败，bundle仍durable，processing lease到期后由现有lease recovery重新排队。
   两条路径都不得丢diagnostics或把partial bundle当成功。
5. 同一Stop hook若已在安全reader中持有完整验证后的不可变
   `ValidatedCursorSnapshotInput`，即使worker先提交了 interrupted degraded，hook仍可调用
   helper；helper在 transaction 内解析identity、构造resolved Full并提交。这不是worker重开或
   读取后来变化的原路径。

必须有两条可控barrier交错测试：

- worker absence check → hook Full transaction commit → worker InterruptedDegraded transaction：后者
  重查到full，degraded/drop均零新增，loader/outcome为full。
- worker InterruptedDegraded transaction commit → hook Full transaction commit：后者写full、链接
  capture drop并推进watermark；loader/outcome只选full，旧degraded/drop只作诊断。

对bundle transaction在semantic recheck后、blob后、captured event后、capture drop后、
task coalesce后、commit前和commit后/Waiting前逐点crash injection。commit前各点重启
后必须看到整包零写并由原task重试；commit后必须看到完整bundle，Waiting失败则lease recovery
重试。测试同时覆盖上述两种提交顺序，不能以概率并发测试替代。

### 6. 共用 IR 与现有 raw archive（P-006、P-007、P-013）

新增 `src/session_rollup/cursor_transcript.rs` 作为唯一 Cursor IR validator/projection：

- raw projection 把每条 usable user/assistant message 连同物理 record ordinal 传给
  Cursor exact-content occurrence primitive，且必须提供 bundle 内解析出的
  `transcript_identity_id`；该 primitive 复用共享 SQL 时仍不得经过 legacy `trim()` 入口；
- prompt projection使用现有 8 KiB/message、64 KiB total、128 message budget；
- 两种 projection 都带相同 source Stop/snapshot hash，测试比较输入 ordered message sequence；
- 独立 `turn_ended` 仅用于内部 turn/status consistency validation，不进入 raw/prompt；
  assistant `tool_use` block 被严格验证并留在 IR，但不伪装成 user/assistant text，也不要求
  未观察到的独立 `tool_result`；
- IR/prompt 按 JSONL 文件行序分别保留重复同 role 同文本 record；不得按内容 hash 重排或删除；
- tool payload继续留在 capture ledger，不伪装成 raw user/assistant turn；
- identified `raw_messages` 由 identity+ordinal 区分 occurrence；content hash 只参与稳定字段校验/
  legacy claim。同文不同 ordinal 不得归一；本 issue 不新增 message-id 列。

全量 IR validation 先完成，才由 `commit_cursor_snapshot_bundle` 进入唯一一次 raw insertion。
#862 的 transaction-aware savepoint 必须嵌套在该 immediate bundle 中，保证任何 raw insert
错误不留下部分写入；SessionRollup publish 不再 drain。Cursor 不调用 Claude/Codex JSONL
parser，后两者 dispatch byte-identical。

### 7. 终态、重试与现有 checkpoint（P-009、P-012、P-013）

不再引入 `session_capture_outcomes`、history/drop 新表、`source_message_id` 或 outcome revision。
终态复用现有表：

- 每次完成尝试写 `captured_events.event_type = cursor_capture_outcome`；versioned JSON 包含 source
  Stop stable event key、canonical project/session identity、Stop key、status/loop、
  `full|degraded|blank`、reason、snapshot proof（full 时）、summary range/ID（若有）和 completed
  time。`blank` 必须使用 `reason: "no_usable_evidence"`、空 content，且不得携带 summary ID/range。
  stable outcome event ID 由 canonical `(host_id, project_id, session_row_id, session_id)` +
  source Stop stable event key + canonical Stop key + evidence hash + terminal result 生成；source Stop
  row id用于绑定校验但不单独替代 stable key。
- path/parser failures 只在 `capture_drop_events` 留历史。同一已绑定 IR 的 worker retry最终 full，
  或同一 source Stop 后续 full 时，可用新 outcome event id 写现有 `recovered_event_id`
  表示操作面已恢复；原 source Stop 的 degraded 历史不改写、不删除。
- doctor/session 对每个 semantic key 直接按 `full > degraded > blank` terminal priority 派生
  authority，不按最新 row选择；blank 是无 usable evidence 时的独立 terminal result，不与
  degraded summary 伪合并。blank 后迟到 approved payload 生成 degraded 时升级到 degraded；
  反向到达或重放不能降级当前态。旧blank/degraded/drop仍可审计，但linked recovery不再计作
  当前actionable。
- session-level `capture_health` 使用两阶段 selector，不能跨 Stop 直接取最高 fidelity。第一阶段
  先按 source Stop 的 immutable `(captured_events.created_at_epoch, captured_events.id)` 选择
  当前 Stop；第二阶段才在该 Stop 内按 `full > degraded > blank` 选 current outcome。这里的
  `id` 仅作为同 epoch 的 source-Stop capture-order tie-breaker，不使用 outcome row id；
  replay 必须引用原 source Stop，不能推进该顺序。因此较早 Stop 的 full 不得遮蔽较晚 Stop 的
  degraded/blank。doctor 仍保留并统计所有 Stop 的 outcome/history。

SessionRollup task 已在 Stop capture transaction 中 durable 创建；IR/degraded companion 只推进
其 high watermark。worker 保持当前 retry checkpoint：

1. plan：通过full-first loader加载 exact range，并按 canonical Stop key/status 拆分 work item；
   混合的已批准状态不共享 prompt 或 LLM；未批准的 `error` 不得进入 task range。
2. prepare：逐 work item 验证 Cursor IR 或构造 payload-only/blank，可在内存中完成 bounded prompt/LLM；
   不写 raw、summary 或 side effects。
3. publish：开启 immediate transaction 后先 full-first 重查；authority 变化则整项 zero-write/retry。
   authority 未变时，full 路径只读取 bundle 已提交的 raw/IR evidence，并在同一 transaction 写
   summary、允许的
   citation/candidate/workstream/follow-up side effects 与 outcome；degraded 只写 authoritative
   summary/outcome，blank 只写 outcome，后二者禁止记忆侧 side effects。publish transaction
   对 raw tables 必须 zero writes；raw archive 先于 summary outcome 可见是有意的 capture-evidence
   checkpoint，不表示 rollup/side effects 已完成。
4. terminal：publish transaction 提交后，外层worker仅以 lease owner mark Done；所有当前读取面按
   outcome 引用选择 summary。full 后到会产生新的 full item/outcome，并隐藏旧 degraded summary。

若 publish commit 后、task Done 前崩溃，重试从 stable outcome ID 与已原子提交的
summary/follow-up 状态判定 no-op，并复用 bundle-owned raw，再返回成功。若更早失败，task按现有
backoff重试。terminal outcome event
本身 task kind 为 None，不递归推进 high watermark；后续 Stop range loader忽略所有内部 event。

### 8. doctor 与读取面（P-012）

在现有 `query_system_stats` / doctor checks 上增加 Cursor 聚合：

- 最近窗口 full/degraded/blank 数量和每个 Stop 的当前 terminal outcome；
- 最新 degraded reason/time/locator，连续降级比例；
- unresolved Cursor drop 与 linked recovered history；
- outcome JSON/schema损坏、DB查询失败必须 doctor Fail/Error，不能当零行。

CLI 的现有 `remem raw sessions --json` summary 与 `remem raw messages --json` session envelope，
以及本地 `/api/v1/sessions/{id}`，都通过同一 current-outcome selector 投影
`capture_health`：`fidelity` (`full|degraded|blank`)、批准的 Stop `status`、稳定
`reason_code`（full 时 null）和脱敏的 `stop_key`。存在 authoritative summary 时才显示其内容；
selector 必须先按 immutable source-Stop capture order 选择当前 Stop，再在该 Stop 内使用
`full > degraded > blank`，与 outcome 插入/重放时间无关；blank 内容保持空。较早 full 不得
遮蔽较晚 degraded/blank。
非 Cursor 或无 outcome 的 session 返回 `capture_health: null`，不得制造默认值。
`raw sessions --json` 的 candidate set 必须 union raw-message aggregation 与 authoritative
Cursor outcomes；没有 raw-backed tuple 的 degraded/blank 以 output-only
`source_root: "cursor-outcome"`、source Stop epoch、零 message/role counts 和空 samples出现。
该虚拟 locator 不写 `raw_messages`/identity；`raw messages --json` 只用它返回空 message page
和相同 capture-health envelope。project/time filter、稳定排序、去重及 corrupt-outcome
fail-closed 都需覆盖。`cursor-outcome` 是保留 source-root label：
`ScanRoot::parse` 必须在打开/修改数据库前拒绝用户 `--root cursor-outcome=...`，ingest
入口还要 preflight 拒绝绕过 parser 构造的 required root；default internal roots不受影响。
读取面若发现持久 raw/identity row 使用该 label，必须把它视为 collision 并 non-zero，
不能与虚拟行合并、隐藏或制造 raw message。
`src/context/query.rs` 的 session summary 读取也必须先解析 current outcome 引用的 summary ID/range，
不得直接枚举同 session 的全部 `session_summaries`。API handler、API public contract、context
session tests 与 corrupt-outcome fail-closed cases 都纳入本 packet。日志是补充证据，不是唯一历史。

### 9. GH-824 installed-hook 边界（P-001、P-014）

GH-825 runtime fixture测试直接调用 approved normalized invocation，不依赖安装。GH-824 合入且
runtime capability probe确认 GH-823 Stop wiring + GH-825 reader可用后，才执行真实 `hooks.json`
installed-hook E2E。若 capability缺一项，#824必须不安装/广告 `stop` summarize。

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| P-001 外部门 | SpecRail route + #822 fixture manifest + capability gate | 缺任一 exact-head approval/fixture时 implement route和 runtime capability均 blocked |
| P-002 GH-823 payload唯一合同 | GH-823 outer parser + observe boundary | `cargo test -q observe::hook --lib`；approved exact/one-byte-over/malformed fixtures断言 non-zero、zero writes，合法未知名称原样入账 |
| P-003 normalized Stop入口 | GH-823 Stop validator + GH-825 typed invocation | identity/workspace/status/loop missing/wrong/unknown fixtures全部 zero writes；approved fixture成功 |
| P-004 Stop/task与companion bundle原子性 | Cursor summarize integration + `commit_cursor_snapshot_bundle` | Stop+pending task同事务；bundle的identity/raw/event/blob/drop/coalesce逐点rollback；事务失败整包零写且task可重试；helper是唯一raw writer，publish raw-write spy=0，Cursor raw_ingest_failures写入spy始终为零 |
| P-005 path/size trust | Cursor path reader | trusted-root matrix、exact/one-byte-over approved max、symlink/reparse/FIFO/socket/device/TOCTOU tests |
| P-006 immutable canonical IR | `cursor_transcript.rs` + `cursor_snapshot.rs` + ledger/blob | PR #874 role/message 与 PR #914 role/message+tool_use+turn_ended fixtures 无逐消息ID/timestamp/tool_result仍成功；物理 ordinal 覆盖所有 record，raw ordinal允许空洞，turn_ended不进raw/prompt且不合成字段；安全reader固定source root/normalized path，helper先解析identity再构造Full，candidate/bundle/companion全程携带三元identity；坏中间/坏尾零IR；worker从不重读路径；same Stop/status/identity/hash/length replay no-op，same hash但different path identity也冲突 |
| P-007 shared full IR + identified occurrences | `cursor_snapshot.rs` + `session_identity.rs` + Cursor exact-content occurrence primitive in `raw_occurrence.rs` + `raw_reconcile.rs` + raw/prompt projections | bundle-only raw insertion preserves append/ordinal/whitespace/conflict rules; multiple growing companions validate every approved prefix boundary and longest approved selection，fork/truncation/mutated-prefix fail；publish raw-write spy=0; prompt consumes same hash/sequence; raw reconcile selects the shared Cursor parser from authoritative companion chain instead of Claude/Codex classifier; `cargo test -q memory::raw_archive --lib`; `cargo test -q memory::raw_reconcile --lib` |
| P-008 payload-only/full优先 | bundle arbiter + semantic-priority outcome loader + SessionRollup prompt | `full > degraded > blank` 与到达顺序无关；blank→degraded、degraded→blank、degraded↔full及replay矩阵均收敛到最高可用证据；无full时null/missing/corrupt + usable payload产生grounded degraded summary，source IDs全在range内 |
| P-009 no data=blank | SessionRollup prepare/outcome | transcript/payload空时 AI mock=0、summary/topic=0、blank outcome/doctor可见 |
| P-010 status矩阵 | normalized Stop + status-aware coalesced-range planner + prompt metadata | completed/aborted各覆盖full/payload-only/blank且有证据AI=1、无证据AI=0；若GH-823批准error，则其各分支AI=0、error log、证据不丢、状态未改写，并覆盖completed+error、aborted+error与三状态mixed range按Stop拆分；若未批准，则exact error输入non-zero、zero writes、AI=0且不能进入task |
| P-011 loop幂等 | GH-823-approved canonical Stop key + project/session/source-Stop-bound stable event ID | #822后GH-823 amendment未批准则blocked；批准后new/replay/conflict矩阵与跨项目同session/Stop fixture：重复零新增/零AI，不同project生成不同ID，冲突non-zero/zero writes，新key推进一次high watermark |
| P-012 current/history diagnostics | priority outcome loader + source-Stop selector + capture_drop + doctor + CLI/context/session API projections + `ScanRoot` validation | 每个 Stop 内固定 `full > degraded > blank`；session-level 先按 immutable source Stop `(created_at_epoch,id)` 选当前 Stop再取其 fidelity，覆盖 earlier-full/later-blank、earlier-blank/later-full、same-epoch tie和replay不推进；blank→degraded迟到证据只提升同一 Stop，反向到达/重放不降级；blank reason/content/summary invariants固定；旧Stop/blank/degraded诊断保留但不从 context/CLI/API 暴露为当前 summary；drop recovery链接、损坏JSON/DB读取fail；`ScanRoot::parse`/ingest preflight拒绝 reserved `cursor-outcome` label，持久化collision读取fail，CLI raw sessions union outcome-only rows且 raw messages返回空页；API `capture_health` full/degraded/blank/null contract 通过；Cursor degradation 不新增 raw_ingest_failures；`cargo test -q doctor --lib` |
| P-013 retry/checkpoint一致性 | immediate bundle/publish transactions + Waiting/lease recovery + SessionRollup work planner | absence-check交错、两种commit顺序、bundle raw与其他写点/commit/Waiting fault injection；publish authority recheck发生在任何 summary/side-effect write 前且永不写raw；bundle commit前全回滚，commit后raw/IR不丢；degraded遇full则zero-write retry，且degraded无candidate/workstream/follow-up；`cargo test -q session_rollup --lib` |
| P-014 internal-event隔离/零回归 | range loader/formatter + host dispatch | internal event sentinel不进prompt/candidate/task；Claude/Codex byte snapshots不变；完整`cargo test` |

## 数据流

```text
#823 validated Cursor Stop(status + canonical loop key)
        |
        +-- stable session_stop + pending SessionRollup task
        |          (one existing capture savepoint)
        |
        +-- #822-approved secure full read + parse
        |       |-- success --> Validated snapshot input ----------.
        |       '-- failure --> Degraded candidate ---------.       |
        |
        '-- hook crash/early worker claim                    |       |
                |-- within grace --> existing Waiting        |       |
                '-- after grace --> Interrupted candidate ---+-------+
                                                            v
                        commit_cursor_snapshot_bundle
                        IMMEDIATE: recheck -> resolve identity ->
                        Full construction + prefix-chain/full-first ->
                        raw + companion + diagnostics + task watermark
                        v
status-aware per-Stop work plan (mixed statuses never share an LLM)
                        v
full-first SessionRollup prepare in memory (never reopen original path)
        |-- full IR --> read bundle-owned raw + bounded prompt projection
        |-- degraded --> bounded ledger payload evidence
        '-- no evidence --> blank, no LLM/summary
                        v
IMMEDIATE publish: full-first recheck before any DB write
        |-- full --> summary + allowed side effects + outcome (zero raw writes)
        |-- degraded --> summary + outcome, no memory side effects
        '-- blank --> outcome only
                        v
worker lease-owned Done
                        v
doctor/session select current full > degraded > blank; historical outcomes/drops remain auditable
```

## 备选方案

- 新建 outcome/history/drop 表或 raw message ID 列：拒绝，违反 GH-825 issue 的 no-new-schema。
- 把 fidelity 塞进严格的 `session_summaries.transcript_evidence_json`：拒绝，blank没有 summary，且
  该 JSON已有 deny-unknown evidence schema。
- worker稍后读取原始路径：拒绝，不能证明 Stop 时字节，也会产生TOCTOU。
- raw/prompt分别解析 Cursor：拒绝，会产生格式漂移和部分成功。
- malformed/over-limit tool payload写 hash metadata后继续：拒绝，违反 GH-823 zero-write合同。
- Stop先提交、IR/degraded之后才创建task：拒绝，hook crash会留下不可发现的孤儿Stop。
- worker先absence check、再用多个autocommit写degraded/drop/task：拒绝，hook可在中间提交full，且
  crash会留下partial bundle。
- `aborted` 一律抑制 LLM：拒绝，会丢失中止前的高价值根因；有证据时按GH-823 canonical运行
  grounded总结。`error` 必须遵循GH-823 canonical跳过LLM并保留捕获证据。
- degraded后由worker重读后来路径升级：拒绝；只有同一Stop hook已经持有的不可变已验证
  snapshot input（由helper解析identity后构造Full），或已绑定IR的瞬态worker retry，才可最终full。

## 风险

- Security: host路径和嵌套payload不可信。外层限额、trusted root、no-follow、descriptor检查、
  全量验证、递归脱敏和locator-only诊断必须同时存在。
- Compatibility: GH-823只实现部分host白名单或#824提前安装Stop会使捕获失败；capability test和
  分阶段依赖阻止宣称可用。
- Performance: transcript最大值由#822实测后人工冻结；hook需完整读取/解析并将IR写入blob，PoC
  必须证明可接受延迟。不能为降低延迟改成partial full。
- Storage: 规范IR复用event_blobs会增加ledger体积；沿用现有retention/classification并在#822
  尺寸证据后评估，不新增Cursor专用schema。
- Concurrency: Stop task可能早于companion被claim；必须在新增immediate production helper内重查并
  full-first仲裁，不能以事务外absence check决定写入。Waiting失败靠lease recovery，bundle失败靠
  原task defer/backoff；两者都必须保留可见诊断与重试能力。
- Data quality: v071 已提供 transcript occurrence identity；Cursor 必须以 path identity + 物理
  record ordinal 保留同文重复项，不能退回 identity=NULL content dedup。path 被替换导致既有
  ordinal 稳定字段变化时 fail closed/rollback，而不是覆盖或创建伪 message ID。
- Ordering: PR #914 只证明 completed/aborted 的数字 `loop_count: 0`；非零、缺失/null和
  error 仍不能解释。GH-823 packet amendment 必须在 exact head 人工批准 canonical key；
  GH-825只消费批准结果。

## 测试计划

- [ ] Spec: write_spec route、workflow packet、planned-changes manifest、`git diff --check`。
- [ ] Real fixture: 固定 PR #874 Cursor 3.6.31 `{role,message}` 与 PR #914 Cursor
  3.12.17 `{role,message}`/`tool_use`/`turn_ended` exact-head 正例、来源和 secret scan；
  断言没有逐消息ID/timestamp/独立tool_result仍有效，物理ordinal覆盖所有record且不合成字段；
  不得含真实home path、email或token。
- [ ] Unit: parser full-validation、approved max ±1、path矩阵、canonical status/loop矩阵、IR strict
  schema、Full构造前trusted source root/normalized path/identity解析、物理ordinal/文件行序、
  同文重复 occurrence、同路径多Full append prefix-chain/最长approved boundary replay、
  fork/truncation/mutated-prefix拒绝、既有 ordinal
  stable-field conflict、identity=NULL 路径禁用、internal-event filtering、stable snapshot event
  IDs、cross-project same-session/Stop key distinct IDs、Stop/task rollback、
  same Stop/status/identity/hash/length replay no-op、same hash/different
  path identity与changed hash conflict、bundle full-first、publish raw-write spy=0、
  raw reconcile Cursor parser chain routing/missing-conflicting metadata、
  每Stop内`full > degraded > blank`所有到达与replay顺序、跨Stop
  earlier-full/later-blank与earlier-blank/later-full、same-epoch source-Stop id tie、
  replay不推进session current Stop、reserved `cursor-outcome` parse/preflight/persisted-collision、
  Waiting无attempt增量。
- [ ] Integration: Stop/task原子落账；两条barrier控制提交顺序；bundle semantic recheck/blob/event/
  raw/drop/coalesce/commit/Waiting逐点crash；Cursor raw-ingest-failure spy和publish raw-write
  spy始终为零；
  preparation grace前后；payload-only、blank no-AI、
  completed/aborted grounded；若GH-823批准error则覆盖error no-AI/mixed range，否则覆盖outer
  rejection、zero writes、AI=0；shared IR、raw rollback、outcome/worker checkpoints。
- [ ] Schema: migration/schema snapshot before/after byte-equivalent；无新migration、表、列、索引。
- [ ] Regression: `cargo fmt --check`, `cargo check`, focused tests, `cargo test`；不降低Claude/Codex
  断言。
- [ ] Runtime fixture: GH-823 foundation + GH-825 reader后，以normalized invocation跑full/null/
  missing/corrupt/oversized/status-loop场景，不依赖#824。
- [ ] Installed E2E: 仅在#824合入并通过capability probe后，以隔离HOME/合成workspace验证真实
  Cursor Stop hook；不读取用户真实会话。

## 回滚方案

- GH-825 reader/outcome和GH-823 Stop wiring均在canonical Cursor capability gate后；关闭capability
  即停止新Cursor Stop处理，不改变Claude/Codex。
- 没有migration或down migration。回滚二进制会忽略开放event_type中的Cursor IR/outcome rows，
  但保留ledger/drop/raw历史；重新升级后按stable event ID安全重试。
- 若Cursor格式漂移，关闭对应format capability并显式degraded；不得回退为格式嗅探或Claude/
  Codex parser。
- installed环境先用当前版本#824 uninstall/repair移除managed Stop hook，再降级binary。
