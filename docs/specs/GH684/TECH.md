# GH684 Tech Spec: Legacy observation dual-path retire-vs-freeze

Issue: https://github.com/majiayu000/remem/issues/684
Route: write_spec
Locale: zh-CN
Status: Draft, needs human approval before implementation
Related:
- 权威契约：`docs/specs/legacy-observation-retirement/TECH.md`
- SpecRail packet：`specs/GH684/tech.md`
- 反重写收敛：`docs/specs/current-memory-contracts/`

清单核对基线：2026-07-02 静态分类 + 2026-07-08 真实库空态确认（GH684-T5）。
行号基于 `origin/main`（本 spec 分支 base），随代码演进需在 spec-update PR
中同步。

## 1. Writer / Reader 清单（file:line）

### `pending_observations`（死队列）

| 类型 | 引用 | file:line | 路径 |
| --- | --- | --- | --- |
| writer（默认路径） | 无 | — | PostToolUse 写 `captured_events`+`ObservationExtract`，不写此队列 |
| writer（admin） | `migrate_legacy_pending` | `src/db/pending/admin/migration.rs:68` | `remem pending migrate-legacy`，把行改记为 `captured_events` 并标 `migrated` |
| writer（admin） | retry/purge failed | `src/db/pending/admin/mutate.rs:8,27,59` | 手动迁移准备 |
| reader | Stop hook 计数读 | `src/observe/hook.rs:565` | 只读一个 count 记 "ignored N legacy pending" |
| reader | status 计数 | `src/cli/actions/query/status.rs:168-173` | ready/delayed/processing/expired/failed |
| reader | doctor 活性/恢复 | `src/doctor/capture_liveness.rs:49-62` | failed 计数 + 迁移引导 |
| reader | doctor legacy 表面 | `src/db/query/legacy_surfaces.rs`（`legacy_pending_observation_surface`） | 行数/last-write/frozen 违规 |
| 备注 | claim/lease + `enqueue_pending` 已删 | GH684-T6 | `queue.rs`/`claim.rs` 已从 crate 移除；测试经 `db::test_support::insert_legacy_pending_fixture` 播种 |

### `observations`（+ `observations_fts`）（当前中间态，非 legacy）

| 类型 | 引用 | file:line | 路径 |
| --- | --- | --- | --- |
| writer | `persist_observations` | `src/observation_extract.rs:323` → `src/db/observation.rs` | `ObservationExtract` 抽取任务内 |
| writer | 压缩写入 | `src/summarize/compress.rs:99`（`store_compressed_observations`） | Stop-hook compress 作业 |
| reader | MCP `get_observations(source='observation')` | `src/mcp/server/context_tools.rs:141,143` | 当前 "extracted observations"（GH684-T8 措辞） |
| reader | timeline anchor | `src/retrieval/search/observation.rs:10,36` → `src/db/query/search.rs:10`（`search_observations_fts`） | 仅 `remem timeline`，不触达主 `search` |
| reader | 候选晋升证据 | `src/memory_candidate.rs` | `captured_events→observations→candidates→promoted` |
| `observations_fts` writer | 迁移定义的 SQL 触发器 | `src/migrations/v001_baseline.sql:184` | 无 Rust 写者 |

### `session_summaries`（共享，双写 —— 真正目标）

| 类型 | 引用 | file:line | 处置相关 |
| --- | --- | --- | --- |
| writer（当前） | `persist_session_rollup` | `src/session_rollup/persist.rs:9,34` | `SessionRollup` 抽取任务 |
| writer（旧） | `enqueue_summary_jobs` | `src/summarize/summary_job/hook.rs:220` | → worker `JobType::Summary`（`src/worker.rs:57`） |
| writer（旧 finalize） | `finalize_summarize` | `src/db/summarize/session/finalize.rs:4,24,28` | DELETE+INSERT |
| reader | context 注入 sessions | `src/context/query.rs:613`、`src/context/claude_memory/runtime.rs:157`、`src/context/injection_gate/data_version_hint.rs:444` | 载荷关键 |
| reader | user-context | `src/user_context/summary.rs:512`、`src/user_context/extraction/source.rs:415` | recall/extraction |
| reader | timeline | `src/timeline/summary.rs:28,125`、`src/timeline/detail.rs:43` | 月度聚合 |
| reader | `remem why` | `src/git_trace.rs:469,501` | git trace join |
| reader | observation-extract 上下文 | `src/observation_extract.rs:284` | 抽取输入 |
| reader | doctor/status | `src/doctor/capture_liveness.rs:101,255,270,285`、`src/db/query/stats.rs:142` | 计数/活性 |
| governance | scope cleanup | `src/memory/scope_cleanup/mutate.rs` | 手动 |

## 2. Retire-vs-Freeze 决策矩阵（已记录决策）

**总决策：RETIRE（收敛到 capture ledger），不是 freeze。**
理由：freeze（只读、保留写者）在这里是反模式——"冻结表面却仍有活跃写者"
是 bug 不是 warning（`legacy-observation-retirement/TECH.md` 设计规则）。因此
对死表面直接 retire、对双写者删掉冗余那一路，而非长期只读并存。

