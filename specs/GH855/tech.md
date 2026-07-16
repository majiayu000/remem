# Tech Spec

## Linked Issue

GH-855

## Product Spec

[`product.md`](product.md)

## Evidence and Approval Gate

- 代码与 schema 基线：`origin/main@37f391ca704ae11ec811c330cdccdbc1527ccd49`。
- GH-672 已实现 candidate/direct-save/pack 的 pattern quarantine、source trust、memory/lesson
  injection rescan、governance acknowledgement 与 doctor 计数；本设计扩展现有契约，不重做它。
- `docs/research/agent-memory-optimization-research-2026-07.md` 在上述基线不存在。issue 对该文件的
  引用不能作为已验证证据，任何百分比均不进入本设计。
- GH-852 在当前 main 没有已合并 spec/implementation，且其目标是新增 host-native memory 来源。
  依赖方向为 GH-852 的新来源必须复用 GH-855 的 verdict；GH-855 不依赖 GH-852。
- route gate 允许 `write_spec`，但本文件明确保留 `spec_approval`：缺失报告须由不可变 revision，
  或 maintainer 记录批准的等价一手证据补齐。

## Codebase Context

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| Pattern 与 trust | `src/memory/poisoning.rs:4`, `src/memory/poisoning.rs:50`, `src/memory/poisoning.rs:109` | v1 scanner 覆盖 override、execution、concealment、authority、opaque payload；summary 无 evidence 时降为 `external_content` | 必须复用并扩成多 surface verdict，不能建立第二套规则 |
| Candidate persistence | `src/memory_candidate.rs:477`, `src/memory_candidate.rs:479`, `src/memory_candidate.rs:485` | trust 从 evidence 推导，但 quarantine 只扫描 `candidate.text` | 原始 source 命中可被模型转述/省略洗白 |
| Observation output | `src/observation_extract/response.rs:41`, `src/observation_extract.rs:335`, `src/observation_extract.rs:399` | 严格解析 JSON/字段后持久化模型文本；生成字段未统一重做 redaction/pattern verdict | capture -> observation -> candidate 的生成边界 |
| Rollup prompt | `src/session_rollup/prompt.rs:44`, `src/session_rollup/prompt.rs:90` | prompt 告知模型 transcript 不可信，event content 在入 prompt 前 redaction/bound | prompt 指令不是持久化或注入安全边界 |
| Rollup persistence | `src/session_rollup/mod.rs:90`, `src/session_rollup/mod.rs:115`, `src/session_rollup/persist.rs:16` | 模型输出解析后直接持久化，再执行 archive/candidate/topic/native side effects | 需要在任何衍生 side effect 前形成 durable verdict |
| Context summary query | `src/context/query.rs:508`, `src/context/query.rs:591`, `src/context/types.rs:22` | recent summaries 只按内容/时间查询；brief 没有 summary id 或 poisoning metadata | legacy/current summary 无法被统一复扫和审计 |
| Retrieval steering | `src/context/query.rs:47`, `src/context/query.rs:201`, `src/context/implicit_query.rs:40`, `src/context/hybrid_context.rs:27` | summary 在 poisoning eligibility 前已参与 self-diagnostic、cluster/session dedup、implicit query 与 hybrid memory retrieval | render 前再 drop 已太晚，poisoned row 已能改变其它 memory 的选择与排序 |
| Injection defense | `src/context/poisoning.rs:16`, `src/context/poisoning.rs:43` | 只 retain/复扫 memories 与 lessons；失败时 drop memory | 必须把 summaries 接入同一 fail-closed 入口 |
| Session rendering | `src/context/sections/sessions.rs:18`, `src/context/sections/sessions.rs:45` | request/completed 经 inline formatting 后直接进入 `## Sessions` | 最终 model-visible sink |
| 其它 summary sink | `src/context/claude_memory/runtime.rs:143`, `src/observation_extract.rs:275`, `src/summarize/summary_job/persist.rs:10`, `src/user_context/extraction/source.rs:407`, `src/user_context/recall/sources.rs:234` | 多个 native-memory、后续 LLM prompt 和 user-context 路径直接读取 summary 字段，没有 poisoning eligibility gate | 只修 SessionStart 会留下等价旁路 |
| Legacy summary writer | `src/db/summarize/session/finalize.rs:4`, `src/db/summarize/session/finalize.rs:28` | 旧 summarize 路径替换写入 `session_summaries`，未 redaction/scan，也没有 captured-event evidence metadata | 所有 summary writer 都必须产生可验证 verdict，不能靠表默认值放行 |
| Git/MCP summary trace | `src/git_trace.rs:514`, `src/git_trace.rs:583`, `src/git_trace.rs:628`, `src/mcp/server/commit_tools.rs:9` | commit lookup 两处 SQL join summary 正文并直接序列化给 MCP；`summary_from_row` 没有 summary ID、project 或 poisoning metadata | 这是独立 model-visible sink，必须与 context reader 使用同一 gate 和 identity binding |
| Governance | `src/memory/governance.rs:5`, `src/memory/governance.rs:45`, `src/memory/governance.rs:133` | `AcknowledgePattern` 只接受 memory IDs，要求 reason/confirm 并写审计 | 通过 target kind 复用动作、授权与 audit |
| Doctor | `src/doctor/memory_poisoning.rs:6`, `src/doctor/memory_poisoning.rs:15`, `src/doctor/memory_poisoning.rs:25` | 只统计 candidate 与 memory injection drops；数据库错误为 `Warn` | poisoning 状态缺失不能伪装为健康 |
| Status | `src/db/query/stats.rs:13`, `src/db/query/stats.rs:96`, `src/cli/actions/query/status/types.rs:5`, `src/api/handlers/status.rs:69` | shared stats 无 poisoning section；API 可在 refresh 失败时显式返回有界 stale cache | 新计数应一次查询、CLI/API 同步、失败可见 |
| Public adversarial eval | `src/eval/memory_bench/tests.rs:58`, `src/eval/memory_bench/diagnostics.rs:9`, `eval/public/memory/suites/adversarial-policy/suite.json` | 现有 15 cases 是 retention-policy fixture，不含 instruction/authority/opaque 类别，也不经过 capture/extraction | 需扩类别且另加生产 pipeline E2E，不能把静态 policy fixture 冒充 capture 证明 |
| Current contract | `docs/specs/memory-poisoning-defense/PRODUCT.md`, `docs/specs/memory-poisoning-defense/TECH.md`, `docs/specs/README.md` | GH-672 是当前 poisoning contract | implementation 必须更新 current truth 与索引状态 |
| Schema/migrations | `src/migrate/types.rs:306`, `src/migrate/types.rs:351`, `src/migrations/v061_memory_poisoning_injection_drops.sql` | 当前 main 最新 v069；drop table 仅以 `memory_id` 为目标 | summary metadata 应落在 summary 行，避免复制 memory-only drop schema |

