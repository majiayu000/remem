# Token 浪费诊断报告 — 2026-02-21

## 现象

remem 最近 token 消耗异常高。单日 445 次 AI 调用，其中 77% (343 次) 为无效浪费。

## 根因分析

### BUG-1: 短命 Session 风暴（严重）

`social-auto-upload` 项目在 20 分钟内创建 187 个独立 Claude Code session（每 ~6 秒一个），
`remem` 项目同期 135 个。每个 session 都走完整生命周期：

```
session-init → observe → summarize → flush AI → summary AI
```

**根因**: 外部脚本/自动化频繁重启 Claude Code，每次生成新 session_id，
remem 把每一个都当独立会话处理。

**数据**:
- 爆发期 01:00-01:20，343 次 AI 调用
- 产出 202 条重复 summary（内容几乎相同）
- 浪费 ~147,251 output tokens

### BUG-2: Summarize 缺少去重 Gate

`summarize()` 的 gate 只检查 `count_pending(session_id) > 0`，不检查：
- 同项目是否刚生成过 summary（无冷却期）
- assistant message 是否与上次相同（无内容去重）
- session 是否足够长（短命 session 不值得 summarize）

**位置**: `src/summarize.rs:90-98`

### BUG-3: 旧版 Summary 数据污染

数据库中 329 条 `mem-*` session summary 无对应 observation（旧版 claude-mem 迁移残留）。
虽然 context 查询有 LIMIT 11 保护，但：
- 数据库体积持续膨胀（13MB）
- 同 session 重复 summary 未被 UPSERT 清理（旧版 ssh session 有 15 条）

### BUG-4: 缺少全局 Rate Limit

多项目并行时 AI 调用无频率限制。445 次调用集中在 ~3 小时内，
无"每分钟最多 N 次"或"每项目冷却期"保护。

## 影响

| 指标 | 数值 |
|------|------|
| 日 AI 调用 | 445 次（正常应 ~100） |
| Output tokens | ~209,607 |
| Input tokens (估算) | ~1,335,000 |
| 浪费占比 | 77% |
| Haiku 成本 | ~$0.60/天（$0.46 浪费） |

## 修复方案

1. **Summarize 冷却期** — 同项目 5 分钟内不重复 summarize
2. **Session 最小活动量** — pending < 3 且 session 时长 < 60 秒时跳过
3. **Message hash 去重** — 相同 assistant message 不重复处理
4. **全局 Rate Limit** — 每分钟最多 N 次 AI 调用
5. **清理旧数据** — 删除无效 `mem-*` summary + 同 session 重复 summary