| 表面 | 决策 | 依据 | 移除机制 |
| --- | --- | --- | --- |
| `pending_observations` | **retire**（迁移后 drop） | 默认路径无写者，dogfood + 备份库全 0 行（GH684-T5）；写/claim 表面已删（GH684-T6） | drop 前弃用窗口 ≥1 个 minor release + guarded 迁移（预检残留行拒绝）；`migrate-legacy` 作逃生舱 |
| `observations` | **reclassify-current**（保留） | 抽取管线活跃中间态 | 不 drop；防措辞回退 |
| `observations_fts` | **reclassify-current**（保留） | 触发器维护，跟随 `observations` | 不 drop |
| `session_summaries`（表） | **keep** | context/timeline/user-context 载荷关键 | 不 drop |
| 旧 summary 写者链（`enqueue_summary_jobs`→`JobType::Summary`→`finalize_summarize`） | **retire-summary-writer-only** | 与 `SessionRollup` 重复的双写者；dogfood 上 2479 失败作业 + 24019 无归属调用 | 等价 fixture 通过 + Stop 副作用改归属后，仅删 Summary 作业路径，不删表 |

弃用窗口/移除日期机制：drop 迁移只在 guarded 预检确认残留价值行为 0（除 spec
显式判为无价值者）后执行；至少跨一个 minor release，其间 doctor 播报即将
drop、release notes 携带该条；drop 与 `src/migrate/schema_drift.rs` 更新同 PR。

## 3. 分阶段实现 issue 拆分

每阶段独立可评审，均需通过：
```bash
cargo fmt --check && cargo check && cargo test
```

### Phase 1 — Inventory + Decision（spec-only，本 PR）
- 文件范围：`docs/specs/GH684/*`、（如需）`docs/specs/legacy-observation-retirement/TECH.md`。
- 验收：清单表 file:line 与代码一致；决策矩阵每表面一条已记录决策。
- 状态：大部分已由现有权威契约完成；本 PR 为 GH684/ 目录镜像。

### Phase 2 — Doctor 可观测
- 文件范围：`src/db/query/legacy_surfaces.rs`、`src/doctor/capture_liveness.rs`、
  `src/cli/actions/query/status.rs`、对应 tests。
- 验收：doctor 新 section 报告每表面 row_count / last_write_epoch / disposition /
  frozen_write_violations；冻结表面收到写入 → error finding；`status --json` 镜像。
- 现状：`query_legacy_surface_stats` 已存在（disposition/row_count/last_write/
  frozen_write_violations 五表面），需补冻结写入 error 判定与文档。

### Phase 3 — Writer Freeze（按 retire 集）
- 3a 旧 summary 链：文件 `src/session_rollup/`、`src/summarize/summary_job/`、
  `src/worker.rs`、`src/db/summarize/session/`。验收：等价 fixture
  `summary_writer_equivalence_fixture_documents_field_level_deltas`
  （`src/session_rollup/tests.rs`）锁定字段契约；Compress/Dream/raw archive/
  citation/failure-lesson/candidate finalize/native-memory sync 副作用有新
  归属后再删 `JobType::Summary`。
- 3b `pending_observations`：写/claim 表面删除已由 GH684-T6 完成；保留
  read/report + admin。
- 现状：3a 的 GH684-T2/T3/T4 已落等价与副作用回归；剩 in-flight 作业升级处理
  与最终 writer 删除待决。

### Phase 4 — Value Migration + Drop
- 文件范围：`src/db/pending/admin/migration.rs`、新 drop 迁移
  `src/migrations/`、`src/migrate/schema_drift.rs`、`remem cleanup` 动作。
- 验收：`migrate-legacy` 报告 migrated/skipped/valueless 计数；弃用窗口后 guarded
  drop（预检拒绝残留行）；drop 后 schema-drift 测试同 PR 更新；2479 失败旧作业
  经显式 `remem cleanup` 清理而非静默迁移。

## 4. 迁移 / 回填计划

- `pending_observations`：`migrate_legacy_pending`（`src/db/pending/admin/migration.rs:68`）
  把残留行改记为 `captured_events` 并标 `migrated`，作为野外非空库的迁移路径；
  扩展其报告输出 migrated/skipped/valueless。drop 前 guarded 预检确认为 0 行。
- 旧 summary 链：无行级回填——`session_summaries` 表保留；只需把
  `finalize_summarize` 的载荷字段（request/decisions/learned/next_steps/
  preferences）在 `persist_session_rollup` 侧确保等价（GH684-T3 已 port），
  cooldown 侧效作为单独退役项。
- 兼容：跨 drop 迁移不支持降级，schema-version gate 已拒绝旧二进制读新 schema。
  加密库走正常 open 路径，无特判。

## 5. Doctor 报告新增

- 复用 `LegacySurfaceStats`（`src/db/query/legacy_surfaces.rs`）：per-surface
  `row_count` / `last_write_epoch` / `disposition` / `frozen_write_violations`。
- capture_liveness 新增：冻结表面（如 `pending_observations` 写路径已删后仍
  出现新写入、或 `session_summaries` 被非 rollup 路径写入）→ **error** finding，
  非 warning。
- `remem status --json` 镜像上述计数供脚本消费。
- 弃用窗口期：doctor 播报即将 drop 的表面及其目标 release。

## 6. 验证

```bash
cargo fmt --check
cargo check
cargo test
```
分阶段附加：Phase 3 等价 fixture + Stop 副作用回归；Phase 4 迁移幂等 +
guarded-drop 拒绝 + 迁移后 schema-drift 测试；每次 drop 前在 epic 记录一次
dogfood 干跑。

## 7. Open Questions

- 升级时刻在途 `JobType::Summary` 作业：排空、拒绝、还是转 `SessionRollup`？
- `get_observations` 措辞修复后，`source='observation'` 名称保留还是改名
  （client churn 权衡）？
- 旧 `finalize_summarize` 的 `summarize_cooldown` 侧效退役后是否需要在 rollup
  侧等价替换。