## 设计方案

### 1. 统一 redaction 与多 surface verdict（B-001～B-005、B-014）

在 `src/memory/poisoning.rs` 增加共享的、无网络/无 LLM 的 verdict API，而不是让 observation、
candidate、rollup 与 context 各自维护 deny list。概念模型：

```rust
struct PoisoningSurface<'a> {
    stage: PoisoningStage,            // source | generated
    artifact_field: &'static str,
    evidence_event_id: Option<i64>,
    redacted_text: &'a str,
}

struct PoisoningVerdict {
    source_trust_class: SourceTrustClass,
    primary: Option<InstructionPatternMatch>,
    matched_stage: Option<PoisoningStage>,
    evidence_event_id: Option<i64>,
    artifact_field: Option<&'static str>,
}
```

- 调用方先用现有 `redact_sensitive_text` 对每个 source/generated text field 生成规范扫描文本；持久化
  的 LLM 产物也使用该 redacted value，不能扫描脱敏副本却保存原副本。
- source surface 按 event ID 升序，generated surface 按固定 field table 顺序扫描；pattern table
  继续定义 primary match 优先级。因此相同输入、pattern version 与 schema 必须产生同一 verdict。
- verdict 只保存 stage、field 名、event ID、pattern/version 和 trust，不保存命中 excerpt。日志调用
  统一 safe formatter，禁止直接格式化 source/generated text。
- source event 不因 match 改写或删除。任何 event load、redaction、scan 或 metadata persistence 错误
  返回 `Err`，由 extraction transaction fail closed；不得把 missing evidence 映射为 safe。
- `INSTRUCTION_PATTERN_SET_VERSION` 仅在 pattern 行为变化时递增；ack 精确绑定 ID/version。

### 2. Observation/candidate 阻断 source laundering（B-001～B-005）

`observation_extract` 保留现有严格 JSON parsing，但在 dedup 和数据库写入前：

1. 对 title/subtitle/narrative/facts/concepts/files 字段逐项重新 redaction；redaction 后 required 内容
   全空时返回 extraction error。
2. 加载 range 中全部 referenced captured events，校验 event ID/项目/session/range 一致，并计算一次
   source verdict；事件缺失或 ownership 不一致视为错误，不降级。
3. observation 继续作为已脱敏的中间证据持久化；它本身不是 active memory/context sink。
4. 创建每个 memory candidate 时，把 candidate text、observation 生成字段和其完整 event evidence
   一并交给共享 verdict。source 或 generated 任一 match 都写入现有
   `review_status='quarantined'`、`quarantine_pattern_*`，并把闭集 block reason 扩展为
   `quarantined_source_instruction_pattern` / `quarantined_generated_instruction_pattern`。
5. auto-promote、candidate approval、pack/direct-save 与 injection 的 GH-672 gate 保持；candidate
   acknowledgement 只确认该 candidate 的精确 match，不确认 source range 的其他衍生 artifact。

不为 observations 新增“active poison”状态：所有会成为 active memory 的 observation 路径必须经
candidate verdict；生产测试同时搜索 observation 的其他直接注入/推广调用点，发现绕过则在本 issue
内接入统一 verdict，不允许以 manifest 未列为理由放行。

### 3. Session summary schema 与 durable quarantine（B-006、B-007、B-012、B-015、B-016、B-021）

