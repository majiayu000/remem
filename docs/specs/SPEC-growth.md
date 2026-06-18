# remem Growth Spec — 从自用工具到受欢迎的开源项目

> 目标：让 remem 成为 Claude Code 用户的必装记忆系统
> 作者：lifcc + Claude
> 日期：2026-03-23
> 状态：DRAFT

---

## 一、现状诊断

### 产品现状
- 3000 行 Rust，单 binary 3.6MB，55 个测试全绿
- 已稳定运行 1 个月：14,915 会话、4,322 observations、621 memories、230 个项目
- 日均消耗 $2-5（Haiku 模型），用户无感知
- 架构完整：Hook 采集 → LLM 提炼 → SQLite 存储 → MCP 检索

### 核心阻塞
1. **无法安装** — 需要 Rust 工具链编译，砍掉 90% 潜在用户
2. **无法感知** — 静默运行，用户不知道记忆在帮自己
3. **无法发现** — 没有 release、没有 CI、没有社区入口
4. **无法区分** — 用户不理解 remem vs Claude Code 原生记忆的差异
5. **版本号 0.1.0** — 没有发过任何 release

---

## 二、5 个维度的完整设计

### 维度 1：安装体验（P0 — 不做就没人用）

> 核心原则：用户在哪装东西，remem 就在哪出现。
> Claude Code 用户 ≈ Node.js + macOS/Linux 开发者，覆盖 npm + brew + curl + cargo 四条路。

#### 1.1 预编译 Release Binary（所有安装渠道的基础）

**CI/CD Pipeline**：`.github/workflows/release.yml`

触发条件：push tag `v*`

构建矩阵：
| Target | OS | Arch | 产物名 |
|--------|-----|------|--------|
| x86_64-apple-darwin | macOS Intel | x86_64 | remem-darwin-x86_64 |
| aarch64-apple-darwin | macOS Apple Silicon | aarch64 | remem-darwin-aarch64 |
| x86_64-unknown-linux-gnu | Linux | x86_64 | remem-linux-x86_64 |
| aarch64-unknown-linux-gnu | Linux ARM | aarch64 | remem-linux-aarch64 |

每个 target 产出：
- `remem-<os>-<arch>.tar.gz`（binary + LICENSE + README）
- `remem-<os>-<arch>.tar.gz.sha256`（校验和）

Release 自动创建，附上 CHANGELOG 对应版本内容。

**关键约束**：
- rusqlite 使用 `bundled` feature，SQLite 静态链接，零外部依赖
- `opt-level = "z"` + `lto = true` + `strip = true` 已配置
- Linux 构建使用 `cross` 或 GitHub Actions 原生 runner
- 所有安装渠道（npm/brew/curl）最终都是下载这些预编译 binary，不重复编译

#### 1.2 安装渠道总览

```
用户习惯              安装命令                              优先级
─────────────────────────────────────────────────────────────────
Node.js 开发者     →  npm install -g @anthropic-tools/remem    P0
macOS 开发者       →  brew install majiayu000/tap/remem        P0
通用 / CI          →  curl -fsSL .../install.sh | sh           P0
Rust 开发者        →  cargo install remem                      P1
源码编译           →  cargo build --release                    已有
```

所有渠道安装后都自动运行 `remem install` 配置 hooks + MCP。

#### 1.3 npm 分发（覆盖最多 Claude Code 用户）

Claude Code 用户大概率有 Node.js 环境。npm 是最低摩擦安装路径。

**架构**：采用 esbuild/turbo 验证过的 "platform-specific optional dependencies" 模式：

```
npm packages:
  @anthropic-tools/remem            ← 主包（thin wrapper）
  @anthropic-tools/remem-darwin-arm64    ← macOS Apple Silicon binary
  @anthropic-tools/remem-darwin-x64      ← macOS Intel binary
  @anthropic-tools/remem-linux-x64       ← Linux x86_64 binary
  @anthropic-tools/remem-linux-arm64     ← Linux ARM binary
```

**主包 `@anthropic-tools/remem`**：

