# Spec: autoDream — 记忆合并与剪枝

**状态**: Draft
**日期**: 2026-04-07
**灵感来源**: Claude Code 源码泄露中发现的 autoDream 后台记忆整理机制

---

## 1. 背景与动机

remem 当前的记忆写入逻辑（`memory/store/write.rs`）已经对 `topic_key` 做了 upsert——同一 `(project, topic_key)` 写入时直接覆盖。但以下情况仍会导致记忆库膨胀和质量下降：

| 问题 | 原因 |
|------|------|
| 无 topic_key 的记忆无限堆积 | `topic_key = NULL` 时每次都插入新行 |
| 语义相近但 topic_key 不同的记忆共存 | 不同会话对同一话题用了不同 key |
| 旧记忆内容被新决策推翻但未清理 | 后续更新覆盖了 topic_key，旧行仍在 |
| `preference` 类记忆版本混乱 | 同一偏好被多次以不同措辞写入 |

autoDream 是一个**异步后台 job**，定期扫描记忆库，用 LLM 完成语义去重、内容合并、矛盾消解，保持记忆库精简准确。

---

## 2. 目标

- **合并**：同一 project 内语义相近的记忆 → 合并为一条
- **去重**：无 topic_key 的重复内容 → 识别后保留最新、标 stale 旧的
- **消解矛盾**：同一话题内容互相矛盾 → 保留更新的，在内容中注明演进
- **不改变接口**：现有 MCP tools、CLI 命令全部不变

**不做**：
- 不压缩 observations（已有 `Compress` job）
- 不处理跨 project 的记忆
- 不自动删除，只标 stale（保留可审计性）

---

## 3. 数据流

```
触发（CLI / worker 空闲）
        ↓
load_dream_candidates(project)
  → 查 memories WHERE project=? AND status='active'
  → 按 memory_type 分组（preference / decision / discovery / ...）
        ↓
cluster_by_topic(memories)
  → 同 topic_key prefix 归一组（topic_key IS NULL 单独处理）
  → 每组 ≥ 2 条才进入下一步
        ↓
for each cluster:
  call_ai(DREAM_PROMPT, cluster_contents)
  → 输出：合并后的 title + content + topic_key + 被替换的 id 列表
        ↓
apply_dream_result(conn, result)
  → upsert 合并记忆（insert_memory_full with topic_key）
  → mark_memories_stale(old_ids)
        ↓
log_dream_summary(project, merged_count, stale_count)
```

---

## 4. 新增文件与改动

### 4.1 新文件：`src/dream.rs`（主入口）
```
src/dream.rs
src/dream/
  candidates.rs   — 查询候选记忆、按 topic 聚类
  merge.rs        — LLM 调用 + 结果解析
  apply.rs        — 写回合并结果、标 stale
  constants.rs    — DREAM_PROMPT、阈值常量
  tests.rs
```

### 4.2 改动：`src/db_models.rs`
新增 `JobType::Dream`：
```rust
pub enum JobType {
    Observation,
    Summary,
    Compress,
    Dream,      // ← 新增
}
```
`as_str()` → `"dream"`
`from_db()` → `"dream" => Ok(Self::Dream)`

### 4.3 改动：`src/worker.rs`
```rust
db::JobType::Dream => dream::process_dream_job(&job.project).await,
```

### 4.4 改动：`src/cli/actions/maintenance.rs`
新增 `run_dream(project: Option<&str>)` —— 手动触发入口：
```
remem dream              # 对所有 project 运行
remem dream --project X  # 只跑指定 project
```

### 4.5 改动：`src/cli/dispatch.rs` + `src/cli/types.rs`
新增 `dream` 子命令。

---

## 5. 核心常量（`dream/constants.rs`）

```rust
/// 每个 project 每次 dream 处理的最大 cluster 数（防止单次过贵）
pub const DREAM_MAX_CLUSTERS: usize = 30;

/// cluster 内记忆数下限（少于这个数不值得合并）
pub const DREAM_MIN_CLUSTER_SIZE: usize = 2;

/// 同 topic_key prefix 匹配长度（前 N 个字符视为同组）
pub const TOPIC_KEY_PREFIX_LEN: usize = 20;

/// dream job 最小间隔（秒），防止重复触发
pub const DREAM_COOLDOWN_SECS: i64 = 3600;  // 1 小时
```

---

## 6. 聚类策略（`dream/candidates.rs`）

**Step 1**：查活跃记忆
```sql
SELECT id, topic_key, title, content, memory_type, updated_at_epoch
FROM memories
WHERE project = ?1 AND status = 'active'
ORDER BY memory_type, topic_key, updated_at_epoch DESC
```

**Step 2**：分组规则（纯内存操作，不用 LLM）
- `topic_key` 相同 → 同组（正常情况不应出现，upsert 已处理）
- `topic_key` 前 20 字符相同 → 同组
  - 例：`auth-middleware-design-v1` 和 `auth-middleware-design-v2` → 同组
- `topic_key IS NULL` → 按 `memory_type` 各自单独分组，每组最多 `DREAM_MAX_CLUSTERS` 条
- 组内仅 1 条 → 跳过

