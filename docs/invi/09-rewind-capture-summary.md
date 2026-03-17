---
source: remem.save_memory
saved_at: 2026-03-16T04:09:56.620115+00:00
project: remem
---

# Rewind AI 全量记忆捕获机制调研

完成 Rewind AI 深度调研，输出到 docs/invi/09-rewind-capture.md（413 行，14KB）。

核心发现：
1. **捕获机制**：~1 FPS 屏幕录制（ScreenCaptureKit）+ OCR（Apple Vision）+ 音频转录（Whisper 批量处理）
2. **存储优化**：3,750x 压缩率（帧间去重），用户平均 14-30 GB/月
3. **隐私优先**：100% 本地存储，仅 Ask Rewind 功能上传文本数据
4. **检索策略**：全文搜索（SQLite FTS5）+ 向量搜索 + LLM 重排序
5. **商业困境**：2024 年转向硬件（Limitless Pendant），2025 年被 Meta 收购后停止维护

对 remem 的关键启示：
- **不要盲目模仿全量捕获**：remem 不需要屏幕录制，应聚焦对话历史、代码变更、工具调用
- **学习本地优先理念**：默认本地存储（SQLite），可选云端同步
- **借鉴混合检索**：全文搜索 + 向量搜索 + LLM 生成
- **避免商业化陷阱**：开源项目，专注用户价值而非变现
- **必须自动捕获**：不能依赖 Claude 主动调用 save_memory

技术选型建议：
- 存储：SQLite + FTS5 + Qdrant（向量）
- 捕获：MCP 协议 hook + Git hooks + 文件监控
- 检索：三阶段（全文 → 向量 → LLM）
- 隐私：敏感信息检测 + 排除列表 + SQLCipher 加密

开源替代 Screenpipe 的架构值得参考：Rust + SQLite + REST API + 事件驱动。
