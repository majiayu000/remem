# remem Codebase Audit Report

> Date: 2026-07-09
> Target: local remem worktree (exact path withheld; crate `remem-ai` v0.5.21)
> Stack: Rust 2021 · clap CLI · rmcp MCP · axum API · rusqlite/sqlcipher · Node Codex plugin
> Mode: **full** · Agents: 7 (contract / dataflow / security / architecture / config / tests / concurrency)
> Previous audit: [docs/audit-2026-05-29.md](docs/audit-2026-05-29.md)
> Method: 7 parallel read-only agents + main-session spot verification of Critical/High

> Historical snapshot: this report's original source commit, branch, and dirty
> state were not recorded. It is not a current-main defect list. The reconciliation
> below compares it with `origin/main@6e4734ccbeb11b279db1528c99099a6361facda0`
> on 2026-07-16. Machine-readable provenance and per-finding evidence live in
> `.audit/findings.json`.

## 2026-07-16 Current-Main Reconciliation

The Critical/High ledger contains 24 historical findings: 16 `resolved`, 6
`refuted`, 0 `new`, 1 `duplicate`, and 1 `security_private`. Every entry has one
status, a last-verified commit, and a resolution or tracking reference. The
security-private entry intentionally contains no affected path, reproduction,
deployment detail, or payload; its only public routing reference is
`GHSA-rc4v-c467-85j4`.

Medium findings are intentionally outside the JSON ledger. They remain below as
unverified historical observations, not current defects. Adding Medium to the
ledger requires a separate current-main verification pass.

| Finding | Severity | Current status | Evidence / route |
| --- | --- | --- | --- |
| `dataflow--summary-promote-dead-gate` | critical | `resolved` | PR #697 |
| `contract--pending-vs-pending-review` | critical | `resolved` | PR #585 |
| `config--encrypt-plaintext-bak` | critical | `resolved` | PR #519 |
| `concurrency--summarize-lock-non-cas` | critical | `refuted` | `specs/GH684/tasks.md#sp684-t7` |
| `arch--dual-capture-and-promote` | critical | `duplicate` | GH-684 |
| `errors--context-memory-load-silent` | high | `resolved` | PR #444 |
| `errors--capture-ledger-warn-continue` | high | `resolved` | PR #435 |
| `security-private-001` | high | `security_private` | `GHSA-rc4v-c467-85j4` |
| `dataflow--observe-silent-skip` | high | `resolved` | PR #422 |
| `dataflow--observation-type-coerce-discovery` | high | `resolved` | PR #838 |
| `arch--unimplemented-extraction-kinds` | high | `refuted` | GH-820 calibration |
| `config--empty-release-assets` | high | `refuted` | GH-820 calibration |
| `config--encrypt-key-exists-skip` | high | `resolved` | PR #559 |
| `arch--dual-identity-models` | high | `refuted` | GH-820 calibration |
| `arch--host-registry-quad` | high | `refuted` | GH-820 calibration |
| `concurrency--mark-job-done-no-rows-check` | high | `resolved` | PR #843 |
| `concurrency--summary-job-skip-marks-done` | high | `refuted` | `specs/GH684/tasks.md#sp684-t7` |
| `concurrency--enqueue-job-race` | high | `resolved` | PR #843 |
| `tests--fts-fixture-active-only` | high | `resolved` | PR #842 |
| `crypto--sqlcipher-silent-plaintext-open` | critical | `resolved` | `src/db/crypto.rs` at compared main |
| `dataflow--fts-stale-blackhole` | critical | `resolved` | PR #279 |
| `dataflow--raw-archive-cross-session-dedup` | critical | `resolved` | PR #279 |
| `arch--dual-schema-sqlite-lineage` | critical | `resolved` | `src/db` at compared main |
| `security--rust-api-unauthenticated` | high | `resolved` | `src/api/server.rs` at compared main |