在本基线使用下一个空闲 migration `v070_session_summary_poisoning.sql`；implementation 开始前必须
重新检查 main，若 v070 已被占用则只顺延编号并同步 manifest/测试，不复用已发布 version。为
`session_summaries` 增加：

- `poisoning_status TEXT NOT NULL DEFAULT 'legacy_unscanned'`，闭集
  `legacy_unscanned|safe|quarantined|acknowledged`；
- `source_trust_class TEXT NOT NULL DEFAULT 'external_content'`；
- `quarantine_stage`, `quarantine_field`, `quarantine_event_id`,
  `quarantine_pattern_id`, `quarantine_pattern_version`；
- `acknowledged_pattern_id`, `acknowledged_pattern_version`, `acknowledged_at_epoch`；
- `poisoning_block_count INTEGER NOT NULL DEFAULT 0`, `poisoning_last_blocked_at_epoch`；
- `poisoning_side_effects_released_at_epoch`，用于 approval 后 exactly-once release。

添加 `CHECK`/组合 invariant：safe 不得带 quarantine/ack；quarantined 必须有 stage、pattern/version；
acknowledged 必须同时有 quarantine 与精确 ack；block count 非负。schema drift 检查验证字段、索引和
约束，迁移测试覆盖 v069 database、空数据库、重复 dry-run/real migration 与 rollback-on-error。

`legacy_unscanned` 是一次性 migration provenance，不是 writer fallback。migration 先用新增列的
`DEFAULT 'legacy_unscanned'` 标记升级前 rows，随后在同一 migration 创建数据库 invariant：任何
`INSERT` 的最终 `poisoning_status='legacy_unscanned'` 都 `RAISE(ABORT)`，任何从其它状态回退到
`legacy_unscanned` 的 `UPDATE` 也 `RAISE(ABORT)`。因此迁移后的旧 binary 若省略新列会显式失败，不能
借 default 制造新的 legacy row；runtime insert 必须显式写 `safe` 或 `quarantined`。schema drift 与
upgrade tests 必须验证 trigger/等价 constraint 的 SQL 和行为，而不只验证列存在。

`persist_session_rollup` 的新事务顺序：

1. 在调用 summarizer 前加载完整 source range/transcript evidence，验证 event ID、project、session、
   range 与 ownership，重新 redaction 全部 source surfaces；缺失或不一致立即返回 error，不调用模型；
2. parse 后重新 redaction summary/structured/topic fields；
3. 计算 source + generated verdict；
4. 用现有 range uniqueness 写一个 durable summary：无 match 为 safe，有 match 为 quarantined；
5. quarantined 时不写 topic segments、不运行 candidate/native-memory side effects，并记录不含内容的
   error/audit metadata；raw archive 的独立成功/失败 checkpoint 仍按现有规则保存；
6. safe 时才执行现有 checkpointed side effects。

已存在 durable summary 的 worker retry 不再次调用 summarizer；它以 summary ID + project + range 和
当前 generation 重新加载 poisoning state。safe 才补跑 checkpoint，quarantined 保持阻断，精确
acknowledged 通过原子 compare-and-set 至多一次写 `poisoning_side_effects_released_at_epoch` 后补跑。
side effect 内部现有 checkpoints 继续负责各步骤幂等；任何失败保留未完成 checkpoint，下一次重试
继续。允许的单调转换是 migration row `legacy_unscanned -> quarantined`、runtime/current
`safe -> quarantined`、同 generation `quarantined -> acknowledged`；新 pattern generation 可把旧
acknowledged row 转为新的 quarantined generation，但任何 retry/陈旧 writer 都不得回退为
`legacy_unscanned`、覆盖更新 generation 或把 matched row 写回 safe。

旧 `finalize_summarize` writer 迁移到同一 summary persistence helper，并必须提供与 rollup 相同的完整
source evidence snapshot。生成字段先 redaction/scan，source + generated verdict 成功后才允许写
`safe`/`quarantined` row；若该 job 无法取得完整 evidence，则返回 error、零 summary/topic/candidate/
native-memory derived write，并保留规范 source/capture 供原 job retry。禁止持久化 generated-only
`legacy_unscanned` row 后再由 read path 洗白。implementation 必须 closure-audit 所有
`INSERT|UPDATE|REPLACE session_summaries` writer，并用故障注入证明无隐式 default fallback。

### 4. Summary eligibility、retrieval 与 model-visible sink fail-closed（B-008、B-010、B-012、B-015、B-022、B-023）

建立一个共享的 summary eligibility loader，内部 raw row 带 summary ID、project/owner、session identity、
全部 model-visible fields 与 poisoning/ack metadata；只有验证后的 `EligibleSessionSummary` 才能把正文
交给调用方。禁止 reader 直接用裸 SQL 绕过它。`query_recent_summaries` 的 batch SQL 只能用 project/
owner、时间、稳定 ID 取 raw rows 和排序，不能在 gate 前用 request/completed 等正文做 `CASE`、过滤或
`ORDER BY`；每个 row decode 后的第一步是 project/ID/state/schema/scanner eligibility：

- 对 safe/acknowledged/current 及 legacy rows 都复扫实际 render haystack，而非只信数据库 status。
- match 且 ack 不精确时排除 summary，并用事务/CAS 更新 quarantine metadata、block count 与 last
  blocked time；记录失败时仍排除并返回结构化 `session_poisoning` 错误。
