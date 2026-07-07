# GH759 Tech Spec: 配置驱动的 auto-promote 治理策略

Issue: https://github.com/majiayu000/remem/issues/759
Product spec: `specs/GH759/product.md`
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
# 允许自动晋升的 source_kind 列表；首版默认保持现有安全边界
allowed_source_kinds = ["explicit_user_statement"]
# 是否要求 claim 文本在 user 源事件中有保守文本支持；首版默认保持 true
require_text_support = true
# 严格模式一键还原：置 true 时忽略上面三项，恢复 0.9 + explicit_user_statement + 文本支持
strict = false
```

不变（不进配置，维持 hardcode）：claim_type ∈ {Preference, Constraint}、risk=Low、sensitivity=Normal、非 third-party framing、source_event_ids 全部 user-authored、claim_key 非空、claim_key 冲突拦回 review、non-retention 全量执行。

理由：风险/敏感度/third-party framing/user-authored/non-retention 是安全边界（B1/B3），confidence 是首版默认放宽旋钮；source_kind/text-support 可以配置，但只有在下述保护实现后才能作为默认放宽项。

### 2.2 代码改动

1. `src/install/config.rs`（或现行 runtime config 模块）：新增 `UserContextAutoPromoteConfig` 结构 + 默认值 + 反序列化测试。缺段/缺字段用默认值（B4）。
2. `should_auto_promote(batch, candidate)` → `should_auto_promote(batch, candidate, policy: &AutoPromotePolicy)`：
   - `candidate.confidence >= policy.min_confidence`
   - `policy.allowed_source_kinds.contains(&candidate.source_kind)`；默认只允许 `explicit_user_statement`
   - `policy.require_text_support` 为 true 时调 `is_supported_by_user_source_event`；默认保持 true
   - `strict=true` 时使用 `AutoPromotePolicy::strict()` 常量。
3. `auto_promote_allowed`（candidates.rs）同步接受 policy，两层判定用同一份 policy 实例（消除双处 hardcode 漂移风险），但 `third_party_requires_review`、risk、sensitivity、claim_key、claim_key conflict、non-retention 不受配置影响。
4. block reason：`low_confidence` / `source_requires_review` / `no_supporting_source_event` 语义不变，仅判定阈值来自 policy。
5. 非默认允许 inferred source kind 时必须使用现有 parser 值 `inferred_from_behavior`，并同步更新 extraction prompt，使模型不会继续把 inferred claims 标成 review-only risk；否则默认 `allowed_source_kinds` 保持单元素。
6. 非默认 `require_text_support=false` 时，不能只跳过 `should_auto_promote` 内的文本支持检查；还必须让 earlier candidate queue support gate policy-aware，并在 auto-promote 前扫描 cited source event 的完整相关文本，避免 source_preview 为空或只含匹配片段时绕过 secret/external/non-retention 保护。
7. Codex drain-only 捕获不是本 issue 的实现范围；默认产出 fixture 必须构造 user-authored captured event，不能用 Codex 无 user prompt 捕获路径证明。

### 2.3 观测

- `remem doctor` / status 目前的 promotion-funnel 主要覆盖 `memory_candidates`。实现 GH-759 时必须新增或扩展 user-context candidate/claim 统计，或在 PR 中明确不用 doctor funnel 作为验收证据。
- 每次自动晋升写入的 claim 保留 `source_kind` 原值，`remem user claims why` 可见判定依据。

## 3. 测试计划

| 测试 | 类型 | 断言 |
|---|---|---|
| config 默认值/缺段/非法值 | 单元 | 默认 0.7；缺段不 panic；负数/超 1.0 报错拒绝 |
| strict=true 等价旧行为 | 单元 | 与现 hardcode 判定结果逐条一致（fixture 复用现有 should_auto_promote 测试） |
| 放宽默认下 confidence 0.75 + explicit_user_statement + text support | 单元 | 晋升 |
| confidence 0.65 | 单元 | 拦回 review，reason=low_confidence |
| sensitivity=Sensitive 任意配置 | 单元 | 永不晋升（B3） |
| third-party framing 任意配置 | 单元 | 永不晋升，reason=third_party_requires_review |
| non-retention 内容任意配置 | 单元 | 不落库（B1） |
| inferred_from_behavior 非默认放行 | 单元/集成 | 只有 prompt/parser/queue-support/non-retention source 扫描同时更新后才可晋升 |
| 集成 fixture（A2/A3） | 集成 | user-authored fixture 放宽配置 ≥1 active；严格配置 0 active + ≥1 pending_review |

## 4. 迁移与回滚

- 无 schema 变更、无数据迁移。
- 回滚 = `strict = true` 或还原默认值；已晋升 claim 用现有 `claims suppress/delete` 治理。

## 5. 风险

- 错误 claim 进入 recall 面：由治理面兜底（P2），且 recall 有预算截断；观察期可用 `strict` 快速止血。
- 双层判定不一致：改动后两层共享同一 policy 实例，风险消除而非引入。