The remainder of this document is the original 2026-07-09 analysis, retained
for provenance. Its open/new wording and roadmap are historical; the table
above and `.audit/findings.json` are authoritative for current status.

Findings separate **Fact** (file:line) / **Inference** / **Suggestion**. Critical/High below were spot-verified against live code unless marked `unverified`.

---

## Summary

| Level | Count | Verified | Key areas |
|-------|------:|---------:|-----------|
| **Critical** | 5 | 5 | Dual memory producers + review queue blackhole; API `pending` vocab; encrypt plaintext `.bak`; summarize lock race; dual capture stores |
| **High / P1** | 14 | 12 | Context load silent empty; capture ledger warn+continue; one security-private finding; empty runtime assets; identity dual model; host registries; job lease; test fixture FTS drift |
| **Medium / P2** | 18 | 0 (unverified) | Field-name triad; version sync; god modules; unbounded queues; extension cost; ARCHITECTURE drift |

**Cross-agent confirmation (raises confidence):**
- **Dual capture / dual promote pipelines** — dataflow + architecture + config agents
- **Stop-hook capture ledger warn+continue** — dataflow + security + config
- **Summary candidates never auto-promote** — dataflow (summary thresholds + `auto_promote_batch=None`)
- **API `pending` vs writer `pending_review`** — contract agent, verified against stats + candidates handlers
- **Job lease zero-row silent success** — concurrency + config

---

## Delta vs Previous Audit (2026-05-29)

| Status | Items |
|--------|-------|
| **Resolved** | C1 dual `schema.sqlite` lineage; C2 fail-open plaintext open (now `REMEM_ALLOW_PLAINTEXT_DB`); C3 FTS active-only blackhole (`v020`); C4 raw archive cross-session dedup (`v021`); C5 architecture auto-promote type map (supports_observation_type + tests); Rust API unauthenticated (Bearer middleware) |
| **Still-open / residual** | Silent context-load → empty UI (H1); dual capture pipeline debt; observation type silent coerce to `discovery`; encrypt/backup cipher edge cases; memory-type vocabulary drift at product boundary |
| **New** | Summary promote hard-gates to `pending_review` forever; REST `pending` status blackhole; encrypt leaves plaintext `.bak`; summarize lock non-CAS + no cooldown pre-claim; empty plugin release `assets{}`; one security-private item; FTS test fixture still active-only |

---

## Critical (Fix Immediately)

### C1 — Stop/summary 路径几乎无法自动变成 active memory（设计性死门）
**Severity: Critical** · verified · agents: dataflow
**Fact**

Summary → candidate 路径硬编码：

```16:17:src/memory/promote/summary.rs
const SUMMARY_CANDIDATE_CONFIDENCE: f64 = 0.74;
const SUMMARY_CANDIDATE_RISK: &str = "medium";
```

且 `persist_summary_candidates` 传入 `auto_promote_batch: None`：

```287:310:src/memory_candidate.rs
pub(crate) fn persist_summary_candidates(...) -> Result<CandidatePersistSummary> {
    ...
    persist_candidate_rows(..., None)
}
```

而 auto-promote 要求 `risk_class == "low"`、`confidence >= 0.80`、且有 source `ObservationBatch`：

```505:519:src/memory_candidate.rs
fn should_auto_promote(...) -> bool {
    candidate.scope == "project"
        && candidate.risk_class == "low"
        && candidate.confidence >= AUTO_PROMOTE_MIN_CONFIDENCE
        ...
        && is_supported_by_source_observation(candidate, batch)
}
```

**Inference (高):** SessionStart / search 读 active `memories`，不读 `pending_review`。Stop 后 summary 只堆 review 队列 → 用户侧像“装了但没记忆”。这与项目记录的失败模式 #2（review 队列无人 drain）同构。

**Suggestion:** 二选一做清楚：
1. 给 summary 路径单独的 promote 策略（明确 evidence bridge + 阈值/风险），或
2. 坚持进 review，但 SessionStart/doctor/`remem status` **必须**把 `pending_review` 当成用户可见错误/待办，禁止静默空上下文。

