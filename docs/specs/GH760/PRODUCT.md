# GH760 Product Spec: 旧 user-scope preference 记忆显式回填为 user_context_claims

Issue: https://github.com/majiayu000/remem/issues/760
Route: write_spec
Locale: zh-CN
Status: Draft, needs human approval before implementation
Related: #575, #759
Evidence: `docs/artifacts/multi-ai-research-personal-memory-grok-gemini-20260707-144830.md`（随 GH759 spec PR 入库）

## 1. 背景

remem 有两套用户个人记忆数据面：旧的 `memories` 表 preference 行（含 `owner_scope='user' AND owner_key='user:default'` 的全局用户偏好）与新的 `user_context_claims`。两者只在读侧（`remem user summary` 的 `load_memory_sources`）汇合，不存在任何写侧迁移。存量用户偏好永远无法获得 claim 层的治理能力（suppress/supersede/why 审计链）与按需 recall 的 claim 语义。

业界对标：Gemini 2026-03 的 Import（从其他 AI 导入 memories / 聊天历史）是用户显式触发的迁移工具；Grok 与 Gemini 均无静默全量回填。回填应显式、幂等、可审计。

## 2. 目标

P1. 新增用户显式触发的回填命令：`remem user backfill`。

P2. 默认 dry-run：只输出候选与决策清单，不写库；`--apply` 才执行。

P3. 范围：`owner_scope='user' AND owner_key='user:default' AND memory_type='preference' AND status='active'` 的 memories 行。

P4. 每条回填 claim 保留源引用：可经 `remem user claims why` 追溯到源 memory id。

P5. 幂等：重复运行（含 dry-run → apply → 再 apply）不产生重复 claim。

P6. 审计报告：转换 / 跳过（含原因：non-retention 拦截、去重命中、文本超限等）逐条列出，机器可读（`--json`）+ 人读两种输出。

P7. 不删除、不修改任何源 preference 行。

## 3. Non-Goals

N1. 不做自动 / 定时回填；仅手动命令。

N2. 不回填 project-scope preference（首版只做 user-scope；project 偏好的 claim 化属后续议题）。

N3. 不做反向迁移（claims → memories）。

N4. 不改变 summary 读侧现有的三路汇合行为。

N5. 不引入 LLM 调用：回填是确定性数据搬运，不重写文本。

## 4. 行为不变量

B1. non-retention 分类命中的内容不回填（与自动提取同一拦截层）。

B2. 源 preference 行字节不变。

B3. dry-run 零写库（含无审计行写入之外的任何表变更；审计报告本身输出到 stdout/文件，不入库）。

B4. 回填 claim 的 `source_kind` 为独立标识（如 `preference_backfill`），与手动 / 自动来源可区分。

## 5. 验收

A1. dry-run 输出的候选数 = 满足 P3 条件且未被 B1/去重拦截的行数（测试覆盖）。

A2. apply 后 `remem user claims list` 新增数 = 报告声明的转换数。

A3. 再次 apply 新增 0 条（幂等测试）。

A4. 任一回填 claim 的 `why` 输出含源 memory id。

A5. `cargo test` 全绿。
