# GH759 Tech Spec: 配置驱动的 auto-promote 治理策略

Issue: https://github.com/majiayu000/remem/issues/759
Product spec: `docs/specs/GH759/PRODUCT.md`
Status: Draft, needs human approval before implementation
Base: origin/main（写作时 1de527f）

## 1. 现状代码路径

- 提取层门槛：`should_auto_promote`（`src/user_context/extraction/mod.rs`，写作时 454-472 行附近）——claim_type ∈ {Preference, Constraint}、risk=Low、sensitivity=Normal、confidence ≥ 0.9、source_kind=="explicit_user_statement"、非 third-party framing、全部 source_event_ids user-authored、`is_supported_by_user_source_event` 文本支持。
- 存储层复核：`auto_promote_allowed`（`src/user_context/candidates.rs`，587-594 行附近）——重复校验 risk/sensitivity/confidence/source_kind + claim_key 非空。
- 拦截原因枚举：`third_party_requires_review / claim_type_requires_review / risk_requires_review / sensitivity_requires_review / low_confidence / source_requires_review / source_not_user_authored / no_supporting_source_event / missing_claim_key`。
- non-retention：`src/user_context/non_retention.rs`，在 extraction 与 candidates 两处执行。

## 2. 设计

### 2.1 配置结构

`config.toml` 新增段（沿用现有 runtime config 加载机制，`remem config` 可见）：

```toml
[user_context.auto_promote]
# 晋升所需最低置信度，默认 0.7（原 hardcode 0.9）
min_confidence = 0.7
# 允许自动晋升的 source_kind 列表，默认在 explicit_user_statement 之外
# 增加 inferred_user_preference（LLM 从用户自述行为推断、但有 user-authored 源事件支撑）
allowed_source_kinds = ["explicit_user_statement", "inferred_user_preference"]
# 是否要求 claim 文本在 user 源事件中有保守文本支持，默认放宽为 false
require_text_support = false
# 严格模式一键还原：置 true 时忽略上面三项，恢复 0.9 + explicit_user_statement + 文本支持
strict = false
```

不变（不进配置，维持 hardcode）：claim_type ∈ {Preference, Constraint}、risk=Low、sensitivity=Normal、source_event_ids 全部 user-authored、claim_key 非空、claim_key 冲突拦回 review、non-retention 全量执行。

理由：风险/敏感度/user-authored 是安全边界（B1/B3），confidence 与 source_kind/文本支持是精确率-召回率旋钮；只有旋钮进配置。

### 2.2 代码改动

1. `src/install/config.rs`（或现行 runtime config 模块）：新增 `UserContextAutoPromoteConfig` 结构 + 默认值 + 反序列化测试。缺段/缺字段用默认值（B4）。
2. `should_auto_promote(batch, candidate)` → `should_auto_promote(batch, candidate, policy: &AutoPromotePolicy)`：
   - `candidate.confidence >= policy.min_confidence`
   - `policy.allowed_source_kinds.contains(&candidate.source_kind)`
   - `policy.require_text_support` 为 true 时才调 `is_supported_by_user_source_event`
   - `strict=true` 时使用 `AutoPromotePolicy::strict()` 常量。
3. `auto_promote_allowed`（candidates.rs）同步接受 policy，两层判定用同一份 policy 实例（消除双处 hardcode 漂移风险）。
4. block reason：`low_confidence` / `source_requires_review` / `no_supporting_source_event` 语义不变，仅判定阈值来自 policy。
5. `source_kind="inferred_user_preference"`：提取 prompt 已能输出 source_kind 字段；若当前枚举无此值，允许 LLM 在系统 prompt 中按现有说明产出，本 spec 不改 prompt 的 candidate 生成语义（N5），只放行已产出但此前被一刀切拦下的值。若实现时发现提取端从未产出该值，则第一版仅放宽 `min_confidence` 与 `require_text_support`，`allowed_source_kinds` 保持单元素，并在 PR 中说明。

### 2.3 观测

- `remem doctor` 的 promotion-funnel 探针（#374）已存在：确认放宽后 funnel 数字变化可见即可，不新增探针。
- 每次自动晋升写入的 claim 保留 `source_kind` 原值，`remem user claims why` 可见判定依据。

## 3. 测试计划

| 测试 | 类型 | 断言 |
|---|---|---|
| config 默认值/缺段/非法值 | 单元 | 默认 0.7；缺段不 panic；负数/超 1.0 报错拒绝 |
| strict=true 等价旧行为 | 单元 | 与现 hardcode 判定结果逐条一致（fixture 复用现有 should_auto_promote 测试） |
| 放宽默认下 confidence 0.75 + explicit_user_statement | 单元 | 晋升 |
| confidence 0.65 | 单元 | 拦回 review，reason=low_confidence |
| sensitivity=Sensitive 任意配置 | 单元 | 永不晋升（B3） |
| non-retention 内容任意配置 | 单元 | 不落库（B1） |
| 集成 fixture（A2/A3） | 集成 | 放宽配置 ≥1 active；严格配置 0 active + ≥1 pending_review |

## 4. 迁移与回滚

- 无 schema 变更、无数据迁移。
- 回滚 = `strict = true` 或还原默认值；已晋升 claim 用现有 `claims suppress/delete` 治理。

## 5. 风险

- 错误 claim 进入 recall 面：由治理面兜底（P2），且 recall 有预算截断；观察期可用 `strict` 快速止血。
- 双层判定不一致：改动后两层共享同一 policy 实例，风险消除而非引入。
