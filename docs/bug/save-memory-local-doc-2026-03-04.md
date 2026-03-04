# save_memory 误替代“本地文档保存”问题（2026-03-04）

## 背景

在 Claude Code 会话中，用户要求“把问题记录下来/保存文档”时，agent 可能调用 `remem.save_memory` 工具并返回 `"status":"saved"`，但没有在本地仓库产生文档文件。

这会导致：
- 用户以为“文档已保存”，实际只写入了 memory DB
- 问题复盘与团队协作缺少可见的本地文档
- 语义上把“长期记忆”与“文档落盘”混为一谈

## 根因

1. 工具语义不完整：`save_memory` 仅写 SQLite 观察记录，无本地文件副本。
2. 提示语义偏差：MCP 指令强调“Use save_memory”但没有明确“保存文档要先写本地文件”。
3. 回执不可验证：返回结果只有 memory `id`，没有本地路径信息。

## 设计目标

1. 用户说“保存文档”时，不允许只进 memory。
2. 即使调用 `save_memory`，也至少有一份本地文档。
3. 调用结果要可验证（能看到本地路径）。

## 方案

### 1) save_memory 默认双写

- 写入 memory（SQLite observations）
- 同时写入本地 Markdown 备份

默认路径：

`~/.remem/manual-notes/<project>/<timestamp>-<title>.md`

可通过参数覆盖：

- `local_path`：指定本地落盘路径

### 2) 指令层约束

在 MCP instructions 中新增规则：

- 用户要求“save/write/update document”时，先创建或更新本地文件
- `save_memory` 仅作为长期记忆备份，不替代项目文档

### 3) 可观测回执

`save_memory` 返回增加：

- `local_status`：`saved` / `disabled`
- `local_path`：本地文件路径（若启用本地备份）

## 配置项

- `REMEM_SAVE_MEMORY_LOCAL_COPY`（默认 `true`）：是否启用本地备份
- `REMEM_SAVE_MEMORY_LOCAL_DIR`（默认 `~/.remem/manual-notes`）：本地备份根目录

## 预期效果

1. “保存文档”不再出现“看似成功但没有本地文件”的假阳性。
2. memory 与本地文档形成双轨：一份用于检索记忆，一份用于协作与审计。
3. agent 行为更符合用户语义预期，减少反复纠偏成本。