- legacy row 无 match 可本次注入，但保持 `legacy_unscanned`，使 pattern version 升级后仍会复扫；
  不在读路径自动授予永久 safe/ack。
- scanner、row decode、state query 或审计 update 失败时，在正文参与任何 comparison/selection 前排除
  该 row；context 继续处理已验证安全 rows，并把错误加入 `LoadedContext.errors` 和现有 hook error
  surface。全局 schema/query 失败返回零 summary + error，不能用旧 cache/空错误伪装成功。
- ack 仅在 `poisoning_status='acknowledged'`、ID/version 与本次 match 完全相同时放行；新 pattern
  或版本变化重新阻断。

只有 eligible rows 才进入 `is_session_summary_self_diagnostic`、display-request fallback、cluster/session
dedup、stale fallback 与 limit；随后才可进入 `build_implicit_context_query`、hybrid retrieval、abstention、
fact-label query、memory dedup/ranking 或 render。retrieval-steering regression 使用一个含唯一攻击 token、
本会召回特定 memory 的 poisoned/error summary，并断言其结果与物理删除该 row 的 baseline 在 implicit
query、hybrid channel/rank、abstention、selected memory IDs 和 summary cluster selection 上完全相同；
同时断言 error 可见。安全 summary 对照仍应提供既有 retrieval signal。

同一 loader/gate 同时替换 `context::claude_memory` native sync、observation extraction 的 prior-summary
context、legacy summarizer 的 existing-summary context、user-context extraction/recall/summary activity
读取；context data-version hint 纳入 status/pattern version，使 quarantine/ack 变化必然触发重新渲染。
timeline/count-only 查询可继续只读安全计数；任何输出 summary 正文或把正文交给 LLM 的新调用点都
必须使用 eligibility gate。implementation 以 `rg 'session_summaries'` closure audit 证明没有裸读 sink。

`git_trace` 的 `linked_sessions_for_commit` 和 `query_session_commits` 不再 join/select summary 正文：SQL
只取得稳定 `summary_id`，并同时保留 commit project、link session/memory-session identity。随后逐项调用
`load_eligible_summary_by_id(conn, expected_project, summary_id, expected_session_identity)`；loader 必须
确认 row ID、project/owner 与 link identity 均匹配才返回正文。合法“没有关联 summary ID”仍返回
`summary: null`；存在 summary ID 但未确认命中、身份不匹配或 scanner/schema/audit 失败则整个
`lookup_commit`/`commits_for_session` 返回 typed error，不能伪装成 null/空正文。MCP
`commit_tools.rs` 将该 error 映射为明确 tool error 并保留安全 metadata，不返回部分含正文 JSON。

不扩展 `memory_poisoning_injection_drops(memory_id NOT NULL)` 为多态表；summary block 的计数和最近
时间保存在 summary 行，避免破坏现有 memory drop 历史。doctor/status 聚合两种目标。

### 5. 复用 governance target（B-009～B-012、B-014）

给现有 governance 请求增加闭集 `GovernanceTargetKind::{Memory,SessionSummary}`：

- CLI `remem govern --target-kind memory|session-summary` 与 MCP `govern_memory.target_kind` 为向后兼容
  可选字段，缺失默认 `memory`；selector 仅支持 memory，summary acknowledgement 必须显式 ID，
  防止模糊查询批量放行。
- `session-summary` 只允许 `acknowledge-pattern`；delete/reject/stale 与其它组合在 dry-run 和 real
  mode 都拒绝，避免把 memory 状态机错误套用到 summary。
- 继续复用 reason、actor、`confirm_destructive`、dry-run preview 和 operation audit。audit target
  type/ID 明确记录 session summary；不记录内容。
- transaction 在验证项目、quarantine state、当前 scanner match、ID/version 和 checkpoint 后才写
  ack。CAS 失败表示并发状态已变化，整体回滚并要求重新 preview。
- 成功 ack 将状态切为 acknowledged 并 enqueue/触发现有 rollup retry；真正 side-effect release 仍
  由 `B-011` 的 checkpoint 执行，不在 governance transaction 内调用 LLM 或外部写入。

### 6. Doctor/status 可见性（B-007、B-008、B-013、B-014）

`query_system_stats` 一次性返回 poisoning aggregate：pattern version、candidate/summary quarantined、
summary legacy-unscanned、source/generated breakdown、memory injection drops、summary context block
count/latest safe metadata。CLI `StatusReport` 和 HTTP `/status` 增加 `poisoning_defense` 对象。

- SQL/row/schema 错误让 `query_system_stats` 返回 error；CLI status 非 0，HTTP 无 cache 时 500。
- HTTP 既有 bounded stale cache 可以保留，但响应必须已有 `stale=true`、generated time 与 warning；
  超过最大 stale window 不返回旧 0/旧健康值。
- doctor 数据库不可打开、任一 poisoning query/breakdown/latest 查询失败时返回 `Status::Fail`；有
  quarantine/block 时为 `Warn`，全零且查询完整才为 `Ok`。