---

### C2 — REST `/stats` 与 `/candidates` 用 `pending`，写入用 `pending_review`
**Severity: Critical** · verified · agents: contract
**Fact**

写入：

```360:360:src/memory_candidate.rs
let review_status = "pending_review";
```

系统 stats 正确：

```133:133:src/db/query/stats.rs
"SELECT COUNT(*) FROM memory_candidates WHERE review_status = 'pending_review'"
```

REST 错误：

```26:28:src/api/handlers/stats.rs
let pending_candidates: i64 = conn.query_row(
    "SELECT COUNT(*) FROM memory_candidates WHERE review_status = 'pending'",
```

```21:25:src/api/handlers/candidates.rs
let status = params
    .status
    ...
    .unwrap_or("pending");
```

**Impact:** 插件/REST 默认列表与 stats 对真实队列恒为 0；与 C1 叠加后，review 队列既不自动 promote 也**看不见**。

**Suggestion:** 全栈统一 `pending_review`（或共享 enum）；未知 status 拒绝；回归测试：插入 `pending_review` 后 `/stats` 与 `/candidates` 非零。

---

### C3 — `remem encrypt` 永久留下明文 `remem.db.bak`
**Severity: Critical** · verified · agents: config
**Fact**

```203:210:src/db/crypto.rs
let backup_path = db_file.with_extension("db.bak");
std::fs::rename(&db_file, &backup_path)?;
std::fs::rename(&encrypted_path, &db_file)?;
// logs backup path; no delete/shred
```

**Impact:** 加密 UX 暗示 at-rest 保护，但同目录明文备份完整保留 prompts/code/decisions。

**Suggestion:** 验证加密库可读后默认 shred/删除 `.bak`；若需保留，要求 `--keep-plaintext-backup`。

---

### C4 — Summarize 锁非 CAS，且 cooldown 未 pre-claim（与 ARCHITECTURE 文档不符）
**Severity: Critical** · verified · agents: concurrency
**Fact**

```38:65:src/db/summarize/cooldown.rs
// SELECT lock_epoch → if expired → INSERT ... ON CONFLICT DO UPDATE
// 无 atomic WHERE lock_epoch IS NULL OR lock_epoch < cutoff + changes()==1
```

ARCHITECTURE 写 “Pre-claim | Record cooldown before AI call”；实现是 **AI 成功后** 才写 cooldown，且 lock 可被并发 worker 同时“拿到”。

**Impact:** 同项目并行 summarize → 重复 AI 成本、summary/candidate 竞态；文档承诺的 3-gate 失效。

**Suggestion:** `BEGIN IMMEDIATE` + CAS 条件更新；同一事务 pre-write cooldown/processing claim；失败路径再释放。

---

### C5 — 双 capture 写入 + 双 durable-memory 生产者（架构债已成产品风险）
**Severity: Critical** · verified · agents: architecture + dataflow
**Fact**

- Observe 双写：`record_capture_event` + legacy `insert_event`（`src/observe/hook.rs`）
- Worker 跳过 legacy `JobType::Observation` 却 `mark_job_done`（`src/worker.rs`）
- Durable memory：Stop→Summary→`promote_summary_to_memory_candidates` **与** capture→observation_extract→memory_candidate **并行**
- `session_summaries` 两系（legacy `session_row_id IS NULL` vs rollup）靠过滤共存（`docs/ARCHITECTURE.md`）

**Impact:** 状态分裂、doctor/status 难解释、任一路径失败时另一路径可能“看起来在工作”但 active memory 仍空（叠加 C1/C2）。

**Suggestion:** 选一个 canonical promote 路径；另一路降级为 archive/diagnostic。删除或迁移 `pending_observations` / `JobType::Observation` 双写。

---

## High / P1 (This Week)