```json
{
  "name": "@anthropic-tools/remem",
  "version": "0.2.0",
  "description": "Persistent memory for Claude Code",
  "bin": { "remem": "bin/remem" },
  "optionalDependencies": {
    "@anthropic-tools/remem-darwin-arm64": "0.2.0",
    "@anthropic-tools/remem-darwin-x64": "0.2.0",
    "@anthropic-tools/remem-linux-x64": "0.2.0",
    "@anthropic-tools/remem-linux-arm64": "0.2.0"
  },
  "scripts": {
    "postinstall": "node postinstall.js"
  }
}
```

**`bin/remem`**（thin launcher）：
```javascript
#!/usr/bin/env node
const { execFileSync } = require("child_process");
const path = require("path");

// 按 os+arch 定位 binary
const platformPkg = {
  "darwin-arm64":  "@anthropic-tools/remem-darwin-arm64",
  "darwin-x64":    "@anthropic-tools/remem-darwin-x64",
  "linux-x64":     "@anthropic-tools/remem-linux-x64",
  "linux-arm64":   "@anthropic-tools/remem-linux-arm64",
}[`${process.platform}-${process.arch}`];

const binPath = require.resolve(`${platformPkg}/bin/remem`);
execFileSync(binPath, process.argv.slice(2), { stdio: "inherit" });
```

**`postinstall.js`**：
```javascript
// npm install -g 后自动配置 Claude Code hooks + MCP
const { execFileSync } = require("child_process");
try {
  execFileSync(binPath, ["install"], { stdio: "inherit" });
} catch (e) {
  console.log("Run 'remem install' manually to configure Claude Code hooks.");
}
```

**平台包结构**（以 `@anthropic-tools/remem-darwin-arm64` 为例）：
```
package.json   { "name": "...", "os": ["darwin"], "cpu": ["arm64"] }
bin/remem      ← 预编译 Rust binary（从 GitHub Release 拷贝）
```

**发布流程**（集成到 `release.yml`）：
1. GitHub Release 构建完成后
2. 下载 4 个 binary
3. 分别打包到 4 个平台 npm 包
4. 发布主包 + 4 个平台包到 npmjs.com
5. 需要 `NPM_TOKEN` secret

**用户体验**：
```bash
npm install -g @anthropic-tools/remem
# → 自动下载对应平台 binary
# → 自动执行 remem install
# → 完成，重启 Claude Code 即可
```

**备选 scope**：如果 `@anthropic-tools` 不可用，用 `@remem-ai/remem` 或无 scope 的 `remem-cli`。

#### 1.4 Homebrew tap

```
homebrew-tap/           ← 独立 repo: majiayu000/homebrew-tap
  Formula/remem.rb
```

**Formula**：
```ruby
class Remem < Formula
  desc "Persistent memory for Claude Code"
  homepage "https://github.com/majiayu000/remem"
  version "0.2.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/majiayu000/remem/releases/download/v0.2.0/remem-darwin-aarch64.tar.gz"
      sha256 "..."
    end
    on_intel do
      url "https://github.com/majiayu000/remem/releases/download/v0.2.0/remem-darwin-x86_64.tar.gz"
      sha256 "..."
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/majiayu000/remem/releases/download/v0.2.0/remem-linux-aarch64.tar.gz"
      sha256 "..."
    end
    on_intel do
      url "https://github.com/majiayu000/remem/releases/download/v0.2.0/remem-linux-x86_64.tar.gz"
      sha256 "..."
    end
  end

  def install
    bin.install "remem"
  end

  def post_install
    system bin/"remem", "install"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/remem --version")
  end
end
```

**安装命令**：
```bash
brew install majiayu000/tap/remem
```

**Formula 自动更新**：release.yml 成功后，用 `gh` 命令自动更新 homebrew-tap repo 中的 Formula（更新 URL + sha256）。

**升级到 homebrew-core 的条件**：
- GitHub stars > 75
- 30+ forks 或 notable users
- 提交 PR 到 Homebrew/homebrew-core

#### 1.5 curl 一行安装脚本

文件：`install.sh`（仓库根目录）

```bash
curl -fsSL https://raw.githubusercontent.com/majiayu000/remem/main/install.sh | sh
```

脚本逻辑：
1. 检测 OS（darwin/linux）+ Arch（x86_64/aarch64）
2. 检测已有包管理器，给出建议：
   - 有 brew → 提示 `brew install majiayu000/tap/remem`
   - 有 npm → 提示 `npm i -g @anthropic-tools/remem`
   - 都没有 → 直接下载 binary