- human output 只显示 ID/pattern/version/stage/trust/count/time；JSON 不输出载荷/secret。

### 7. Capture E2E 与 adversarial-policy eval（B-017、B-018）

现有 memory bench 只根据 `retention_allowed` 构造 memory，不能证明 capture/extraction 防线。实现
必须提供两个互补证据：

1. 在 `src/eval/capture_poisoning/` 增加生产 pipeline harness：隔离数据库，调用真实 capture
   adapter/schema、migration、observation persistence、candidate verdict、rollup persistence、context
   rendering、doctor/status query；summarizer/extractor 使用确定性 fixture closure，不访问网络。
   fixture 分别覆盖 tool response、Claude/Codex transcript、英中 override、authority、opaque、secret
   混合、模型洗白输出和 benign quote。报告断言 active poison memory=0、injected poison summary=0、
   expected quarantine/block/observability 完整。
2. 把公共 `adversarial-policy` suite revision 提升为 v2，新增
   `instruction_injection`、`authority_claim`、`opaque_payload`、`benign_quoted_instruction` 类别和显式
   poisoning expectation 字段。runner 不再把 `candidate_count=0` 写死为“通过”，而是消费 capture
   harness 结果；恶意 case leak=0，benign quote 进入 expected review/quarantine，不伪装为 active
   retention。重新生成 v2 manifest/report/artifacts 与 baseline，并由 verifier 校验。

public artifact 可包含 ID、metrics、redacted fixture 和 DB snapshot；snapshot/reader input/report 在
提交前做 secret/payload safety check。恶意 fixture 使用明显的测试 token，仍必须经 redaction 后落盘。

### 8. Compatibility、文档与 GH-852 接口（B-015、B-019、B-020）

- 更新 current contract `docs/specs/memory-poisoning-defense/`，明确 raw evidence taint、summary 状态机、
  governance target、failure semantics 与 eval；`docs/specs/README.md` 保持它为 current。
- README/architecture 只记录用户可见 quarantine/recovery/status 与数据流，不复制研究报告未验证
  百分比。
- 定义内部 `PoisoningEvidenceProvider`/等价输入契约：adapter 提供稳定 source kind、project/session、
  redacted texts、event IDs 和 trust evidence。当前 Claude/Codex capture adapter 接入；GH-852 将来新增
  host-native importer 必须实现同一契约，否则返回 error。
- public CLI/MCP 新字段可选且默认 memory；HTTP status 只加字段。SQLite migration 不回填 ack，
  不调用 LLM。
- `src/**` 变更触发 remem package/version gate；所有发行 surface 同步一次 patch bump。

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| `B-001` | shared redaction/verdict、observation/rollup persistence、safe diagnostics | unit fixture 在 source/generated/doctor/status/artifact 放测试 secret；`cargo test poisoning_redaction` 断言仅有 redaction marker |
| `B-002` | capture ledger + taint propagation | integration fixture 命中后断言 captured event/raw archive 仍存在、衍生 artifact quarantined、无 silent-success counter |
| `B-003` | `memory::poisoning` versioned pattern table | `cargo test memory::poisoning` 覆盖英中五类、空白/非法 metadata、稳定 precedence |
| `B-004` | candidate multi-surface verdict | `cargo test memory_candidate::tests::poisoning` 增加 source-only match/washed-output case，active memory=0 |
| `B-005` | evidence load + extraction transaction | negative tests：missing/cross-project event、scan/storage error 均返回 error、零 candidate/summary、capture 可重试 |
| `B-006` | rollup pre-persist verdict + side-effect gate | `cargo test session_rollup::tests::poisoning` 对每个 output field/source match 断言 durable quarantine 且 topic/candidate/native 写入为 0 |
| `B-007` | summary poisoning metadata/uniqueness | migration constraint tests + repeated worker test 断言同 range 单 summary、stage/pattern/trust/range 完整且无 payload |
| `B-008` | shared summary eligibility + context/native/observation/summarize/user-context sinks | focused tests 覆盖 current/legacy/query-error/audit-error；Sessions、Claude memory、后续 prompts 与 user-context 均无载荷并有可见错误；`rg` closure audit 无裸读正文 sink |
| `B-009` | governance target enum、CLI/MCP defaults | CLI/MCP parse/integration tests：缺 target 只改 memory；explicit summary target 使用同一 action/audit；无 parallel queue |
| `B-010` | summary acknowledgement validator + transaction | table-driven negative tests 覆盖错 ID/version、升级、非 quarantine、错 project、缺 reason/actor/confirm、CAS race，数据库无部分写 |
| `B-011` | approval-triggered checkpoint release | rollup retry tests：ack 前零 side effect，ack 后各一次；candidate 仍 source-quarantined；第二次 retry 零增量 |
| `B-012` | schema state checks + CAS + range uniqueness | 并发 connection/barrier test 覆盖 worker/review/context 交错；最终状态单调、单 summary、单 release |
| `B-013` | SystemStats、CLI/API status、doctor | focused tests 覆盖完整计数、SQL failure `Fail`/non-zero/500、fresh cache、marked stale、expired stale |
| `B-014` | safe audit/log/status serialization | snapshot/JSON assertions + repository secret/payload scan；输出只含 allowlisted metadata |
| `B-015` | v070 legacy migration + injection rescan | pre-v070 migration fixture：全部旧 summary 为 legacy；safe legacy 可注入，matched/error legacy 阻断，无 LLM/ack backfill |
| `B-016` | rollup empty/parse/redaction validation | focused tests：EmptyRange 不变；合法 empty structured fields 依据 source/summary；全空/缺 required/malformed 明确失败、无 row |
| `B-017` | capture poisoning E2E harness | `cargo run -- eval-capture-poisoning --json-out /tmp/remem-capture-poisoning.json`；校验离线、两条路径、zero active/injected poison 与完整 observability |
| `B-018` | adversarial-policy v2 fixture/runner/report | `cargo run -- bench memory --suite adversarial-policy ...` + artifact verifier；required categories test、leak=0、benign expected-review metric |
| `B-019` | adapter evidence provider contract | compile/unit contract tests覆盖当前 Claude/Codex sources；不完整 provider negative fixture fail closed；GH-852 不作为 test prerequisite |
| `B-020` | SpecRail/human approval evidence | implementation preflight 人工检查 spec approval comment 指向存在的 immutable report/equivalent revision；缺失时 route 保持 blocked |
| `B-021` | migration-only legacy provenance + all runtime summary writers | upgrade/old-binary trigger tests证明只有 pre-v070 rows 为 legacy；rollup/finalize/其它 writer 的 missing/cross-project/range evidence 均 error、零 derived write；CAS/retry 不降级状态或重复 side effect |
| `B-022` | raw-row-first eligibility in `query_recent_summaries` before dedup/implicit/hybrid selection | poisoned/scanner/schema/audit-error row 与 row-absent baseline 的 implicit query、cluster/session dedup、hybrid channel/rank、abstention、selected memory IDs 完全一致，且 error visible；safe control 仍 steering |
| `B-023` | `git_trace` summary-ID loader + MCP commit tools | git/MCP tests覆盖 safe/ack、no-summary、poisoned、wrong project/summary/session identity、scanner/schema/audit error；失败返回 typed tool/query error 且 response 无 summary payload |

