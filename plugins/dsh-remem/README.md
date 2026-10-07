# remem for DeepSeek Harness

[English](#english)

把已安装的 remem 接到 DSH 的真实生命周期。插件只调用本地 CLI：每轮首步
注入项目记忆，记录用户提示、助手文本与工具结果，轮次结束排队自动提炼。
记忆提取、来源治理、加密与检索都沿用 remem。

当前包为发布准备版本，尚未上传 npm。验证目标：DSH `0.2.0-rc.2` 与 Cordis
`4.0.4`。DSH 主线 API 与该 npm 版本不同；本插件不宣称已验证主线兼容。
需要 Node.js 22.19+ 和包含 DSH 宿主适配的 remem 0.6.103（此分支，未发布）。旧 remem 会在
首次真实 CLI 请求返回可见错误。

## 本地安装

从 remem 仓库执行：

```bash
cargo build --bin remem
cd plugins/dsh-remem
npm ci
npm test
npm pack
```

通过 DSH CLI 向已有 profile 安装生成的 bundle：

```bash
dsh plugin --profile headless add /absolute/path/remem-ai-dsh-remem-0.1.0.tgz
```

`dsh.bundle` 指向随包发布的 `cordis.patch.yml`，CLI 将其注册为 profile layer。
在该 profile 的用户 patch 中设置 `config.executable` 为构建出的 remem
**绝对路径**。发布后可使用包名安装；本地验收不依赖已发布的 npm 包。

```yaml
- id: remem
  config:
    executable: /absolute/path/remem/target/debug/remem
    # 可选：已有 remem memory_ai profile，不在插件里配置凭证
    # profile: codex
```

确保会话有绝对 `cwd`；在 remem 内配置提炼使用的 executor/profile。插件继承
remem 环境与配置，不下载二进制，不注册 MCP，不修改 DSH 的审批规则。
`session/flush` 和插件卸载等待待处理子进程。CLI 错误写入宿主错误日志并在
flush / 下次请求准备时传播；详细诊断查 remem 日志，插件不转发可能含敏感
值的 stderr。

自动捕获只覆盖插件加载后的实时事件。用户来源 `user` 会被保存，插件注入
来源 `remem` 不会被当成人类提示重复收录。历史 DSH 文件导入未实现。提炼
图片和文件仅保存有界引用元数据，不读取附件字节。用户提示捕获不执行
无法送达的即时召回；每轮首步仅在已提交历史的记忆快照变化时注入，取消
请求准备不会消耗后续注入。CLI 准备阶段不写注入审计或使用计数；
交付事实以 DSH 已提交的记忆快照为准。工具参数先解析 JSON，再由 remem 按键脱敏；
非法 JSON 返回可见捕获错误，不保存原始参数。提炼任务落盘不等于已生成长期记忆：后者要求 remem 的 AI executor 正常工作。

## 验证

```bash
npm test
# 使用真实分支二进制和隔离数据库运行生命周期烟测：
REMEM_DSH_BINARY=/absolute/path/remem/target/debug/remem npm test
```

烟测使用真正的 Cordis、DSH AgentLoop/SessionStore 与 remem CLI，模型流是
确定性测试输入，无外部 API 凭证。它验证注入、提示/助手/工具捕获、Stop
与后台任务队列；不宣称真实模型记忆质量。

## English

This thin plugin connects an installed remem CLI to DeepSeek Harness. It injects
project memory at the first step of each turn, captures live human prompts,
assistant text and completed tools, and queues remem's existing distillation at
turn end. Retrieval, storage, redaction and governance stay in remem.

The package is prepared for publication and is not yet on npm. Tested APIs target
DSH 0.2.0-rc.2 and Cordis 4.0.4; the current upstream source uses a different API.
Use Node.js 22.19+ and the remem binary built from this branch, run `npm ci`,
`npm test`, `npm pack`, then run `dsh plugin --profile headless add /absolute/path/to/the.tgz`.
The bundle is registered as a profile layer. Override the `remem` entry with the
executable's absolute path using the user patch above. Configure memory AI in
remem 0.6.103 from this branch (not yet released). Old binaries fail visibly on the first real DSH CLI request.

Live sessions require an absolute project cwd. Capture runs in order and is
awaited at session flush and plugin disposal. Errors remain visible without
forwarding secret-bearing subprocess stderr. Injected remem context is attributed
to its own source and never recaptured as a human prompt. Historical DSH transcript
import, automatic MCP registration, and memory AI credentials are outside this
adapter. Image/file capture retains bounded reference metadata without reading
attachment bytes. Prompt capture does not perform undeliverable recall. Snapshot
deduplication uses committed session history, so cancelled preparation can retry.
CLI preparation writes no injection audit or usage credit; committed DSH snapshots
are the delivery record. Tool arguments are decoded before secret-key redaction;
malformed JSON causes a visible capture error without storing the raw arguments.
Successful queueing does not imply completed LLM memory promotion.