3. 从 GitHub Release 下载对应 binary
4. 验证 sha256 校验和
5. 安装到 `~/.local/bin/remem`
6. 检查 `~/.local/bin` 是否在 PATH 中，不在则提示添加
7. 自动执行 `remem install`（配置 hooks + MCP）
8. 打印成功信息 + 下一步提示

**安全**：脚本开头 `set -euo pipefail`，每步有错误处理。

#### 1.6 cargo install（Rust 用户）

```bash
cargo install remem
```

需要在 `Cargo.toml` 中完善 metadata 并发布到 crates.io：

```toml
[package]
name = "remem"
version = "0.2.0"
edition = "2021"
description = "Persistent memory for Claude Code — auto-captures decisions, bugs, and patterns across sessions"
license = "MIT"
repository = "https://github.com/majiayu000/remem"
homepage = "https://github.com/majiayu000/remem"
keywords = ["claude-code", "memory", "mcp", "ai", "developer-tools"]
categories = ["command-line-utilities", "development-tools"]
readme = "README.md"
```

**发布流程**：release.yml 中在 binary 构建完成后自动 `cargo publish`。需要 `CARGO_REGISTRY_TOKEN` secret。

#### 1.7 安装渠道对比

| 渠道 | 目标用户 | 前置依赖 | 自动配置 | 自动更新 |
|------|---------|---------|---------|---------|
| `npm i -g` | Node.js 开发者（最多） | Node.js | ✅ postinstall | `npm update -g` |
| `brew install` | macOS 开发者 | Homebrew | ✅ post_install | `brew upgrade` |
| `curl \| sh` | 通用 / CI / 无包管理器 | curl | ✅ 脚本内执行 | 重新运行脚本 |
| `cargo install` | Rust 开发者 | Rust toolchain | ❌ 需手动 `remem install` | `cargo install remem` |
| 源码编译 | 贡献者 / 定制需求 | Rust toolchain | ❌ 需手动 | git pull + rebuild |

#### 1.8 版本同步策略

一次 `git tag v0.2.0` 触发 release.yml，自动完成：
1. 构建 4 个平台 binary → GitHub Release
2. 发布 npm 主包 + 4 个平台包 → npmjs.com
3. 更新 homebrew-tap Formula → majiayu000/homebrew-tap
4. 发布 crate → crates.io

所有渠道版本号始终一致，由 Cargo.toml 中的 version 驱动。

#### Done-when
- [ ] `npm install -g @anthropic-tools/remem` 在 macOS + Linux 上 < 30 秒完成
- [ ] `brew install majiayu000/tap/remem` 在 macOS 上 < 30 秒完成
- [ ] `curl ... | sh` 在全新 macOS（Intel + Apple Silicon）和 Ubuntu 上 < 30 秒完成
- [ ] `cargo install remem` 从 crates.io 安装成功
- [ ] 所有渠道安装后 `remem --version` 输出一致的版本号
- [ ] 所有渠道安装后重启 Claude Code，SessionStart hook 自动注入 remem 上下文
- [ ] push tag 后 4 个渠道自动发布，无需手动操作

---

### 维度 1.5：偏好记忆增强（P0 — 用户认为最重要的功能）

> "Claude 总是不记得我的代码风格偏好" — 这是用户最高频、最直接的痛点。
> 当前 remem 有 preference 类型的 observation/memory，但注入不够显式，Claude 感知不到。

#### 问题分析

当前偏好数据流断裂：
```
偏好被捕获（observe → flush → observations type=preference）  ✅ 有
偏好被存储（memories table, memory_type=preference）          ✅ 有
偏好被注入到新会话                                            🟡 混在 50 条 observations 里，不够显式
Claude 按偏好行动                                             ❌ 没有专门的 prompt 引导
```

核心问题：偏好被当作普通 observation 处理，没有特殊通道。

#### 设计方案

##### A. 偏好专区注入（context.rs 改造）

在 SessionStart 注入的上下文中，增加独立的 **Preferences** 区块，置于最前面（比 observations 和 sessions 优先级更高）：

