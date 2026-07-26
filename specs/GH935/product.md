# Product Spec

## Linked Issue

GH-935

complexity: large

Locale: zh-CN

## 当前事实

PR #937 已合并 `cross-host-v1` 基础设施：versioned charter、task/run
schema、双向各 12 个 `skeleton_todo` task、artifact leak scanner 和离线
dry-run。当前没有可执行 fixture、真实宿主 harness、benchmark run、统计报告
或可公开引用的跨宿主结论；任何后续工作都必须保留这一事实，不能把
infrastructure readiness 表述为 outcome evidence。

## 用户问题

用户需要知道：Claude Code 中形成的历史经验能否在全新、隔离的 Codex
任务中可靠使用，反向是否同样成立；remem 相比目标宿主原生记忆和固定协议
导出文件是否提高 continuation outcome，并且这种提高是否以 scope leak、
陈旧记忆误用、额外成本或单向失败为代价。

现有基础设施只能证明 benchmark 合同和骨架存在，不能回答上述问题。

## 目标

- 把现有 24 个 task skeleton 升级为可执行、确定性、hidden-test 评分的
  双向任务。
- 使用来源宿主与目标宿主完全隔离的 HOME/config/session store，执行四个
  primary conditions 的完整 288-run 最小矩阵。
- 对 host-native import 做独立 paired ablation，并完整记录来源到目标的
  evidence attribution 和 exported-file 成本。
- 生成 direction-specific 与 aggregate 报告，以 task-cluster paired
  bootstrap、stop-loss 和 wording gate 决定允许公开的结论。
- 保持 benchmark 手工触发、可审计、可中断恢复；live host/network/LLM
  执行不得成为普通 CI 的隐式副作用。

## 非目标

- 不实现 Claude Code 与 Codex 的实时双向会话桥。
- 不让 remem 管理 agent lease、通用任务队列或生产并发调度。
- 不把完整来源 transcript 作为 primary condition 的默认目标上下文。
- 不把 host-native memory 自动提升为可信 canonical truth。
- 不在本 Issue 中实现 #852 的全部导入机制；这里只验证其贡献与边界。
- 不用预填 gold memory、手工 `save_memory` 或目标可见 hidden fixture
  替代真实 capture/extraction/retrieval 路径。
- 不把 diagnostic condition 的结果混入 primary claim。

## Behavior Invariants

### 生命周期与任务集

1. **B-001** 在 24 个任务全部 `ready`、完整 primary matrix 和所需
   diagnostic ablation 均执行、artifact 验证与 claim gate 均完成前，suite
   状态必须保持“基础设施/证据不足”；缺失结果不得显示为 0、PASS 或已完成。
2. **B-002** v1 可执行任务集必须恰好覆盖两个方向各至少 12 个任务，并在
   每个方向各覆盖以下 12 类一次或以上：
   `architecture_decision`、`prior_bug_root_cause`、
   `failed_attempt_lesson`、`negative_constraint`、
   `workstream_next_action`、`stale_superseded_decision`、
   `branch_specific_truth`、`multi_hop_relation`、
   `user_project_preference`、`unresolved_conflict_abstention`、
   `git_evidence`、`same_name_repo_isolation`。
3. **B-003** task 只有在 deterministic fixture、真实 source episode、
   非空 hidden tests、非空 score commands、明确 allowed/forbidden paths
   和完整 gold facts 都存在且 `todo` 为空时才能变为 `ready`。
4. **B-004** task 的必需字段缺失、为空、越界，或 `ready` 与未完成 TODO/
   空评分证据并存时，验证必须 fail closed；不得跳过该 task 后继续声称矩阵
   完整。
5. **B-005** `claude_to_codex` 的来源/目标必须分别为
   `claude_code`/`codex`，`codex_to_claude` 必须相反；来源与目标相同、
   未声明或被 alias 替代时 run 无效。

### 矩阵与可比性

6. **B-006** primary evidence 的最小完整矩阵必须是
   `24 tasks × 4 primary conditions × 3 runs = 288` 个有效 run artifact，
   两个方向各 144 个；少一个、重复顶替一个或存在未验证 artifact 都不得
   生成 complete verdict。
7. **B-007** native-memory contribution 必须使用
   `remem_without_host_native_import` 与
   `remem_with_host_native_import` 的 paired ablation；两个方向、同一 task、
   同一 run index 和相同非 ablation 配置必须成对，缺少任一侧时结论为
   insufficient。
8. **B-008** 同一 `(direction, task_id, run_index)` 的 primary conditions
   必须共享相同 fixture revision、target prompt、hidden scoring 和模型/
   reasoning 配置；只允许 condition memory surface 不同，避免用输入漂移
   制造效果差异。