### H1 — Context 主通道吞掉 memory 查询错误 → “No previous sessions”
**verified** · `src/context/query.rs:115-120` + `src/context/render.rs` empty path
DB 错误 log 后 `Vec::new()`，`errors` 不记 → 渲染走 empty state。违反 U-29。

### H2 — SessionStop capture ledger 失败仅 warn，仍 enqueue summary
**verified** · `src/summarize/summary_job/hook.rs:119-127`
ledger 未写入时 legacy jobs 继续 → 双管道进一步分叉。

### H3 — Security-private finding
Public details withheld. Current routing status: `security_private`; private
tracking reference: `GHSA-rc4v-c467-85j4`.

### H4 — observe 无 adapter / classify None 静默 `Ok(())`
**verified** · `src/observe/hook.rs`
对比 session-init 有 SKIP 日志。捕获入口丢事件无声。

### H5 — Observation 解析未知 type 静默改写为 `discovery`
**verified** · `src/memory/format/parse.rs`
证据类型被改写 → auto-promote support 判断失真（C5 residual）。

### H6 — `ExtractionTaskKind::RuleCandidate` / `IndexUpdate` 已注册未实现，defer 后耗尽 attempts 会跳过 watermark 区间
**verified** · `src/extraction_worker.rs:133` + `exhaust_extraction_task`
未实现 kind 不应可入队。

### H7 — Plugin release manifest `assets: {}` 空
**verified** · `plugins/remem/runtimes/remem-releases.json` v0.5.21
`downloadRuntime` 无法拉到匹配 binary。

### H8 — `remem encrypt` 仅看 `.key` 存在就认为已加密
**verified** · `src/cli/actions/maintenance.rs`
崩溃在 key 写出、encrypt 未完成时永久卡在“已加密”。

### H9 — 双 identity 模型（string `project` vs normalized hosts/projects/sessions）
**verified** · migrations v001 + v006
扩展与查询必须双路径维护。

### H10 — Host 注册四处独立（adapter / install / context / capture allowlist）
**verified** · architecture agent
加 host 易 partial-wire。

### H11 — Job `mark_job_done` 不检查 rows-affected（lease 丢失仍“成功”）
**verified** · `src/db/job/state.rs`
与 extraction `ensure_task_updated` 不一致。

### H12 — Summary job 在 cooldown/lock 时 `Ok(())` → worker mark done，永不重试
**verified** · `src/summarize/summary_job/process.rs`
锁竞争导致摘要永久丢失。

### H13 — enqueue_job SELECT-then-INSERT 竞态可重复 inflight jobs
**verified** · `src/db/job/enqueue.rs`

### H14 — 测试 fixture FTS 仍 active-only（与生产 v020 漂移）
**verified** · `src/memory/types.rs` setup triggers
大量 FTS 测试在测旧 bug 行为。

---

## Medium / P2 (Plan) — labeled `unverified` unless noted

| ID | Summary | Primary locus |
|----|---------|---------------|
| M1 | Search 三套 envelope（REST/MCP/CLI）字段名 triad：`content`/`text`/`preview` | api/types, mcp, memory types |
| M2 | MemoryType 8 种 vs UI/MCP save 文档 5 种 | widget, mcp SaveMemoryParams |
| M3 | 三套 status JSON（REST flat / CLI nested / stats） | api status vs CLI vs widget |
| M4 | Dual MCP 工具面：Rust tools vs Node `remem_*` | plugins tools.js |
| M5 | npm `0.4.5` 未进 version-sync gate（Cargo/plugin 0.5.21） | npm/remem, scripts/ci |
| M6 | ARCHITECTURE Module Overview 严重过时（~9k LOC、已删模块） | docs/ARCHITECTURE.md |
| M7 | graph_candidate ~684 / memory_candidate ~648 / observation_extract ~596 偏大 | src/ |
| M8 | 扩展成本高：新 task kind / MemoryType / host 约 6–12 文件 | extension-cost |
| M9 | Vector search 有 state_key current 过滤，FTS/LIKE 无（C3 residual） | retrieval/ |
| M10 | LIKE 不搜 `search_context` | like.rs |
| M11 | capture_events / event_blobs / raw_messages 无 cleanup TTL | maintenance |
| M12 | Context gate 优先级 CLI>config>env 非常规 | invocation.rs |
| M13 | HOME 缺失时 data_dir → `./.remem` | db/core.rs |
| M14 | 迁移 mark_applied `INSERT OR IGNORE` | migrate/state.rs |
| M15 | Runtime download 任意 redirect | remem-runtime.js |
| M16 | Worker/API 异步路径上同步 open_db/migrate | worker, api |
| M17 | 无界 pending/jobs；plugin 并发无 semaphore | pending, server.js |
| M18 | encrypt 中途失败可留 `.db.enc` 临时文件 | crypto.rs |