```markdown
# [tools/remem @main] context 2026-03-23

## Your Preferences (always apply these)
- 全部使用中文交流
- 优先使用 ok_or_else()? 处理 Option，禁止非测试代码中的 expect()
- Rust 项目使用 cargo，Node.js 使用 pnpm/bun，Python 使用 uv
- 代码风格：snake_case，单文件不超过 200 行
- 提交消息格式：<type>: <description>

## Core (observations + sessions below)
...
```

**实现逻辑**：
1. context.rs 新增 `render_preferences()` 函数
2. 从 memories 表查询 `memory_type = 'preference'` 的记录
3. 从 observations 表查询 `type = 'preference'` 的记录
4. 去重合并后，提取为简短的 bullet point 列表
5. 放在上下文最前面，确保不会被 context window 截断

**Token 预算**：偏好区块上限 500 tokens（约 20-30 条偏好），不挤占 observations 的配额。

##### B. 偏好显式捕获增强（observe.rs 改造）

当前问题：偏好大多来自 session summary 的 `preferences` 字段自动提升，但提升质量不稳定。

增强方案：
1. **PostToolUse 中识别偏好信号** — 当用户在会话中纠正 Claude 的行为时（"不要用 npm"、"用中文"、"snake_case"），observe.rs 识别这类模式并标记为 preference 类型
2. **save_memory 偏好快捷方式** — MCP 工具增加 `memory_type=preference` 的显式支持，Claude 被纠正时主动保存
3. **偏好去重** — 同一偏好不同表述应合并（如 "用中文" 和 "全部使用中文交流"），基于 topic_key 去重

##### C. `remem preferences` 命令（新增）

让用户查看和管理自己的偏好：

```bash
$ remem preferences

Your Preferences (23 total, 5 projects):

  Global:
    • 全部使用中文交流 (from 12 sessions)
    • snake_case naming, API 边界 camelCase (from 8 sessions)
    • 使用 pnpm/bun, 禁止 npm/yarn (from 15 sessions)

  tools/remem:
    • 优先使用 ok_or_else()? 处理 Option (from 3 sessions)
    • 记忆质量优先，不为省成本砍功能 (from 2 sessions)

  om-generator:
    • Next.js App Router, 禁止 Pages Router (from 5 sessions)
```

```bash
$ remem preferences --add "总是使用 Tailwind CSS，不要 inline style"
$ remem preferences --remove 15    # 按 ID 删除
$ remem preferences --global       # 只看全局偏好
```

##### D. 偏好跨项目共享

偏好天然应该跨项目生效。设计：
- **全局偏好**：出现在 3+ 个不同项目中的偏好自动提升为全局
- **项目偏好**：只在特定项目出现的偏好保留在项目级别
- **注入逻辑**：新会话注入 = 全局偏好 + 当前项目偏好

##### E. 与 CLAUDE.md 的关系

用户可能已经在 CLAUDE.md 中写了偏好规则。remem 不重复这些，而是：
1. context.rs 注入时检测 CLAUDE.md 中已有的规则（简单关键词匹配）
2. 重复的偏好标记为 `[also in CLAUDE.md]`，不重复注入
3. remem 捕获到的新偏好如果不在 CLAUDE.md 中 → 注入
4. 长期稳定的偏好 → 建议用户手动加入 CLAUDE.md（`remem preferences --suggest-claude-md`）

#### Done-when
- [ ] SessionStart 上下文有独立的 "Your Preferences" 区块，置于最前
- [ ] 偏好区块包含全局 + 当前项目的合并偏好列表
- [ ] `remem preferences` 命令能查看、添加、删除偏好
- [ ] 出现在 3+ 项目的偏好自动标记为全局
- [ ] 新会话中 Claude 能直接按偏好行动，无需用户重复提醒

---

### 维度 2：用户感知价值（P1 — 做了就有 wow）

#### 2.1 安装后首次体验优化

当前问题：安装后什么也看不到，直到下一次会话结束才有数据。

方案：`remem install` 成功后打印引导信息：

```
✓ remem installed successfully!

  Next: Start a new Claude Code session in any project.
  remem will silently capture your work and build memory.

  After your first session ends, try:
    remem context --cwd .    # see what remem remembers
    remem usage --today      # see today's memory stats

  Tip: After a few sessions, new conversations will
  automatically know about your past decisions and patterns.
```

#### 2.2 `remem status` 命令（新增）

一个快速查看 remem 运行状态的命令，让用户随时确认"它在工作"：

