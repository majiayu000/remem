# GH760 Tech Spec: `remem user backfill` 实现设计

Issue: https://github.com/majiayu000/remem/issues/760
Product spec: `specs/GH760/product.md`
Status: Draft, needs human approval before implementation
Base: origin/main（写作时 1de527f）

## 1. CLI 面

`src/cli/types.rs` 的 `remem user` 子命令新增：

```text
remem user backfill [--apply] [--json] [--limit <n>]
```

- 无 `--apply`：dry-run，打印审计报告，退出码 0。
- `--limit`：可选，单次最多处理 n 条（默认不限），便于分批。
- `--json`：机器可读报告（stable 字段：`candidates`, `converted`, `skipped[{memory_id, reason}]`, `applied: bool`）。

## 2. 数据流

```text
memories (owner_scope='user', owner_key='user:default',
          memory_type='preference', status='active')
  → 逐条:
      1. non_retention::classify(text)      命中 → skip(reason)
      2. claim_key 生成（复用现有 claim_key 派生逻辑，输入 memory 文本）
      3. 去重: user_context_claims 中已有同 claim_key 的 active/superseded 行，
         或 source_ref 已指向该 memory id → skip(duplicate)
      4. dry-run: 记入 candidates；apply: INSERT active claim
  → 审计报告
```

### 2.1 写入语义

- 复用 `src/user_context/claims.rs` 的 claim 插入路径（与 `create_manual_claim` 同层的内部 fn，抽出共用而非复制 INSERT——同类逻辑单一来源）。
- 字段映射：
  - `claim_type = Preference`
  - `source_kind = "preference_backfill"`（新常量）
  - `confidence`：源 memory 无置信度概念，固定 1.0 并在报告中注明"人工既有记忆"；不参与 auto-promote 统计。
  - `sensitivity = Normal` / `risk = Low`（preference 行本身已过旧管线保存门槛；non-retention 在步骤 1 兜底）
  - source ref：记录 `memory:<id>`（沿用 claims 现有 source metadata 结构；`why` 直接可见）。
  - `status = 'active'`，不走 review inbox（用户显式触发即为审批动作，与 `remem user remember` 同级）。
- 文本超长：复用 claims 现有文本长度约束；超限 skip(reason=text_too_long)，不截断改写（N5）。

### 2.2 幂等

去重键双保险：claim_key 命中 或 source_ref 命中任一即 skip。apply 在单事务内执行（与 `apply_candidate_tx` 同风格），中途失败整批回滚，报告标记 `applied=false`。

## 3. 测试计划

| 测试 | 类型 | 断言 |
|---|---|---|
| dry-run 不写库 | 单元 | 前后 `user_context_claims` 行数不变（B3） |
| apply 转换 + why 溯源 | 单元 | 新 claim source_kind=preference_backfill，why 含 memory:<id>（A2/A4） |
| 幂等 | 单元 | 二次 apply 新增 0（A3） |
| non-retention 拦截 | 单元 | secret-like 文本 skip，reason 正确（B1） |
| project-scope / 非 preference / 非 active 行不入选 | 单元 | 候选集为空（P3/N2） |
| --limit 分批 | 单元 | 处理数 ≤ limit，剩余下次可继续 |
| --json 字段稳定 | 单元 | schema 快照测试 |

## 4. 迁移与回滚

- 无 schema 变更（复用现有表；`preference_backfill` 仅是 source_kind 新值）。
- 回滚：`remem user claims delete` 按 source_kind 批量治理（若 claims delete 暂不支持按 source_kind 过滤，报告中列出全部新 claim id 以便逐条删除；是否加 `claims list --source-kind` 过滤由实现时视改动大小决定，超出则另开 issue）。

## 5. 风险

- 旧 preference 有近似重复（历史上多次会话产生相似偏好行）：claim_key 派生对近似文本可能不去重 → 报告会如实列出，用户可用治理面合并；不在本命令内做语义合并（N5）。
- 固定 confidence=1.0 的语义争议：报告与 source_kind 已可区分来源；如需差异化置信度属后续迭代。
