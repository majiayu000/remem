---
spec: memory-system-v2.1-revisions
status: proposed
date: 2026-05-08
owner: remem
revises: SPEC-memory-system-v2-no-compat-2026-05-08.md
sources:
  - Claude Code v2.1.121 hooks reference (2026-04-28)
  - openai/codex rust-v0.129.0 hooks schema (2026-05-07)
  - remem 0.3.12 src/ audit (5/5 SPEC 落地状态)
---

# v2.1 修订:基于 host 能力调研的 SPEC 补丁

## 0. 背景

v2 SPEC 方向(分层、队列正确性、review)已审过接受。三方调研发现 11 条具体差距:

1. Claude Code 2.1.121 的 hook payload 不含 `turn_id` / `host` / `workspace`;`transcript_path` 在 Stop 时有 race
2. Codex Rust v0.129.0 的 hook 没有 `SubagentStop` / `host` 标识 / `event_id` / 内置后台 spawn 路径
3. 5/5 SPEC 的 Phase 1/2/3 已落地 80%+,几乎全部可平移到 v2 schema(SPEC §15 没承认这点)
4. §17 三条 open decisions 直接阻塞 Phase 1 schema

本文件作为 v2 SPEC 的修订补丁,**不重写原文,只逐条标注修改点**。后续 PR 在描述里 reference 本文件的条目编号(M1-M5 / S1-S4 / N1-N2)作为 review checklist。SPEC 主文件保持不变,v2 ship 后再合并修订形成 v3。

---

## 1. Must-fix(开工前必须 patch)

### M1. §1.3 / §6 显式区分六元组中"原生 vs 合成"字段

**修订**: 在 §1.3 末尾追加:

> 六元组必须显式标注字段来源;host 一律由 install 注入,部分字段由 host payload 提供,其余由 remem 在 capture hook 入口合成:
>
> | 字段 | 来源 | 合成规则 |
> |---|---|---|
> | host | install 时写入 hook 命令行 `--host` 参数 | claude-code / codex-cli |
> | workspace | 运行时从 cwd + `git rev-parse --show-toplevel` 解析,fallback 到 cwd | 必填 |
> | project | workspace + (子目录 / 显式 `--project` 标签) | 必填 |
> | session_id | host payload | 两边都提供 |
> | turn_id | codex-cli turn-scoped payload;claude-code 由 remem 自增计数器(SQLite + 文件锁)合成 | 必填 |
> | event_id | `(turn_id, hook_event_name, tool_use_id?)` 复合 | remem 合成 |

**为什么**: Claude Code payload 不暴露 turn_id;Codex turn-scoped hook 暴露 turn_id 但不暴露 event_id;两者都不暴露 host。SPEC 默认六元组都来自 host 会导致 Phase 1 schema 写到 captured_events 索引时回头改。

### M2. §4.2 host 语义改成"install 注入,运行时验证"

**原文**: "Hook 不能识别 host 时不写 task,写一条 structured error log"

**改为**:

> v2 不允许 `host = unknown`。host 由 install 流程在 hook 配置中**硬编码到命令行参数**,不依赖运行时识别。
>
> - claude-code: hook command 形如 `remem observe --host claude-code`
> - codex-cli: hook command 形如 `remem observe --host codex-cli`,或通过 wrapper 脚本注入 `REMEM_HOST=codex-cli`
>
> 运行时若读不到 `--host`,视为 install 损坏:拒绝写入 captured_events,doctor 报错并提示重新 `remem install --target ...`。

**为什么**: Codex 不导出 `CODEX_VERSION`,Claude Code 的 `$CLAUDECODE=1` 未官方文档化,运行时识别 host 不可靠。把识别问题前移到 install 时是唯一确定性方案。

### M3. §4.3 / §15 Phase 6 install 必须为两个 host 写两份配置

**修订**: §4.3 与 §15 Phase 6 deliverables 追加:

> install 流程为每个目标 host 生成独立配置:
>
> | host | 配置文件 | 启用要求 |
> |---|---|---|
> | claude-code | `~/.claude/settings.json` 的 `hooks` 字段 | 无 |
> | codex-cli | `~/.codex/config.toml` 的 `[[hooks.PostToolUse]]` 表(优先)或 `~/.codex/hooks.json` | **必须 `[features] codex_hooks = true`**,install 自动设置 |
>
> 两份配置共用同一 remem 二进制,通过 `--host` 参数区分。

**为什么**: Codex hook 在没启用 features flag 时不触发。官方文档确认 inline TOML hooks 需要 `[features] codex_hooks = true`;`features.hooks` 不写入 SPEC,除非后续源码/PR 证明确认为受支持 alias。

### M4. §15 Phase 1 / 3 加 5/5 已落地代码复用清单

**修订**: §15 Phase 1 deliverables 后追加:

> 5/5 SPEC 已落地代码可作为实现模板平移,但不能机械照搬。v2 只复用 budget / lease / heartbeat / host isolation 的结构;progress metric 必须改成"event range 或 claimed rows advanced",不能沿用"生成 observation 数"。
>
> | 5/5 工作 | 源文件 | 平移到 v2 |
> |---|---|---|
> | drain budget 常量 | `src/observe_flush/constants.rs:7-11` | 改名 `EXTRACTION_DRAIN_*` |
> | drain loop + ObservationDrainOutcome | `src/observe_flush/batch.rs:26-38` | 挪进 `src/extraction/worker_loop.rs`,但 `0 observations` 不能等于 drained |
> | release_expired_pending_claims | `src/db_pending/claim.rs:87-101` | 改名 `recover_expired_extraction_leases`,SQL 改表名 |
> | host-aware claim 三元组 | `src/db_pending/claim.rs:7-15` | 三元组改 `host_id` FK,逻辑保持 |
> | host-aware enqueue dedupe | `src/db_job/enqueue.rs:6-25` | dedupe 改用 `idempotency_key UNIQUE` |
> | worker_heartbeats 表 | `src/migrations/v004_worker_heartbeat.sql` | 加 `mode TEXT` 列对齐 §6.13 |
> | db_worker.rs 整文件 | `src/db_worker.rs:14-52` | 直接保留 |
> | record_worker_heartbeat | `src/worker.rs:113-118 / 173 / 273` + 测试 :610 | 直接保留 |
> | stuck-job stats fix | `src/db_query/stats.rs:115` | 直接保留 |
>
> 必须**删除**(v2 不允许 unknown host):
> - `src/db_pending/claim.rs:27, 54` 的 `host = ?3 OR host = 'unknown'` 兼容分支
>
> 必须**修正**(v2 liveness invariant):
> - `src/observe_flush/batch.rs:33-35, 150-152` 现有逻辑把 `flush_pending_once() == 0` 当作 drained;v2 中 task 进度只能由 claimed event range 是否推进决定。空提取输出应标记该 range advanced,然后继续 drain 或检查 remaining ready work。

### M5. §17 D1/D2/D4 关闭

参见本文件 §4。Phase 1 schema 在三条决策关闭前不得开工。

---

## 2. Should-fix(实现 Phase 内可解决)

### S1. §7.2 transcript 读取语义

**插入**:

> transcript 读取在两个 host 上语义不同,capture hook 必须分支处理:
>
> - **claude-code**: Stop hook 触发时 `transcript_path` 末尾消息可能未落盘。capture 入口应:
>   1. 优先从 payload 读 `last_assistant_message`(若提供)
>   2. 降级:读已 flush 部分,把"截断"标进 `captured_events.retention_class`,在下次 SessionStart 补全
>   3. 不允许 hook 内 sleep+retry(违反 §7.1 hook 必须快速返回)
>
> - **codex-cli**: `~/.codex/sessions/rollout-*.jsonl` 由 RolloutRecorder 每条 flush,Stop 前已写入完整内容,直接读即可,无 race。

### S2. §8.4 Stop fallback 实现约束

**插入**:

> Stop hook 在 daemon 不健康时 spawn `worker --once`,**子进程必须 detach**:
>
> - macOS: `launchctl kickstart -k gui/$UID/com.remem.worker.once`(预先 `launchctl bootstrap gui/$UID <plist>` 安装一次性 LaunchAgent label)
> - Linux: `systemd-run --user --scope --collect remem worker --once`
> - 通用 fallback: `setsid remem worker --once </dev/null >>~/.remem/worker.log 2>&1 &`,立即 return
>
> 父 hook 不允许调用 `wait`。Codex 的 `kill_on_drop=true` 会让父进程退出时连带 kill 子进程,因此必须真正 detach。

### S3. §6.4 sessions 表 subagent 语义注释

**插入到 §6.4 schema 后**:

> **subagent 语义**:
> - host=claude-code: 一次用户交互可能产生多个 session_id(主 agent 与 SubagentStop 触发的子 agent 各自独立)。promotion 引擎需要把同一 `user_message` 触发的多 session_id 视为一个证据单元
> - host=codex-cli: 没有 SubagentStop hook,一个 session_id = 一次完整交互
>
> 后续 phase 的 evidence_event_ids 关联逻辑必须考虑这个差异。

### S4. §4.1 legacy schema 容许 read-only 命令

**原文**: "v2 detects legacy schema -> refuse to start writable commands"

**改为**:

> read-only 命令(`status`, `doctor`, `search`, `show`, `pending list-failed`)在 legacy schema 下仍可用,只锁写命令(`worker`, `summarize`, `observe`, `dream`, `save_memory`)。doctor 输出固定文案:
>
> ```
> Legacy schema detected (version v002).
> Run `remem admin backup` then `remem admin reset-v2 --confirm-destructive` to upgrade.
> Read-only commands remain available.
> ```