```bash
$ remem status

remem v0.2.0 — running ✓

  Database: ~/.remem/remem.db (124 MB)
  Sessions: 14,915 across 230 projects
  Memories: 4,322 observations + 621 long-term memories
  Today:    53 new observations from 8 sessions
  Workers:  2 active MCP servers
  Pending:  955 events queued (3 projects)
  Cost:     $2.31 today / $51.73 total

  Top projects:
    tools/harness       620 observations
    om-generator-web    408 observations
    om-generator        280 observations
```

实现：纯 SQLite 查询，不需要 AI 调用，< 100ms 响应。

#### 2.3 README Demo GIF

制作一个 30 秒 GIF/视频，展示核心场景：

**场景**：
1. 会话 A：在项目中修复了一个 bug，决定使用方案 X 而非 Y
2. 关闭会话
3. 会话 B：新会话开始，Claude 自动说"我记得你上次决定用方案 X，因为 Y 有性能问题"
4. 对比：没有 remem 时，会话 B 完全不知道会话 A 的决策

工具：asciinema 或 VHS（https://github.com/charmbracelet/vhs）

#### Done-when
- [ ] `remem status` 输出完整运行状态
- [ ] README 顶部有 demo GIF
- [ ] 安装后有清晰的引导信息

---

### 维度 3：精准痛点定位（P1 — README 重写）

#### 3.1 README 结构重组

当前 README 偏技术文档（架构图、数据流、schema），对新用户不友好。

新结构：

```markdown
# remem — Persistent Memory for Claude Code

> Stop re-explaining your project every new session.

[demo GIF]

## The Problem

Every time you start a new Claude Code session:
- You re-explain your project structure
- You re-describe decisions you already made
- You debug the same issues Claude already helped you fix
- You lose context from yesterday's work

Claude Code's built-in CLAUDE.md helps, but it's manual and limited.

## How remem Solves This

remem runs silently in the background. It captures your decisions,
bug fixes, and patterns — then automatically injects them into
every new session.

| | Without remem | With remem |
|---|---|---|
| New session | Start from zero | Picks up where you left off |
| Past decisions | Lost forever | Automatically recalled |
| Bug patterns | Debug again | "This was fixed before, here's how" |
| Cross-session search | ❌ | `search("auth middleware change")` |
| Memory lifecycle | Manual CLAUDE.md | Auto: capture → refine → compress |

## Install (< 30 seconds)

\`\`\`bash
curl -fsSL https://raw.githubusercontent.com/majiayu000/remem/main/install.sh | sh
\`\`\`

## How It Works
[简化版架构图，只保留 4 行 hook 说明]

## Commands
[精简命令表]

## Configuration
[环境变量表，折叠在 <details> 里]

## Architecture
[完整技术文档，折叠在 <details> 里]
```

#### 3.2 痛点驱动的 tagline

主 tagline：**"Stop re-explaining your project every new session."**

备选：
- "Your Claude Code sessions shouldn't start from zero."
- "Persistent memory that actually works — zero config, fully automatic."

#### Done-when
- [ ] README 首屏（无需滚动）包含：问题描述 + 对比表 + 安装命令
- [ ] 技术细节折叠在 `<details>` 中，不干扰新用户

---

### 维度 4：与原生记忆的差异化（P1 — 融入 README）

#### 4.1 对比表（放在 README "The Problem" 之后）

```markdown
## remem vs Claude Code Built-in Memory

| Capability | CLAUDE.md | Auto Memory | remem |
|---|---|---|---|
| Setup | Manual rules | Automatic | Automatic |
| What's captured | What you write | Simple notes | Structured: decisions, bugs, patterns |
| Cross-session search | ❌ | ❌ | ✅ FTS + semantic |
| Timeline browsing | ❌ | ❌ | ✅ timeline tool |
| Memory lifecycle | Manual edit | Grows forever | Auto: active → stale → compressed |
| Multi-project | Per-project only | Per-project only | Global DB, cross-project search |
| Cost visibility | ❌ | ❌ | ✅ token & cost tracking |
| LLM-refined summaries | ❌ | ❌ | ✅ Session summaries with decisions/learnings |
```

#### 4.2 "Works alongside, not against" 定位

