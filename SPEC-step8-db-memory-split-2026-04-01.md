## Goal

继续收债，拆分 `src/db.rs` 和 `src/memory.rs`，把当前过大的文件按已有职责平移到子模块中，同时保持公开 API 路径不变。

## Current State

- `src/db.rs` 同时包含：
  - 路径/工具函数
  - SQLCipher 加密
  - DB 打开与 git 元数据
  - summarize 锁与 summary 写入
  - observation 写操作
  - 测试支撑
- `src/memory.rs` 同时包含：
  - Memory/Event 数据结构
  - memory CRUD
  - event CRUD
  - session/event 辅助查询
  - row mapper 与测试 helper

## Split Plan

### db

- `src/db.rs`
  - 只保留 re-export 和模块组织
- `src/db/core.rs`
  - `deterministic_hash`
  - `to_sql_refs`
  - `truncate_str`
  - `canonical_project_path`
  - `project_from_cwd`
  - `data_dir`
  - `db_path`
  - `open_db`
  - `detect_git_branch`
  - `detect_git_commit`
- `src/db/crypto.rs`
  - `generate_cipher_key`
  - `encrypt_database`
  - 内部 key 加载逻辑与测试
- `src/db/summarize.rs`
  - summarize cooldown/lock/finalize/session
- `src/db/observation.rs`
  - observation insert/stale/compress/accessed
- `src/db/test_support.rs`
  - `ScopedTestDataDir`

### memory

- `src/memory.rs`
  - 只保留 re-export 和模块组织
- `src/memory/types.rs`
  - `Memory`
  - `Event`
  - `MEMORY_TYPES`
  - row mapper
  - test schema helper
- `src/memory/store.rs`
  - memory CRUD
  - memory 查询
- `src/memory/events.rs`
  - event CRUD
  - cleanup/archive/session 统计

## Non-Goals

- 不改对外函数名
- 不改 `db::...` / `memory::...` 调用路径
- 不顺手修改业务 SQL

## Verification

- 先跑与 `db` / `memory` 直接相关的单测
- 再跑：
  - `cargo check`
  - `cargo test`