## 数据流

```text
hook payload / transcript / tool output
  -> existing capture redaction
  -> canonical captured_event + raw archive (retained)
  -> load complete evidence range + validate ownership
  -> redact source surfaces again
  -> deterministic source verdict ---------------------------+
  -> LLM observation / rollup (untrusted generated output)    |
  -> parse + redact generated fields                          |
  -> deterministic generated verdict                         |
                                                              v
                 combined verdict (source match cannot be laundered)
                    | safe                         | matched/error
                    v                              v
       observation -> candidate gate       durable quarantine metadata
       rollup -> summary + checkpoints      no active memory/topic/native/context
                    |                              |
                    v                              v
   raw summary row -> eligibility       existing governance target + exact ack
        before dedup/retrieval                     |
                    |                              |
                    +---------- safe/ack ----------+
                                   v
                      checkpointed, at-most-once side effects
```

Status/doctor 只聚合 metadata；它们不读取或渲染 payload。context eligibility 失败时在任何正文派生选择
前删除对应 summary，保留其他安全 sections，并通过 hook error surface 暴露错误。git/MCP trace 有明确
summary ID 时若 eligibility 失败则返回 typed error，不把失败伪装成“无 summary”。

## 备选方案

- **只加强 extraction/rollup prompt**：拒绝。模型可能遵循、复制或改写不可信输入，prompt 不是
  deterministic persistence/injection gate。
- **只扫描生成后的 candidate/summary**：拒绝。无法覆盖模型省略命中短语后的 source laundering，
  正是当前 candidate 缝隙。
- **命中即删除 captured event/summary**：拒绝。破坏审计、重试与人工 false-positive recovery，且会
  把数据丢失伪装为安全。
- **对整个 extraction task 使用现有 replay-range quarantine**：拒绝。该状态服务失败重放，不提供
  artifact 级 pattern/version acknowledgement，会混淆可靠性与安全状态机。
- **新建 summary review queue/action**：拒绝。会产生与现有 governance 不一致的确认语义和授权面。
- **把 summary 写成 memory candidate 后复用 candidate queue**：拒绝。recent sessions 是独立 context
  surface；shadow candidate 无法可靠控制 legacy summary、topic/native side effects 或 exactly-once
  release。
- **信任 source class 足够高就自动放行**：拒绝。repo file/local tool output 同样可承载投毒，trust
  只影响自动推广，不是 acknowledgement。
- **等待 GH-852 一起实现**：拒绝。当前 capture/rollup 已存在独立可利用路径；未来 importer 反向
  依赖本 verdict contract。

## 风险

- Security: false negative 会把指令载荷送入未来 context；false positive 会阻断合法引用。设计选择
  source taint + fail closed + explicit review，且所有诊断不输出 payload/secret。ack 代码、MCP 参数、
  migration constraints 与 context sink 需要 mandatory human security review。
- Compatibility: legacy summaries 默认保守复扫，可能新增 warning/quarantine；可选 target kind 保持旧
  CLI/MCP 调用 memory-only。schema version 必须在实现时重新分配下一个空闲号。