**Step 3**：过滤太新的（最近 1 小时写入的不参与，避免当前会话未完成就被合并）
```rust
let cutoff = Utc::now().timestamp() - 3600;
candidates.retain(|m| m.updated_at_epoch < cutoff);
```

---

## 7. LLM Prompt（`dream/constants.rs`）

```
DREAM_PROMPT = """
You are a memory consolidation assistant. Given multiple related memory entries,
merge them into a single, accurate, non-redundant memory.

Rules:
1. Keep the MOST RECENT factual information when there are contradictions.
2. Note important historical changes with "Previously: ..." if relevant.
3. Output exactly ONE merged memory in this XML format:
   <memory>
   <topic_key>kebab-case-stable-key</topic_key>
   <type>decision|discovery|preference|bugfix|architecture</type>
   <title>Concise title (max 80 chars)</title>
   <content>Full merged content in markdown</content>
   <supersedes>comma-separated list of input IDs that are now stale</supersedes>
   </memory>
4. If entries are NOT actually related and should remain separate, output:
   <no_merge reason="brief explanation"/>
"""
```

---

## 8. 结果应用（`dream/apply.rs`）

```rust
pub struct DreamResult {
    pub topic_key: String,
    pub memory_type: String,
    pub title: String,
    pub content: String,
    pub superseded_ids: Vec<i64>,
}

pub fn apply_dream_result(conn: &Connection, project: &str, result: &DreamResult) -> Result<()> {
    // 1. upsert 合并后的记忆（复用现有 insert_memory_full）
    memory::store::write::insert_memory_full(
        conn,
        Some("dream"),
        project,
        Some(&result.topic_key),
        &result.title,
        &result.content,
        &result.memory_type,
        None,   // files
        None,   // branch
        "project",
        None,
    )?;

    // 2. 标 stale 旧记忆
    for id in &result.superseded_ids {
        conn.execute(
            "UPDATE memories SET status = 'stale' WHERE id = ?1",
            params![id],
        )?;
    }
    Ok(())
}
```

---

## 9. 触发方式

### 手动（Phase 1，先实现）
```bash
remem dream                  # 所有 project
remem dream --project myapp  # 指定 project
remem dream --dry-run        # 只输出计划，不写入
```

### 自动（Phase 2，后续）
在 `worker.rs` 的 idle 分支里加计时器：
```rust
// worker 空闲超过 30 分钟 → 入队 dream job（有 cooldown 保护）
if idle_streak_secs > 1800 {
    for project in db::get_active_projects(&conn)? {
        db_job::maybe_enqueue_dream(&conn, &project)?;
    }
    idle_streak_secs = 0;
}
```
`maybe_enqueue_dream` 检查 cooldown（上次运行 < 1 小时前则跳过）。

---

## 10. 错误处理

- LLM 调用失败 → `log::warn` + 跳过该 cluster，不中断整个 job（与 compress 一致）
- `<no_merge>` 响应 → 正常情况，直接跳过，不算错误
- 解析失败 → `log::warn` + 跳过
- **禁止**：任何单个 cluster 失败不得 panic 或中断其他 cluster 处理

---

## 11. 测试计划

| 测试 | 位置 | 覆盖点 |
|------|------|--------|
| `test_cluster_by_topic_key_prefix` | `candidates.rs` | 前缀分组逻辑 |
| `test_cluster_null_topic_key` | `candidates.rs` | NULL key 分组 |
| `test_dream_result_parse` | `merge.rs` | XML 解析正确 |
| `test_no_merge_response` | `merge.rs` | `<no_merge>` 不写入 |
| `test_apply_marks_stale` | `apply.rs` | 旧记忆变 stale |
| `test_apply_upserts_merged` | `apply.rs` | 合并记忆写入 DB |
| `test_dream_cooldown` | `db_job/enqueue.rs` | 1 小时内不重复入队 |

---

## 12. 实现顺序

1. `db_models.rs` — 新增 `JobType::Dream`（5 行）
2. `dream/candidates.rs` — 查询 + 聚类（~60 行）
3. `dream/constants.rs` — prompt + 阈值（~30 行）
4. `dream/merge.rs` — LLM 调用 + XML 解析（~70 行）
5. `dream/apply.rs` — 写回 + 标 stale（~40 行）
6. `dream.rs` — 入口 `process_dream_job`（~30 行）
7. `worker.rs` — 接线（5 行）
8. `cli/` — `dream` 子命令（~30 行）
9. 测试（~100 行）

**总计约 370 行**，分布在 8 个文件。

---

## 13. Done 条件

- [ ] `remem dream --dry-run` 输出哪些 cluster 会被合并，不写入
- [ ] `remem dream` 运行后，同 topic_key prefix 的多条记忆合并为 1 条
- [ ] 被合并的旧记忆 status = 'stale'
- [ ] LLM 返回 `<no_merge>` 时不写入、不报错
- [ ] LLM 调用失败时只 warn、跳过该 cluster、继续处理其他
- [ ] `cargo test` 全部通过
- [ ] dream job 在 worker 中正确处理（enqueue / claim / done / retry）
