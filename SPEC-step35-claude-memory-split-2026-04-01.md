# Step 35 - Split claude_memory module

## Why

`src/claude_memory.rs` 当前同时包含 Claude memory 路径编码、session summary 查询、Markdown 内容拼接、索引文件维护和同步运行逻辑。虽然文件还没有超过 200 行很多，但它已经是当前剩余最重的核心逻辑文件之一，继续拆开后可以把路径、渲染、索引维护和同步流程分层，后续调整 Claude 原生 memory 输出时更容易控风险。

本步只做结构拆分，不改变 `sync_to_claude_memory()` 的公开接口，也不改变 native Claude memory 的文件名、内容结构、decision 去重和 MEMORY 索引更新语义。

## Scope

- 保持 `pub fn sync_to_claude_memory(cwd, project)` 对外接口不变
- 将 `src/claude_memory.rs` 拆为 `paths`、`render`、`index`、`runtime`、`tests` 子模块
- 保持 `REMEM_FILE=remem_sessions.md`、Recent Sessions / Key Decisions 的内容结构不变
- 保持 decision 去重逻辑和 `MEMORY.md` pointer 更新语义不变
- 新增最小回归测试，锁住路径编码和 MEMORY 索引写入行为

## Module layout

- `src/claude_memory.rs`
  - 模块声明与 `pub use`
- `src/claude_memory/paths.rs`
  - `encode_project_path`
  - `claude_memory_dir`
- `src/claude_memory/render.rs`
  - `SessionRow`
  - 内容渲染 helper
  - `format_date`
- `src/claude_memory/index.rs`
  - `ensure_memory_index`
- `src/claude_memory/runtime.rs`
  - `sync_to_claude_memory`
  - session summary 查询 helper
- `src/claude_memory/tests.rs`
  - 路径编码 / 索引维护回归测试

## Public interface invariants

- `sync_to_claude_memory()` 继续在 Claude memory 目录不存在时直接 skip 并记录 info
- `sync_to_claude_memory()` 继续只同步最近 `MAX_SESSIONS` 会话和最多 `MAX_DECISIONS` 条决策
- `ensure_memory_index()` 继续在已有 `MEMORY.md` 中缺少 remem pointer 时追加 `## Auto` 区块
- `ensure_memory_index()` 继续在 pointer 已存在时保持幂等，不重复追加
- 文件输出名继续为 `remem_sessions.md`

## Validation

定向测试：
- `cargo test encode_project_path_replaces_slashes_after_canonicalize -- --nocapture`
- `cargo test ensure_memory_index_is_idempotent -- --nocapture`
- `cargo test ensure_memory_index_creates_new_file_when_missing -- --nocapture`

全量验证：
- `cargo check`
- `cargo test`

## Notes

- 测试使用临时目录验证索引文件写入，不接触真实 `~/.claude` 目录。
- 若 `cargo fmt` 再次带出已知无关格式化噪音文件，只恢复这些无行为变化差异，不纳入本批提交。