**关键信息**：remem 不替代 CLAUDE.md，它增强它。
- CLAUDE.md = 你手写的项目规则（保留）
- remem = 自动捕获的工作记忆（新增）
- 两者互补，不冲突

#### Done-when
- [ ] README 包含清晰的对比表
- [ ] 明确说明 remem 与原生记忆互补而非替代

---

### 维度 5：社区和分发（P2-P3）

#### 5.1 GitHub 仓库优化

```bash
gh repo edit majiayu000/remem \
  --description "Persistent memory for Claude Code — auto-captures decisions, bugs, and patterns across sessions. Single Rust binary, zero config." \
  --add-topic claude-code,claude,mcp,memory,ai-agent,rust,claude-code-hooks,llm,developer-tools,productivity
```

#### 5.2 CI Pipeline（`.github/workflows/ci.yml`）

```yaml
on: [push, pull_request]
jobs:
  check:
    - cargo check
    - cargo test
    - cargo clippy -- -D warnings
    - cargo fmt -- --check
```

badges 加到 README 顶部：CI status + License + Release version

#### 5.3 社区发布计划

| 时间 | 渠道 | 内容 |
|------|------|------|
| v0.2.0 发布后 | GitHub Release | changelog + 安装说明 |
| 同天 | r/ClaudeAI | "I built a persistent memory system for Claude Code" |
| 同天 | Claude Code Discord #community | 简短介绍 + 链接 |
| 1 周后 | Hacker News | "Show HN: remem — persistent memory for Claude Code (Rust)" |
| 持续 | X/Twitter | 功能 demo 短视频 |

#### 5.4 用户证据

你自己的使用数据就是最好的证明：

```markdown
## Real-world Usage

Built and dogfooded for 30+ days:
- 14,915 sessions across 230 projects
- 4,322 structured observations captured
- 621 long-term memories promoted
- ~$50 total AI cost (~$1.70/day with Haiku)
```

#### Done-when
- [ ] CI pipeline 绿灯 + README 有 badges
- [ ] GitHub description + topics 已设置
- [ ] 至少在 1 个社区发布

---

## 三、执行计划

### Phase 1：核心功能 + 发布基础（2-3 天）

**1A. 偏好记忆增强（最高优先级）**

| Step | 任务 | 产出 | 验证 |
|------|------|------|------|
| 1A.1 | context.rs 增加 Preferences 专区渲染 | 独立偏好区块 | `remem context --cwd .` 输出顶部有 "Your Preferences" |
| 1A.2 | 偏好查询逻辑：memories(preference) + observations(preference) 合并去重 | 偏好聚合函数 | 单元测试覆盖去重和合并 |
| 1A.3 | 偏好跨项目共享：3+ 项目出现 → 自动标记全局 | 全局偏好逻辑 | 测试：同一偏好在 3 个项目出现后被标记为 global |
| 1A.4 | `remem preferences` 命令（查看/添加/删除） | 新子命令 | `remem preferences` 输出当前偏好列表 |
| 1A.5 | 偏好 vs CLAUDE.md 去重 | 避免重复注入 | context 输出中不包含 CLAUDE.md 已有的规则 |

**1B. 发布基础设施**

| Step | 任务 | 产出 | 验证 |
|------|------|------|------|
| 1B.1 | 创建 `.github/workflows/ci.yml` | CI pipeline | push 后自动运行 cargo check/test/clippy/fmt |
| 1B.2 | 创建 `.github/workflows/release.yml` | Release pipeline | push tag → 4 个 binary + npm + brew + crates.io 自动发布 |
| 1B.3 | 写 `install.sh` 安装脚本 | 一行安装 | 全新机器 `curl ... \| sh` < 30s 完成 |
| 1B.4 | 创建 npm 包结构（主包 + 4 平台包） | npm 分发 | `npm i -g @anthropic-tools/remem` 成功 |
| 1B.5 | 创建 homebrew-tap repo + Formula | brew 分发 | `brew install majiayu000/tap/remem` 成功 |
| 1B.6 | Cargo.toml 完善 metadata | crates.io 分发 | `cargo install remem` 成功 |
| 1B.7 | 修复 `install.rs` 未提交改动 | 确认意图并提交或回滚 | `git status` 干净 |
| 1B.8 | 修复 `dedup.rs:112` 警告 | 无 warning | `cargo check 2>&1 \| grep warning` 为空 |
| 1B.9 | 版本号升级 `0.1.0` → `0.2.0` | Cargo.toml 更新 | `cargo build` 成功 |