---

## Refuted / Downgraded

| Claim | Verdict |
|-------|---------|
| ObservationExtract `Done{to_event_id:None}` 总是错误地跳过证据 | **部分反驳**：`mark_extraction_task_done` 用 `to_event_id.or(high_watermark)`；Written 前已 enqueue MemoryCandidate。真正问题是结果类型被丢弃、无法区分 Empty/NoObs/Written 的可观测性，以及 SessionRollup 同样无 coverage 语义。Severity 从 critical 下调到 high/medium 可观测性债。 |
| 2026-05-29 C1–C5 仍全部 Critical | **已修复主体**；仅 residual 进入本报告 |

---

## Repair Roadmap

| Phase | Scope | Est. files | Outcome |
|------:|-------|------------|---------|
| **0** | C2 status vocab + REST/CLI/plugin 对齐；H14 fixture 对齐 v020 | 4–6 | 可见队列、测试反映生产 |
| **1** | C1 summary promote 策略 或 review  surfacing；H1 context errors；H2 fail-closed capture | 6–10 | 结束“空记忆当成功” |
| **2** | C3 encrypt.bak；H8 encrypt 状态探测；backup wrong-key | 3–5 | 加密承诺可兑现 |
| **3** | C4 summarize CAS+preclaim；H11–H13 job lease/enqueue；H12 lock requeue | 5–8 | 并发正确性 |
| **4** | C5 收敛 dual pipeline（选 canonical promote）；H6 删未实现 kinds；H9–H10 identity/host registry | 15–30 | 架构单一真源 |
| **5** | H3 private security remediation；H7 assets 填充；M5 version sync；M1–M4 契约统一 | 8–12 | 产品面一致 |
| **6** | Medium 清理：TTL、docs、拆 god modules、search channel 对称 | ongoing | 可维护性 |

---

## Agent Coverage

| # | Dimension | Agent type | Top signal |
|---|-----------|------------|------------|
| 1 | Frontend-Backend / API Contract | general-purpose | C2 pending vocab, multi-envelope |
| 2 | Data Integrity & Flow | general-purpose | C1 summary dead promote, dual pipeline |
| 3 | Error Handling & Security | security-reviewer | H1 context silent, H3 private finding |
| 4 | Architecture & Quality | architect | C5 dual systems, registries |
| 5 | Config & Persistence | database-reviewer | C3 encrypt.bak, H7 empty assets |
| 6 | Test Quality | general-purpose | H14 FTS fixture drift |
| 7 | Concurrency | general-purpose | C4 lock race, job lease |

---

## Notes

- 本报告只读审计，未改业务代码；产出文件：`audit-report-2026-07-09.md`、`.audit/findings.json`。
- 建议将 `.audit/` 加入 `.gitignore`（未自动改 gitignore）。
- 与 `docs/specs/SPEC-audit-remediation-2026-05-29.md` 关系：上次 remediation 大部分落地；本次是 **新一轮** 以双管道/契约/并发为主的设计债。