- Performance: 每次 extraction 多扫描 source/generated fields，legacy summary 每次 context load 复扫。
  pattern matcher是有界本地字符串操作；实现记录 p50/p95 与 scan bytes，并限制 status 聚合为 indexed
  counts，不能以 silent skip 换延迟。
- Reliability: raw archive、summary transaction、side effects、ack 与 context block 分属不同 checkpoint。
  并发 CAS/失败注入测试必须证明无重复 LLM、无重复 side effect、无 quarantine 回退。
- Privacy: public eval artifact/DB snapshot 可能意外携带 fixture secret 或载荷。生成后必须运行 redaction
  scan，并只提交测试 token 的 redacted 形式和安全元数据。
- Maintenance: pattern set、summary schema、current docs、eval v2 与多个 status surface 需要同步；统一
  verdict API 和 version gate 限制漂移。
- Evidence: issue 引用报告缺失使威胁优先级来源不可复核；在 human gate 关闭前不能宣称已批准或可实现。

## 测试计划

- [ ] `cargo test memory::poisoning -- --nocapture`
- [ ] `cargo test memory_candidate::tests::poisoning -- --nocapture`
- [ ] `cargo test observation_extract -- --nocapture`
- [ ] `cargo test session_rollup::tests::poisoning -- --nocapture`
- [ ] `cargo test context::tests::render_poisoning -- --nocapture`
- [ ] `cargo test context::tests::sessions -- --nocapture`
- [ ] `cargo test context::tests::retrieval -- --nocapture`
- [ ] `cargo test git_trace::tests -- --nocapture`
- [ ] `cargo test cli::tests_governance -- --nocapture`
- [ ] `cargo test mcp::server::tests -- --nocapture`
- [ ] `cargo test doctor -- --nocapture`
- [ ] `cargo test db::query::stats -- --nocapture`
- [ ] migration upgrade/schema drift/rollback tests 从 v069 fixture 开始，并验证 implementation 时实际分配的
      next-free migration 是 latest；升级前 rows 被 backfill 为 legacy，迁移后 omitted/explicit legacy
      insert 和 nonlegacy -> legacy update 均被数据库拒绝。
- [ ] `cargo run -- eval-capture-poisoning --json-out /tmp/remem-capture-poisoning.json`
- [ ] `cargo run -- bench memory --suite adversarial-policy --condition remem_default --root eval/public --artifact-prefix memory/artifacts/adversarial-policy-v2 --json-out eval/public/memory/reports/adversarial-policy-v2.json`
- [ ] `cargo run -- bench verify --root eval/public --json-out /tmp/remem-bench-verify.json`。
- [ ] `cargo run -- eval-extraction --json --check-baseline`
- [ ] `cargo run -- eval-gates --json-out /tmp/remem-eval-gates.json`
- [ ] `python3 scripts/ci/check_plugin_version_sync.py`
- [ ] `node --test plugins/remem/scripts/remem-runtime.test.js plugins/remem/apps/remem/server.test.js npm/remem/scripts/install.test.js`
- [ ] `cargo fmt --check`
- [ ] `cargo check`
- [ ] `cargo clippy -- -D warnings`
- [ ] `cargo test`
- [ ] `python3 scripts/ci/check_pr_preflight.py --base origin/main --pr-body-file /tmp/pr-body.md`，PR body
      使用 `Closes #855` 且引用批准后的 GH-855 spec；spec-only PR 仍使用 `Refs #855`。
- [ ] 人工 security review：ack authorization/CAS、redaction、日志/API payload、migration constraints、
      pre-retrieval context fail-closed、git/MCP identity binding 和 public artifacts。
- [ ] 人工 evidence gate：批准记录指向缺失研究报告的 immutable revision 或 maintainer 接受的等价
      一手证据，否则不得设置 `ready_to_implement`。

## 回滚方案

若 implementation 未合并，关闭 implementation PR，保留本 spec 与 evidence blocker。若已合并：

1. 先停止/禁用产生新 summary 的 worker，但保持 capture ledger 写入，避免丢失原始证据；这不是
   绕过 gate 的长期降级。
2. 回滚 runtime/status/governance/eval/version commit；SQLite migration 按仓库 forward-only 规则不
   降 schema、不删除 poisoning metadata。旧 binary 忽略新增列，新 binary 回退后数据仍可恢复。
3. 不把 quarantined/legacy rows 批量改成 safe 或 acknowledged，不删除 audit/block history。
4. 修复后用新的 forward migration/implementation PR 恢复，重新跑 capture E2E、adversarial-policy、
   schema drift、完整 CI 和 human security review。

任何 emergency bypass 都必须由 maintainer/security owner 明确批准并记录期限；不能通过直接 SQL
ack、关闭 scanner 或吞掉 context error 达成。