9. **B-009** live host/network/LLM runs 只能由明确的人工命令启动，并记录
   操作者确认、宿主版本、模型和代码/fixture revision；普通 CI、schema
   check、dry-run 和 report verify 不得启动宿主或网络调用。

### 隔离与 condition 边界

10. **B-010** 每个 run 的来源阶段与目标阶段必须使用互不相同的新建
    HOME、config、session store 和 condition data root；目标阶段不得读取
    来源 session store、真实用户 HOME、其他 run 的数据或 benchmark-private
    paths。
11. **B-011** 任何 host HOME、auth/config、session、hidden test、
    private-root 或跨 project/user 泄漏都会使对应 run 无效并触发 suite
    stop-loss；泄漏 run 不得以“任务成功”掩盖，也不得从分母删除。
12. **B-012** 四个 primary condition 的可见 memory surface 是闭集：
    `no_memory` 无记忆面；`target_host_native` 仅有目标宿主自己的 native
    memory；`exported_file` 仅有按固定协议冻结的 repo-local handoff；
    `remem_shared` 仅通过真实 remem pipeline 写入并通过正常
    SessionStart/MCP/Context Bundle 读取。出现额外 surface 时 run 无效。
13. **B-013** `remem_shared` 的来源证据必须经过自动 capture、extraction、
    review/promotion policy 和 production retrieval；直接写入 gold memory、
    调用手工 save、预载完整 evidence 或把 expected answer 写入 prompt
    都是无效捷径。
14. **B-014** `exported_file` 必须在 target prompt 揭示前按固定、target-blind
    协议生成并冻结；生成和后续维护的 wall time、tokens、turns 与变更量必须
    作为结果的一部分记录，缺失成本的 run 无效。
15. **B-015** 来源宿主结束并清理运行态后才能启动目标宿主；阶段重叠、
    共享进程态或共享 session continuity 的 run 无效。
16. **B-016** hidden tests 在 agent 退出前不可读、不可出现在 prompt、
    memory surface 或 artifact preview 中；评分时才注入，读取尝试按
    isolation breach 处理。

### 失败、重试与部分完成

17. **B-017** auth 不可用、宿主崩溃、timeout、capture/extraction 失败、
    scoring failure、scanner failure 和 cleanup failure 都必须形成带
    failure reason 的 artifact 或显式 suite error；不得静默丢弃失败 run。
18. **B-018** 每次重试必须使用新的 `attempt_id`，并保留此前失败 artifact；
    同一 matrix tuple 的成功重试不能覆盖、删除或改写历史失败，claim
    denominator 采用预注册的 attempt policy。
19. **B-019** 中断或取消后，已完成 artifact 保持不可变；resume 只能补齐
    缺失 tuple 或新增 retry attempt，重复 tuple、hash 不一致或部分写入必须
    被拒绝而不是当作完成。

### Attribution、指标与成本

20. **B-020** 每个 `remem_shared` run 必须记录 source/target host、
    source session、captured event、promoted memory、selected context item、
    cited/used ref，以及每个 ref 的 origin、target-time scope 和 validity；
    缺任一必需链路时 run 不得计入 primary evidence。
21. **B-021** attribution ref 必须可回溯到同一 run 的真实 artifact，origin
    只能来自闭集 `remem_canonical_capture`、`host_native_import`、
    `generated_projection`、`manual_save`；未声明 ref、跨 run ref、重复冲突
    origin 或把 generated/native evidence 冒充 canonical 都必须失败。
22. **B-022** 报告的 resolved rate、recall、stale-followed、scope leak、
    memory-hurt、tokens、wall time、turns、失败动作和 attribution 指标必须
    包含所有适用的有效/失败 run，并公开分子、分母和缺失值；无数据必须为空
    或 insufficient，不得填 0。
23. **B-023** 报告必须分别展示 `claude_to_codex` 与
    `codex_to_claude`，再展示 aggregate；任一方向缺失、失败或退化不得被
    aggregate 平均数隐藏。
24. **B-024** `exported_file` 报告必须单列 generation/maintenance cost，
    并同时给出 per-task 与 aggregate 的 tokens、wall time、turns 和变更量；
    比较 resolved rate 时不得省略该成本。

### 统计、stop-loss 与公开 claim

25. **B-025** treatment/control 比较必须按 task cluster 做 paired bootstrap，
    使用记录在报告中的固定算法版本、seed、置信水平和 task-level resampling；
    两个方向分别计算置信区间，aggregate 不能替代方向结果。
