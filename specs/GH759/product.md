# GH759 Product Spec: user-context auto-promote 从事前强审查转向"先入库、事后治理"

Issue: https://github.com/majiayu000/remem/issues/759
Route: write_spec
Locale: zh-CN
Status: Draft, needs human approval before implementation
Related: #674, #579, #760
Evidence: `docs/research/multi-ai-research-personal-memory-grok-gemini-20260707-144830.md`

## 1. 背景

user_context_claims 是 remem 的用户个人记忆 claim 层。自动晋升（auto-promote）当前是双层事前门槛：claim_type ∈ {Preference, Constraint}、risk=Low、sensitivity=Normal、confidence ≥ 0.9、source_kind="explicit_user_statement"、全部源事件 user-authored、且 claim 文本需在 user 源事件中有保守文本支持（`src/user_context/extraction/mod.rs` `should_auto_promote` + `src/user_context/candidates.rs` `auto_promote_allowed`）。

实际效果：管线上线以来 active claims 恒为 0，review inbox 恒为 0。Codex capture 为 drain-only（无 user_prompt_submit 事件），几乎没有事件能满足证据链要求。事后治理面（`claims suppress/delete/why`、supersedes 审计链）已完整实现但永远无事可做。

业界对标（2026-07-07 多 AI 交叉调研，Grok + ChatGPT 双外部一致 + 官方文档）：Grok 与 Gemini 的自动个人记忆均为"先入库、事后治理"，无公开置信度门槛、无入库前审查；治理靠单条删除 / 开关 / Temporary 模式。remem 现有门槛严于业界一个量级。

## 2. 问题

1. 自动 claim 产出为 0，claim 层对用户没有实际价值。
2. 门槛条件 hardcode 在代码里，无法按部署环境调整。
3. 精确率/召回率取舍被固定在极端精确率一侧，与已建成的事后治理能力不匹配。

## 3. 目标

P1. auto-promote 门槛条件配置驱动（config.toml），默认值放宽到可产出水平（confidence 默认 0.9 → 0.7）。

P2. 放宽后自动晋升的 claim 必须可单条治理：suppress / delete / edit / why 全部可用，审计行保留。

P3. non-retention 拦截层行为完全不变（secret / speculative / temporary / general-knowledge / illegal / external 不入库）。

P4. 严格模式可经配置恢复：把阈值和条件调回现值即还原当前行为。

P5. 未过门槛的 candidate 仍进 review inbox（现有 pending_review 流转不变）。

## 4. Non-Goals

N1. 不改变手动 `remem user remember` 路径。

N2. 不改变 claim_key 冲突拦回 pending_review 的行为。

N3. 不解决 Codex user_prompt_submit 事件捕获缺失（独立议题）。

N4. 不做旧 preference 回填（#760 负责）。

N5. 不新增 LLM 调用或改变提取 prompt 的 candidate 生成部分。

## 5. 行为不变量

B1. non-retention 分类的内容在任何配置下都不落库。

B2. 每条自动晋升 claim 的 source refs / block reason 审计语义不变。

B3. sensitivity != Normal 或 risk != Low 的 candidate 在默认配置下仍不自动晋升（放宽仅针对 confidence 与 source_kind，是否放宽 source_kind 由 TECH spec 决定并可配置）。

B4. 配置缺失时使用默认值，不 panic、不静默跳过提取。

## 6. 验收

A1. 阈值/条件可经 config.toml 调整，且解析、默认值、边界有单元测试。

A2. 用默认（放宽后）配置对含用户偏好表述的 session 事件跑提取，产出 ≥ 1 条 active claim（集成测试 fixture）。

A3. 严格配置（等价旧默认）下同一 fixture 产出 0 条 active claim、≥ 1 条 pending_review。

A4. `cargo test` 全绿；现有 adversarial user-context 回归套件（#625）不弱化。