---

## 3. Nice-to-have

### N1. §10 promotion 阈值 calibration

ship Phase 4 前用现有 1777 条 memory(README 内部 eval 数据集)跑一次 calibration,验证 `confidence >= 0.82` 的 precision/recall。若 precision < 0.9 或 recall < 0.5,调整阈值或加更多 review 触发条件。

### N2. §11.2 context compiler 与 Claude additionalContext 耦合

claude-code 的 SessionStart hook 可通过返回 `additionalContext` 字段直接注入文本。Phase 5 context compiler 应优先走这个路径,而非自己拼 prompt。codex-cli 当前没有等价机制,fallback 到 MCP 主动 search。

---

## 4. §17 Open Decisions 推荐答案(解锁 Phase 1)

### D1. captured_events.content_text 是否压缩 assistant full text

**推荐**: **压缩**。

策略:
- ≤ 16 KiB: 存 `content_text`
- 16 KiB - 256 KiB: 存 `event_blobs`(plain),`content_text` 留 prefix/suffix 各 1 KiB + digest
- > 256 KiB: prefix/suffix 各 2 KiB + digest,full body 存 `event_blobs`(gzip)

依据(中置信): assistant 消息体积分布与 tool_result 类似,不压缩会让 captured_events 行膨胀,影响 FTS 性能。压缩对 promotion 影响小(promotion 看 evidence_event_ids,需要时再 join `event_blobs`)。

### D2. vector index 是否 attach 到 observations

**推荐**: **否**。

依据:
- observations 是中间产物,生命周期由 promotion 决定,会被 stale/superseded
- vector 索引维护成本高(re-embedding 在每次 stale 时被迫触发)
- §11.1 retrieval substrate 表已经把 "Debug timeline" 放在 observations + captured event provenance 两层,不需要额外 vector
- 只让 vector attach 到 `memories`(stable,生命周期长)

### D4. rule_candidates 输出 patch 还是 plain text

**推荐**: **plain text 优先,patch 作为可选附件**。

schema 微调(§6.12):
```sql
ALTER TABLE rule_candidates ADD COLUMN proposed_diff TEXT;  -- 可空,unified diff
```

依据:
- 多数 rule 改动是新增段落或行内措辞,不是结构性 patch
- patch 模式要求 candidate 与 AGENTS.md/CLAUDE.md 当前版本绑定,review 期间原文档可能已被人手改,apply 失败率高
- plain text + 路径提示足够 review;若 approve,在 approve 阶段生成 patch

### D3 / D5(不阻塞 Phase 1,推迟到 Phase 4 / import 工具)

- D3 global memory 是否强制 review: 推荐**是**,但延后到 Phase 4 实现时再写死规则
- D5 legacy import 是否重放 raw transcripts: 推荐**否**(默认),提供 `--replay-transcripts` 显式开关

---

## 5. Milestone 拆分(确认版)

| Milestone | 含 v2 Phase | MVP 验收 | 用户可见 | 回滚 |
|---|---|---|---|---|
| **A** 基础设施 | Phase 0 + 1 + 6 子集 | 12 表 CREATE 通过;`admin backup/reset-v2/import` 可用;`~/.remem/v2.sqlite` 写入;legacy schema read-only 容许 | `remem status` 多一行 schema 版本 | 删 `~/.remem/v2.sqlite`,旧 DB 不动 |
| **B** 捕获+队列+提炼 | Phase 2 + 3 + 4 | Codex Bash session → captured_events 增长 + extraction_tasks 收敛 + ≥1 memory_candidate | `remem review list/approve/discard` | 停 v2 worker,恢复 install 前 hook/config backup;旧 DB/runtime 不由 v2 binary 兼容 |
| **C** 检索+上下文+Daemon | Phase 5 + 6 余下 | curated-first search;daemon 24h heartbeat 不丢 | MCP `search` 含 `raw_hits` 标签;`install --worker-daemon` | 卸载 LaunchAgent,恢复 Stop fallback 或恢复 install 前 hook/config backup |

A 开工前必须关闭 §17 D1/D2/D4(本文件 §4 已给答案)。B/C 之间用户可任意停留。

---

## 6. 来源锚点

- Claude Code hooks: <https://code.claude.com/docs/en/hooks> (v2.1.121, 2026-04-28)
- Codex hook engine 源码: <https://github.com/openai/codex> `codex-rs/hooks/` (rust-v0.129.0, 2026-05-07)
- Codex hook 配置: <https://developers.openai.com/codex/config-reference>, PR #18893 / #18888
- remem 5/5 落地证据: file:line 引用见 M4 表格
- v2 SPEC: `SPEC-memory-system-v2-no-compat-2026-05-08.md`
- 5/5 SPEC: `SPEC-observation-drain-scheduler-2026-05-05.md`(supersede 关系不变,但 v2.1 承认 80%+ 代码复用)