<!-- specrail-planned-changes
{
  "version": 1,
  "issue": 855,
  "complete": true,
  "paths": [
    "specs/GH855/product.md",
    "specs/GH855/tech.md",
    "README.md",
    "docs/ARCHITECTURE.md",
    "docs/specs/README.md",
    "docs/specs/memory-poisoning-defense/PRODUCT.md",
    "docs/specs/memory-poisoning-defense/TECH.md",
    "src/migrations/v070_session_summary_poisoning.sql",
    "src/migrate.rs",
    "src/migrate/types.rs",
    "src/migrate/transition.rs",
    "src/migrate/schema_drift/invariants.rs",
    "src/migrate/tests.rs",
    "src/migrate/tests_convergence.rs",
    "src/migrate/tests_schema.rs",
    "src/migrate/tests_schema_drift.rs",
    "src/migrate/tests_session_summary_poisoning.rs",
    "src/memory/poisoning.rs",
    "src/memory/governance.rs",
    "src/memory/governance/tests.rs",
    "src/memory_candidate.rs",
    "src/memory_candidate/tests/poisoning.rs",
    "src/observation_extract.rs",
    "src/observation_extract/prompt.rs",
    "src/observation_extract/response.rs",
    "src/observation_extract/commit_link_tests.rs",
    "src/observation_extract/tests.rs",
    "src/session_rollup/mod.rs",
    "src/session_rollup/parse.rs",
    "src/session_rollup/persist.rs",
    "src/session_rollup/poisoning.rs",
    "src/session_rollup/side_effects.rs",
    "src/session_rollup/tests.rs",
    "src/session_rollup/tests/citation_evidence.rs",
    "src/session_rollup/tests/followup_scheduling.rs",
    "src/session_rollup/tests/poisoning.rs",
    "src/context/poisoning.rs",
    "src/context/query.rs",
    "src/context/implicit_query.rs",
    "src/context/hybrid_context.rs",
    "src/context/types.rs",
    "src/context/claude_memory/runtime.rs",
    "src/context/claude_memory/tests.rs",
    "src/context/injection_gate/data_version_hint.rs",
    "src/context/injection_gate/tests.rs",
    "src/context/tests/mod.rs",
    "src/context/tests/load.rs",
    "src/context/tests/retrieval.rs",
    "src/context/tests/render_poisoning.rs",
    "src/context/tests/sessions.rs",
    "src/git_trace.rs",
    "src/git_trace/tests.rs",
    "src/doctor/memory_poisoning.rs",
    "src/doctor/capture_liveness.rs",
    "src/doctor/tests.rs",
    "src/db/models.rs",
    "src/db/query/summaries.rs",
    "src/db/query/stats.rs",
    "src/db/query/stats/tests.rs",
    "src/db/summarize/session/finalize.rs",
    "src/db/summarize/session/tests.rs",
    "src/summarize/summary_job/persist.rs",
    "src/summarize/summary_job/process.rs",
    "src/user_context/extraction/source.rs",
    "src/user_context/extraction/tests.rs",
    "src/user_context/recall/sources.rs",
    "src/user_context/recall/tests.rs",
    "src/user_context/summary.rs",
    "src/user_context/summary/tests.rs",
    "src/timeline/tests.rs",
    "src/cli/types.rs",
    "src/cli/eval_types.rs",
    "src/cli/dispatch.rs",
    "src/cli/actions/eval.rs",
    "src/cli/actions/maintenance.rs",
    "src/cli/actions/maintenance/tests.rs",
    "src/cli/tests_governance.rs",
    "src/cli/actions/query/status.rs",
    "src/cli/actions/query/status/types.rs",
    "src/cli/actions/query/status/tests.rs",
    "src/mcp/types.rs",
    "src/mcp/server/commit_tools.rs",
    "src/mcp/server/write_tools.rs",
    "src/mcp/server/tests.rs",
    "src/api/handlers/status.rs",
    "src/api/tests.rs",
    "src/eval.rs",
    "src/eval/capture_poisoning.rs",
    "src/eval/capture_poisoning/fixture.rs",
    "src/eval/capture_poisoning/run.rs",
    "src/eval/capture_poisoning/tests.rs",
    "src/eval/memory_bench/types.rs",
    "src/eval/memory_bench/fixture.rs",
    "src/eval/memory_bench/runner.rs",
    "src/eval/memory_bench/diagnostics.rs",
    "src/eval/memory_bench/tests.rs",
    "eval/public/README.md",
    "eval/public/memory/suites/adversarial-policy/suite.json",
    "eval/public/memory/manifests/adversarial-policy-v2.json",
    "eval/public/memory/reports/adversarial-policy-v2.json",
    "eval/public/memory/artifacts/adversarial-policy-v2/",
    "eval/public/reports/baseline.json",
    "eval/public/reports/baseline.md",
    "Cargo.toml",
    "Cargo.lock",
    "plugins/remem/.codex-plugin/plugin.json",
    "plugins/remem/runtimes/remem-releases.json",
    "npm/remem/package.json"
  ],
  "spec_refs": [
    "specs/GH855/product.md",
    "specs/GH855/tech.md",
    "docs/specs/memory-poisoning-defense/PRODUCT.md",
    "docs/specs/memory-poisoning-defense/TECH.md"
  ]
}
-->

本文件不构成 `spec_approval`。只有 maintainer 关闭证据 blocker、批准 product/tech，并把 GH-855
置为 `ready_to_implement` 后，才能创建 `tasks.md` 或开始 implementation。
