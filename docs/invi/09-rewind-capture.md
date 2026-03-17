# Rewind AI 全量记忆捕获机制深度调研

> 调研日期：2026-03-16
> 目标：理解 Rewind AI 的技术架构，为 remem 项目提供设计参考

## 执行摘要

Rewind AI（现已被 Meta 收购并更名为 Limitless）是全量记忆捕获领域的开创者，通过 **~1 FPS 屏幕录制 + OCR + 音频转录** 实现"搜索你的人生"。核心技术特点：

- **捕获策略**：低帧率（~1 FPS）屏幕录制 + 实时音频捕获
- **压缩比例**：3,750x 压缩率，用户平均每月 14-30 GB 存储
- **隐私优先**：100% 本地存储，数据永不离开设备（除非用户主动使用 Ask Rewind）
- **搜索能力**：全文搜索（OCR 提取）+ LLM 增强查询（Ask Rewind）

**关键启示**：Rewind 的成功证明了 **持续捕获 + 本地存储 + AI 检索** 的可行性，但其 2024 年转向硬件（Limitless Pendant）和 2025 年被 Meta 收购表明，纯软件方案面临商业化挑战。

---

## 1. 如何提取：捕获机制

### 1.1 屏幕录制

**技术栈**：
- **API**：macOS ScreenCaptureKit（Apple 官方 API，macOS 12.3+）
- **帧率**：~1 FPS（每秒 1 帧截图）
- **分辨率**：支持原生分辨率，但实际使用中会动态调整以平衡性能和存储