26. **B-026** pair 缺失、fixture/config hash 不一致、有效 task cluster
    不足或 confidence interval 包含 0 时，不得发布“improved/beat”结论；
    CI 包含 0 时最多生成预注册的 directional/insufficient wording。
27. **B-027** stop-loss 阈值固定为：
    `wrong_project_injection = 0`、`wrong_user_injection = 0`、
    `source_private_session_leak = 0`、
    `stale_memory_followed <= 1%`、`memory_hurt <= 2%`；分母必须来自完整、
    可审计的适用 run 集。
28. **B-028** 任一安全边界泄漏或 stop-loss 超阈值都使 release/public
    comparative claim FAIL，即使 resolved rate 或其置信区间改善；不得以
    warning、降级成功或只报告无泄漏子集替代失败。
29. **B-029** host-native import 必须始终保留来源标记和非 canonical trust，
    独立报告 with/without import 的贡献、伤害和 scope/stale 指标；不得把
    diagnostic ablation 当作 primary condition。
30. **B-030** schema/charter/task/report 必须 versioned；现有
    `cross-host-v1` skeleton 与旧 artifact 不得被新 harness 静默视为
    executable/complete，兼容迁移必须显式转换并重新验证，否则拒绝。
31. **B-031** README、README.zh-CN、CHANGELOG 或 release surface 只有在
    committed report hash、完整矩阵、paired bootstrap、direction-specific
    结果和 stop-loss gate 全部通过时才能引用正向跨宿主结论；报告为
    insufficient/FAIL 时只能保留政策或“无公开结论”说明。
32. **B-032** readiness、spec approval、live-run authorization、最终 PR
    review、merge 与 release 均保持人工门禁；`implx auto` 或 benchmark
    执行授权不能替代 security、claim wording 或 release 决策。

## 验收标准

- [ ] 24 个 task 全部从 `skeleton_todo` 升级为 schema-valid `ready`，
  且两个方向各覆盖 12 个必需类别。
- [ ] isolated Claude Code/Codex source→target harness 通过正负 isolation、
  cleanup、timeout、resume 和 artifact-integrity 测试。
- [ ] 四个 primary conditions 形成 288 个验证通过的 run artifacts。
- [ ] native-memory with/without import ablation 在两个方向形成完整 paired
  evidence。
- [ ] 每个 remem run 有完整 capture→memory→selection→use attribution，
  每个 exported-file run 有生成/维护成本。
- [ ] direction-specific 与 aggregate 报告均通过 report schema，缺失值不
  被填 0。
- [ ] paired bootstrap、CI wording rule 和五项 stop-loss 被 deterministic
  gate 验证。
- [ ] public claim surface 只能引用 hash-bound、gate-passed report。
- [ ] 未实际执行任何 run 时仍明确报告 infrastructure/insufficient，不能
  产生 PASS。

## Boundary Checklist

| Boundary | Verdict |
| --- | --- |
| Empty / missing input | covered: B-001, B-003, B-004, B-006, B-020, B-022, B-026 |
| Error and failure paths | covered: B-011, B-016, B-017, B-028 |
| Authorization / permission | covered: B-009, B-016, B-032 |
| Concurrency / race / ordering | covered: B-008, B-015, B-019 |
| Retry / repetition / idempotency | covered: B-018, B-019 |
| Illegal state transitions | covered: B-001, B-003, B-004, B-030 |
| Compatibility / migration | covered: B-030 |
| Degradation / fallback | covered: B-017, B-022, B-026, B-028 |
| Evidence and audit integrity | covered: B-006, B-020, B-021, B-022, B-025, B-031 |
| Cancellation / interruption / partial completion | covered: B-017, B-019 |

## 边界情况

- 一侧宿主可执行、另一侧缺 auth 或不支持隔离时，suite 保持 partial/
  insufficient，不允许只发布可运行方向。
- primary matrix 完整但 native-import ablation 不完整时，可生成 primary
  内部报告，但 GH-935 不能完成，host-native contribution 不得推断。
- 某 task 的三次 run 全部失败仍保留在分母；不得因“无有效成功样本”删除。
- source episode 成功但 extraction 未完成时，目标阶段不得使用手工补写 memory
  继续同一个 primary run。
- interrupted report generation 可从不可变 run artifacts 重建；已有 verdict
  或 report hash 不得原地改写而不更新 provenance。

## 发布说明

本能力分两阶段交付：先落可执行 fixtures/harness/gates，再由人工授权执行
真实矩阵并提交可审计报告。只有最终 report 和 claim gate 通过后，README
才可增加经批准的结果链接；基础设施 PR、dry-run 计数或历史 CI 不能作为
发布结论。