### Phase 2：产品体验 + README（2-3 天）

| Step | 任务 | 产出 | 验证 |
|------|------|------|------|
| 2.1 | 实现 `remem status` 命令 | 新子命令 | `remem status` 输出完整运行状态 |
| 2.2 | 优化 `remem install` 输出 | 引导信息 | 安装后有清晰的 next steps |
| 2.3 | 重写 README（痛点驱动结构） | 新 README.md | 首屏无滚动看到：问题 + 安装 + 对比 |
| 2.4 | README 加入多渠道安装命令 | npm/brew/curl/cargo 四选一 | 每个渠道有独立代码块 |
| 2.5 | 加入对比表（remem vs 原生记忆） | README 章节 | 差异一目了然 |
| 2.6 | 加入 Real-world Usage 数据 | README 章节 | 14,915 sessions / 4,322 observations 等真实数据 |
| 2.7 | 制作 demo GIF | assets/demo.gif | README 顶部展示 |

### Phase 3：社区发布（1 天）

| Step | 任务 | 产出 | 验证 |
|------|------|------|------|
| 3.1 | 设置 GitHub repo metadata | description + topics | `gh repo view` 显示正确 |
| 3.2 | 打 v0.2.0 tag 触发 release | 4 binary + npm + brew + crate 全自动 | 所有渠道版本一致 |
| 3.3 | 端到端验证所有安装渠道 | 在全新环境测试 | 4 个渠道都能 < 30s 安装 |
| 3.4 | 发布到 r/ClaudeAI | Reddit 帖子 | 发布成功 |
| 3.5 | 发布到 Claude Code Discord | Discord 消息 | 发布成功 |

### Phase 4：迭代增强（持续）

| 任务 | 优先级 | 备注 |
|------|--------|------|
| 偏好信号自动识别（用户纠正行为 → 自动捕获偏好） | P1 | Phase 1 之后持续迭代 |
| `remem preferences --suggest-claude-md` | P2 | 建议稳定偏好写入 CLAUDE.md |
| Web dashboard（TUI 或浏览器） | P2 | `remem dashboard` 可视化 |
| 跨项目搜索增强 | P2 | 打破项目隔离壁 |
| "What remem saved you" 周报 | P3 | 定期展示记忆价值 |
| 团队共享模式 | P3 | 远期，需要服务端 |
| Show HN | P3 | 发布 1 周后，收集初始用户反馈后再发 |

---

## 四、风险和约束

| 风险 | 影响 | 缓解 |
|------|------|------|
| cross-compile 失败（特别是 rusqlite bundled SQLite） | Release 无法产出 | CI 中先验证 cross build，失败时用 GitHub Actions 原生 runner |
| install.sh 在某些 Linux 发行版不兼容 | 用户安装失败 | 脚本中检测 curl/wget，给出清晰错误信息 |
| Demo GIF 制作耗时 | 延迟发布 | 先用 asciinema，后续升级为视频 |
| pending 积压问题（当前 955 条） | 影响产品印象 | Phase 1 前清理积压，优化 flush 稳定性 |
| `claude CLI exit status: 1` 偶发 | observation 丢失 | 增加错误日志细节，排查根因 |

---

## 五、不做什么

- **不做 GUI 客户端** — 保持 CLI-first，Claude Code 用户就是终端用户
- **不做 VS Code 扩展** — 聚焦 Claude Code，不分散精力
- **不做 SaaS/云服务** — 保持本地运行，隐私优先
- **不做知识图谱** — FTS5 + 结构化存储已够用，复杂度不值得
- **不改核心架构** — Hook 采集 + LLM 提炼 + SQLite 存储的管线已验证，不动

---

## 六、成功指标

| 指标 | 当前 | 目标（发布后 1 个月） |
|------|------|---------------------|
| GitHub stars | ~0 | 100+ |
| install.sh 下载量 | 0 | 500+ |
| GitHub issues（非自己提的） | 0 | 10+ |
| Reddit/HN 帖子评论 | 0 | 50+ |
| 外部 contributor | 0 | 1+ |