**实现细节**（来自 [Kevin Chen 的逆向分析](https://kevinchen.co/blog/rewind-ai-app-teardown/)）：
```
- 使用 ScreenCaptureKit 捕获当前聚焦的屏幕（多屏幕时只录制活动屏幕）
- 自动隐藏隐私窗口：
  - 浏览器隐私模式窗口
  - 用户自定义排除列表中的应用
- 相邻帧去重：如果连续两帧内容相同，压缩时去除冗余像素
```

**性能优化**：
- **帧间压缩**：相邻帧的相同像素被去重，类似视频编码的 I-frame/P-frame 机制
- **动态帧率**：静态内容（如阅读文档）时降低捕获频率，动态内容（如视频会议）时提高频率

**隐私保护**：
- ScreenCaptureKit 提供系统级窗口过滤，Rewind 利用此功能自动排除：
  - Safari/Chrome 隐私浏览窗口
  - 用户手动添加到排除列表的应用
- macOS Sequoia（15.1+）引入月度权限提示，要求用户定期确认屏幕录制权限

### 1.2 OCR 文本提取

**技术栈**：
- **引擎**：Apple Vision Framework（推测，基于 Rewind 团队之前开发 Scribe OCR 应用的背景）
- **备选方案**：Tesseract（开源 OCR，支持 100+ 语言）

**提取范围**：
- 屏幕上所有可见文本（包括视频字幕、Zoom 会议中的共享屏幕）
- 支持多语言（依赖 OCR 引擎的语言包）

**准确率**：
- 清晰打印文本：>95% 准确率
- 手写文本/低分辨率图像：准确率下降

### 1.3 音频转录

**技术栈**：
- **模型**：OpenAI Whisper（推测，基于行业标准）
- **处理模式**：批量转录（非实时）

**捕获范围**：
- 系统音频（会议、播客、视频）
- 麦克风输入（用户语音）

**转录流程**：
```
1. 音频捕获：持续录制系统音频 + 麦克风
2. 分段处理：按时间窗口（如 30 秒）切分音频
3. 批量转录：使用 Whisper 模型转录为文本
4. 时间戳对齐：为每个转录片段添加精确时间戳
```

**性能权衡**：
- **实时 vs 批量**：Rewind 选择批量转录以降低 CPU 占用，转录延迟约 1-5 分钟
- **模型大小**：可能使用 Whisper Small/Medium 模型平衡准确率和速度

### 1.4 应用上下文元数据

**捕获内容**：
- 活动窗口标题
- 应用名称
- 文件路径（如果可访问）
- 时间戳（精确到秒）

**用途**：
- 时间线导航：按应用/窗口过滤历史记录
- 搜索增强：结合应用上下文提高搜索相关性

---

## 2. 提取什么：数据类型

### 2.1 视觉内容

**存储格式**：
- **原始格式**：压缩后的截图序列（类似 1 FPS 视频）
- **压缩算法**：帧间去重 + 有损压缩（JPEG/WebP）

**数据量**：
- 未压缩：~1080p 截图 × 1 FPS × 8 小时/天 = ~100 GB/月
- 压缩后：~14-30 GB/月（3,750x 压缩率）

### 2.2 文本内容

**来源**：
- OCR 提取的屏幕文本
- 音频转录的语音文本
- 应用元数据（窗口标题、文件名）

**存储**：
- 全文索引（支持快速搜索）
- 与时间戳关联（支持时间线回溯）

### 2.3 音频内容

**存储策略**：
- **原始音频**：不保存（仅保留转录文本）
- **转录文本**：永久保存，带时间戳和说话人标识（如果启用 diarization）

### 2.4 应用活动

**记录内容**：
- 窗口切换事件
- 文件打开/关闭
- 应用启动/退出

**用途**：
- 构建活动时间线
- 统计应用使用时长

---

## 3. 如何保存：存储与索引

### 3.1 本地存储

**存储位置**：
- macOS：`~/Library/Application Support/Rewind/`
- 数据库：SQLite（推测，基于行业惯例）

**加密**：
- **静态加密**：使用 macOS FileVault 或应用层加密
- **注意**：[The Sweet Setup 评测](https://thesweetsetup.com/a-first-look-at-rewind-ai/) 指出数据 **未完全加密**，如果 Mac 被盗且攻击者能登录，数据可被访问

**存储优化**：
- **去重**：相同内容的截图只存储一次
- **增量存储**：仅存储帧间差异
- **压缩**：3,750x 压缩率（官方数据）

**实际存储需求**（用户报告）：
- 官方声称：14 GB/月
- 实际使用：20-30 GB/月（取决于屏幕活动频率）

### 3.2 索引结构

**全文搜索**：
- **引擎**：SQLite FTS5（全文搜索扩展）或自定义倒排索引
- **索引内容**：OCR 文本 + 转录文本 + 元数据
- **搜索延迟**：<100ms（本地搜索）

**向量搜索**（Ask Rewind 功能）：
- **嵌入模型**：OpenAI text-embedding-ada-002 或类似模型
- **向量数据库**：可能使用 FAISS/Qdrant/Chroma
- **用途**：语义搜索（"我上周讨论的那个项目"）

**混合检索**：
- **第一阶段**：全文搜索快速过滤候选结果
- **第二阶段**：向量搜索重排序（reranking）
- **第三阶段**：LLM 生成自然语言回答

### 3.3 隐私保护

**本地优先**：
- 所有截图和音频 **永不上传** 到云端
- 仅在用户主动使用 Ask Rewind 时，**文本数据**（非截图）发送到 LLM API

**敏感内容过滤**：
- 用户可配置排除列表（应用/网站）
- 自动检测隐私浏览窗口

**数据保留**：
- 最小保留期：90 天（[官方文档](https://help.rewind.com/hc/en-us/articles/40862074159131-Configurable-data-retention-Feature-overview)）
- 用户可配置保留期限
- 修改保留期限仅影响新数据，旧数据不受影响

---

## 4. 如何更新：索引与检索

### 4.1 实时索引 vs 批量索引

**Rewind 的策略**：
- **屏幕捕获**：实时（~1 FPS）
- **OCR 提取**：准实时（延迟 <10 秒）
- **音频转录**：批量（延迟 1-5 分钟）
- **全文索引**：增量更新（新文本立即索引）

**权衡**：
- **实时索引**：搜索延迟低，但 CPU 占用高
- **批量索引**：CPU 占用低，但搜索结果有延迟

### 4.2 搜索结果排序

**排序因子**：
1. **时间相关性**：最近的结果优先
2. **文本匹配度**：BM25 算法（全文搜索标准）
3. **应用上下文**：用户当前使用的应用相关结果优先
4. **语义相关性**：向量搜索相似度（Ask Rewind）

**用户反馈**（来自 [Hacker News 讨论](https://news.ycombinator.com/item?id=36877000)）：
- "Ask Rewind 非常强大：'谁在上周会议中提到了 X'、'Joe 说他 Q3 的首要任务是什么'"
- 搜索速度快，但偶尔会漏掉结果（OCR 准确率问题）

### 4.3 记忆过期策略

**保留策略**：
- 用户可配置保留期限（90 天起）
- 超过保留期的数据自动删除
- 删除策略：按时间窗口删除，而非逐条删除

**存储限制**：
- 无硬性存储上限（取决于磁盘空间）
- 官方建议：至少 256 GB 可用空间

### 4.4 跨设备同步

**Rewind 的策略**：
- **不支持跨设备同步**（数据完全本地）
- iPhone 版本独立运行，数据不与 Mac 共享

**Limitless Pendant（硬件转向）**：
- 2024 年 Rewind 推出可穿戴设备 Limitless Pendant
- 数据上传到云端（与原 Rewind 理念相反）
- 2025 年 12 月被 Meta 收购，Pendant 停售，Rewind Mac 应用停止维护

---

## 5. 竞品对比：Screenpipe（开源替代）

### 5.1 架构对比

| 维度 | Rewind AI | Screenpipe |
|------|-----------|------------|
| **语言** | Swift（推测） | Rust |
| **数据库** | SQLite（推测） | SQLite |
| **OCR** | Apple Vision | Tesseract + Apple Vision |
| **音频转录** | Whisper（推测） | Whisper（本地运行） |
| **API** | 无公开 API | REST API (localhost:3030) |
| **开源** | 否 | 是（MIT License） |
| **跨平台** | macOS only | macOS/Windows/Linux |

### 5.2 Screenpipe 技术细节

**架构**（来自 [官方文档](https://mediar-ai.mintlify.app/architecture)）：
```rust
// 事件驱动架构
1. 屏幕捕获线程：持续截图（~1 FPS）
2. OCR 处理线程：提取文本（Tesseract/Apple Vision）
3. 音频捕获线程：录制系统音频 + 麦克风
4. Whisper 转录线程：批量转录音频
5. 索引线程：更新 SQLite 全文索引
6. API 服务器：Axum 框架，提供 REST API
```

**存储结构**：
```sql
-- 简化的 SQLite schema（推测）
CREATE TABLE frames (
    id INTEGER PRIMARY KEY,
    timestamp INTEGER,
    app_name TEXT,
    window_title TEXT,
    ocr_text TEXT,
    screenshot_path TEXT
);

CREATE TABLE audio_chunks (
    id INTEGER PRIMARY KEY,
    timestamp INTEGER,
    transcript TEXT,
    speaker_id INTEGER
);

CREATE VIRTUAL TABLE frames_fts USING fts5(ocr_text, window_title);
```

**优势**：
- 完全开源，可自定义
- 跨平台支持
- REST API 可集成到其他应用
- 支持 Ollama（本地 LLM）

**劣势**：
- OCR 准确率低于 Apple Vision
- 社区较小，文档不完善

---

## 6. 关键启示与设计建议

### 6.1 对 remem 的启示

**1. 不要盲目模仿 Rewind 的"全量捕获"**
- Rewind 的成功依赖于 **屏幕录制** 这一独特场景
- remem 的目标是 **Claude Code 的记忆系统**，不需要屏幕录制
- **应该捕获的**：对话历史、代码变更、工具调用、用户反馈
- **不应该捕获的**：屏幕截图、音频录制

**2. 学习 Rewind 的"本地优先"理念**
- 所有数据本地存储，隐私优先
- 仅在必要时（如 LLM 增强搜索）上传文本数据
- remem 应该：
  - 默认本地存储（SQLite）
  - 可选云端同步（加密）
  - 敏感信息过滤（API key、密码）

**3. 借鉴 Rewind 的"混合检索"策略**
- 全文搜索（快速过滤）+ 向量搜索（语义理解）+ LLM 重排序
- remem 应该：
  - SQLite FTS5 做全文搜索
  - 本地嵌入模型（如 nomic-embed-text）做向量搜索
  - Claude API 做最终的上下文生成

**4. 避免 Rewind 的"商业化困境"**
- Rewind 从软件转向硬件（Pendant），最终被收购
- 原因：纯软件方案难以持续变现（用户不愿为"记忆"付费）
- remem 的定位：
  - 开源项目，不追求商业化
  - 为 Claude Code 用户提供价值，而非独立产品

### 6.2 技术选型建议

**存储层**：
- **数据库**：SQLite（与 Screenpipe 一致）
- **全文搜索**：SQLite FTS5
- **向量搜索**：Qdrant（Rust 生态，性能好）或 FAISS（Python 绑定）

**捕获层**：
- **对话捕获**：Hook Claude Code 的 MCP 协议
- **代码变更捕获**：Git hooks + 文件监控
- **工具调用捕获**：MCP 工具调用日志

**检索层**：
- **第一阶段**：全文搜索（SQLite FTS5）
- **第二阶段**：向量搜索（Qdrant）
- **第三阶段**：LLM 生成（Claude API）

**隐私保护**：
- 敏感信息检测（正则匹配 API key、密码）
- 用户可配置排除列表（文件/目录）
- 数据加密（SQLCipher）

### 6.3 避免的陷阱

**1. 不要过度压缩**
- Rewind 的 3,750x 压缩率是因为视频数据冗余度高
- remem 的数据（文本为主）压缩率远低于此
- 不要为了"看起来厉害"而牺牲数据完整性

**2. 不要依赖云端**
- Rewind 的成功在于"100% 本地"
- Limitless 转向云端后用户反弹强烈
- remem 应该默认本地，云端同步作为可选功能

**3. 不要忽视性能**
- Rewind 的 1 FPS 捕获 + 批量转录是精心设计的权衡
- remem 的捕获频率应该根据实际需求调整（不是越高越好）

**4. 不要假设用户会主动保存**
- Rewind 的自动捕获是核心价值
- remem 的 save_memory 工具不会被 Claude 主动调用
- **必须实现自动捕获机制**

---

## 7. 参考资料

### 技术分析
- [Tearing down the Rewind app — Kevin Chen](https://kevinchen.co/blog/rewind-ai-app-teardown/)
- [Ian Sinnott's Rewind Review](https://notes.iansinnott.com/blog/posts/Rewind.ai+Review)
- [Screenpipe Architecture](https://mediar-ai.mintlify.app/architecture)
- [Screenpipe vs Rewind Comparison](https://screenpi.pe/blog/best-rewind-ai-alternative-2026)

### 官方文档
- [Rewind Data Retention Policy](https://help.rewind.com/hc/en-us/articles/40862074159131-Configurable-data-retention-Feature-overview)
- [Apple ScreenCaptureKit Documentation](https://developer.apple.com/videos/play/wwdc2022/10156/)

### 用户评测
- [Rewind AI Review — The Sweet Setup](https://thesweetsetup.com/a-first-look-at-rewind-ai/)
- [Hacker News Discussion](https://news.ycombinator.com/item?id=36877000)

### 行业动态
- [Meta Acquires Limitless](https://techcrunch.com/2025/12/05/meta-acquires-ai-device-startup-limitless/)
- [Limitless Pendant Launch](https://merlio.app/blog/limitless-ai-guide)

---

## 附录：Rewind 时间线

- **2022 年初**：Rewind 发布，首个"搜索你的人生"应用
- **2023 年**：推出 Ask Rewind（LLM 增强搜索）
- **2024 年 8 月**：推出 Limitless Pendant（可穿戴设备，$99）
- **2024 年底**：Rewind 更名为 Limitless，转向硬件 + 云端
- **2025 年 12 月**：Meta 收购 Limitless，Pendant 停售，Rewind Mac 应用停止维护
- **2026 年**：Rewind 应用在欧盟/英国地区停止服务

**教训**：技术再强，商业模式不成立也会失败。remem 作为开源项目，不应追求商业化，而应专注于为用户创造价值。
