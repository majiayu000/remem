# GH671 Tech Spec: 把高置信度纠偏编译成 hook 强制的确定性检查

Issue: https://github.com/majiayu000/remem/issues/671
Product spec: docs/specs/GH671/PRODUCT.md
Route: write_spec
Locale: zh-CN
Status: Draft, needs human approval before implementation
Related: #383, #617-620

## 1. 当前实现证据

- `src/memory_candidate/apply.rs`：`insert_lesson_metadata` 写入 `memory_lessons(memory_id, confidence, reinforcement_count, source_evidence, last_reinforced_at_epoch, stale_after_epoch)`，初始 `reinforcement_count = 1`。`soft_supersede_routed` 把被取代的 memory `status` 置为 `stale`。[source: src/memory_candidate/apply.rs:408-470]
- migration `src/migrations/v017_memory_lessons.sql` 已建 `memory_lessons` 表；当前最高 migration 为 `v034_graph_edge_file_nodes.sql`。[source: ls src/migrations]
- hook 安装：`src/install/config.rs::build_hooks` 生成各 host 的 hook 命令。Claude Code：`SessionStart -> remem context`（timeout 15000），`UserPromptSubmit -> remem session-init`（15000），`PostToolUse`（matcher `Write|Edit|NotebookEdit|Bash|Grep|Glob|Task`）`-> remem observe`（timeout 120000），`Stop/PreCompact -> remem summarize`。Codex：`SessionStart -> remem context`（带 `REMEM_CONTEXT_CODEX_HOOK_JSON=1`），`PostToolUse`（matcher `Bash`，timeout 3000）`-> remem observe`，`Stop`。`include_observe` 仅 ClaudeCode。[source: src/install/config.rs:28-140]
- hook 热路径入口是独立 CLI 子命令（`observe` / `session-init` / `context` / `summarize`），由 host 以子进程方式调用。`observe` 读取 stdin 事件并写 DB（capture / enqueue），不适合承担「无 DB 写」的 evaluator。[source: src/observe/hook.rs, src/cli/types.rs:70-100]
- 后台 worker：`src/worker.rs::process_job` 按 `job.job_type` 分发（summary / compress / dream…），`run()` 是 claim-next-job 循环。这是 sleep-time 编译应挂载的位置。[source: src/worker.rs:42-178]
- CLI 子命令定义在 `src/cli/types.rs`（`Commands` enum，clap `Subcommand`），子命令实现在 `src/cli/actions/`，分发在 `src/cli/dispatch.rs`。[source: src/cli/]
- doctor 报告：`src/doctor/`（`report.rs::run_doctor`、`environment.rs` 等）。[source: rg doctor src/]

## 2. 设计原则

D1. 谓词模型窄且确定：v1 只支持一个固定的白名单谓词种类集合，不引入任意正则 / 任意脚本。

D2. 计算分层：**编译**（可能用 LLM、可写 DB、可慢）只在 worker；**求值**（纯谓词、只读工件、必须快）只在 hook。两侧互不越界。

D3. Fail policy 分级：warn 规则求值失败 fail-open + error-level log；block 规则的失败策略需人工在 spec 审批时确认（默认建议 fail-open 以保护延迟，但必须 doctor 可见）。

D4. Additive：编译检查不替换 injection；来源 memory 仍照常参与 SessionStart 注入。

D5. 单一真实来源：规则工件由 worker 从 live active memory 全量重建（rebuild），而不是增量打补丁，避免 stale 规则残留。

D6. High-context 文件手动更新：README / SKILL / AGENTS 不由 generator 静默改写。

## 3. Proposed Design

### 3.1 编译规则工件 JSON schema

路径：`~/.remem/compiled_rules/<project_id>.json`（`project_id` 沿用 `src/project_id.rs` 的既有规范化）。文件是一个对象，顶层带元信息 + 规则数组：

```json
{
  "schema_version": 1,
  "project_id": "abc123",
  "compiled_at_epoch": 1770000000,
  "compiler_version": "0.x.y",
  "rules": [
    {
      "rule_id": "r_<stable_hash>",
      "predicate": {
        "kind": "command_prefix",
        "hook_events": ["PostToolUse"],
        "tool_matcher": "Bash",
        "field": "command",
        "match": "npm install",
        "suggest": "该项目约定用 bun，不要用 npm"
      },
      "action": "warn",
      "enabled": true,
      "expires_at_epoch": null,
      "provenance": {
        "source_memory_id": 4213,
        "reinforcement_count": 4,
        "source_hash": "<memory_content_hash>",
        "compiled_at_epoch": 1770000000
      }
    }
  ]
}
```

字段说明：

- `rule_id`：稳定 id，= hash(`source_memory_id` + `predicate` 规范化)，用于 CLI disable / expire 定位。
- `predicate.kind`：v1 白名单谓词种类（见 3.2）。
- `predicate.hook_events`：该规则在哪些 hook 事件求值（`PostToolUse` / `UserPromptSubmit`）。
- `predicate.tool_matcher`：需要匹配的工具名（如 `Bash` / `Edit`），仅 `PostToolUse` 用。
- `predicate.field`：从工具输入中取哪个字段（如 `command` / `file_path` / `content`）。
- `predicate.match`：匹配串 / glob。
- `predicate.suggest`：违反时给用户看的建议文本。
- `action`：`warn`（默认）或 `block`（opt-in）。
- `provenance`：`source_memory_id` + `reinforcement_count` + `source_hash`（用于 supersede 检测）+ `compiled_at_epoch`。

### 3.2 确定性谓词模型（v1 machine-checkable 范围）

v1 只支持四种谓词，全部是对单个工具输入字段的确定性判定，无回溯正则、无跨事件状态：

| kind | 语义 | 典型场景 |
|---|---|---|
| `command_exact` | `field` 值经 token 规范化后等于 `match` | 精确命令禁令 |
| `command_prefix` | `field` 值（trim 后）以 `match` 为前缀 token 序列 | "use bun not npm" → 命令以 `npm install` 开头 |
| `path_glob` | `field`（文件路径）匹配 glob `match` | "不要改 dist/**" |
| `field_contains` | `field` 值包含子串 `match` | "no Co-Authored-By" → commit message 含该串 |

超出这四种的（自由形式、需要语义理解、跨多步条件）一律不编译，保持 injection-only（对应 N1 / N6 / B1）。谓词匹配区分大小写策略与 token 规范化规则在实现时固化到一个纯函数 `eval_predicate(&Predicate, &ToolInput) -> Option<Match>`，便于单测。

### 3.3 编译在哪里跑（worker sleep-time job）

- 新增 job type `compile_rules`（在 `src/worker.rs::process_job` 的 match 分支里加一支，参照 `dream` / `compress`）。
- 触发：worker 在处理 memory 变更 / dream / supersede 后 enqueue 一个 per-project `compile_rules` job；也可由定时 drain 周期性重建。
- 编译流程（全部在 worker，可用 LLM、可读 DB）：
  1. 选取候选：`memory_lessons` 中 `reinforcement_count >= N`（config，默认 3）且来源 memory `status = 'active'`。
  2. 谓词抽取：把 preference 文本映射成 3.2 的白名单谓词。v1 首选启发式规则映射（bun/npm、Co-Authored-By 等已知模式）；如需覆盖更多 preference，可加一个**worker 内**的窄 LLM 抽取步骤，输出必须落在白名单谓词种类且带 confidence，低于阈值或无法结构化的直接丢弃（不违反 N2，因为不在 hook 热路径）。
  3. low-risk 过滤：只保留 warn 级安全谓词；block 需规则显式标注。
  4. 全量 rebuild：把该 project 的规则数组整体重写到工件（D5），原子写（temp file + rename）。
  5. 记录编译状态到 DB（见 4.1）供 doctor 读取。

### 3.4 hook 侧纯谓词 evaluator（无 LLM / 热路径无 DB 写）

- 新增 CLI 子命令 `remem enforce --host <h> --event <PostToolUse|UserPromptSubmit>`，作为**额外**的 hook 挂到对应事件（`build_hooks` 增补），与现有 `observe` 并存。
- 行为：
  1. 从 stdin 读取 host 的 hook 事件 JSON，解析出 project + tool name + tool input 字段。
  2. 读取 `~/.remem/compiled_rules/<project>.json`（只读；文件缺失 = no-op，正常返回）。
  3. 对每条 `enabled && 未过期` 且 `hook_events` 包含当前事件的规则，跑 `eval_predicate`（纯函数）。
  4. 命中 warn → 输出 host 约定的 warning（Claude Code：`systemMessage` / `additionalContext` 警告文本），退出 0。
  5. 命中 block（opt-in）→ 输出 Claude Code block 决策（`{"decision":"block","reason":...}` 或 exit code 2），阻止该工具调用。
- 硬约束：该路径**不** `db::open_db()`、**不**调用任何 AI（B2 / N2）。延迟预算：一次文件 read + JSON parse + O(rules) 谓词匹配，目标个位数毫秒；p95 hook 延迟不变（AC2）。
- Fail policy：工件损坏 / parse 失败 → error-level log（U-29，不能 warning+假成功），warn 规则 fail-open（放行），block 规则失败策略按 D3 / PRODUCT Q4 待审批确认。

### 3.5 decompile-on-supersede 触发

- `soft_supersede_routed` 及 suppress 路径把 memory 置 `stale` 后，enqueue 一个该 project 的 `compile_rules` job。重建时该 memory 不再是 `active`，其规则自然从工件中消失（D5 → B6）。
- 额外快速路径：`remem compiled-rules disable <id>` 直接改写工件里该规则的 `enabled=false`（立即生效，B5），不必等 worker。
- evaluator 侧不校验 memory status（它不碰 DB）；一致性由 worker rebuild + `source_hash` 保证。若 `source_hash` 与当前 memory 内容不符（内容被改写），下次 rebuild 会替换/移除该规则。

### 3.6 CLI 增补

在 `src/cli/types.rs` 的 `Commands` 增加 `CompiledRules { action: CompiledRulesAction }`，子命令：

- `list [--project <id>]`：列出规则 + provenance（source memory id / reinforcement count / compiled_at / action / enabled）。
- `disable <rule_id>`：写工件 `enabled=false`（立即生效）。
- `expire <rule_id>`：写工件 `expires_at_epoch=now`（等效永久失效，保留 provenance 供审计）。

实现放 `src/cli/actions/`，分发在 `src/cli/dispatch.rs`。工件读写走一个共享模块 `src/rules/` 供 worker（编译）、CLI（管理）、enforce（求值）复用，避免三处重复（Anti-Duplication）。

### 3.7 doctor 增补

`src/doctor/` 增加一节：读取编译状态表（4.1）与工件，报告：
- 每 project 编译规则计数（enabled / disabled）。
- 最近一次编译时间戳 + 状态（ok / failed + 原因）。
- 工件损坏 / 不可读时以 error-level 呈现（对应 B7）。

## 4. Schema / Migration 需求

### 4.1 编译状态表（doctor + last-compile）

新增 migration `v035_compiled_rules_state.sql`：

```sql
CREATE TABLE compiled_rules_state (
    project_id TEXT PRIMARY KEY,
    last_compile_epoch INTEGER NOT NULL,
    status TEXT NOT NULL,          -- 'ok' | 'failed'
    rule_count INTEGER NOT NULL,
    last_error TEXT
);
```

- 规则本体存文件工件（hook 只读文件，避免热路径 DB）；DB 只存编译状态，供 doctor 与 worker 用。
- `reinforcement_count` 复用现有 `memory_lessons`（v017），无需新列。
- 不需要为「machine-checkable」加列——该判定在编译期由谓词抽取决定。

## 5. Product Requirement Mapping

| Product 项 | 技术响应 | 验证 |
|---|---|---|
| P1 编译条件 | worker `compile_rules` 选 `reinforcement_count>=N` + 谓词可抽取 + low-risk | worker 单测（选取 + 谓词抽取 + rebuild） |
| P2 evaluator | `remem enforce` 纯谓词、只读工件 | evaluator 单测 + 无 DB/无 LLM 断言 |
| P3 provenance / CLI | 工件带 provenance；`compiled-rules list/disable/expire` | CLI 测试 + list 输出断言 |
| P4 编译只在 worker | `compile_rules` job type | AC7 测试：hook 路径不编译 |
| P5 additive | injection 路径不变，enforce 叠加 | 既有 context 注入测试保持绿 |
| P6 decompile | supersede → enqueue rebuild；disable 直写 | AC5 测试 |
| P7 延迟 | 文件读 + O(rules) 谓词 | AC2 延迟测量 |
| P8 doctor | `compiled_rules_state` + doctor 节 | AC6 doctor 测试 |

## 6. Fixture / Eval 设计（证明违反率下降）

- 参照 `src/eval/`（injection eval `src/eval/injection/`、e2e `src/eval/e2e.rs`）新增 repeated-correction fixture 套件：
  - 场景 A："use bun not npm" → 反复纠正后编译成 `command_prefix npm install`。
  - 场景 B："no Co-Authored-By" → 编译成 `field_contains Co-Authored-By`（commit message / Edit content）。
- 每个场景跑两条 arm：**injection-only baseline**（不启用 enforce）与 **compiled-check**（启用 enforce），统计模拟工具调用序列中的违反率，断言 compiled-check 显著更低（AC1）。
- 延迟基准：对 evaluator 跑 N 次采样，报告 p95，与「无规则文件」基线对比，断言无实质回归（AC2）。

## 7. Affected Files（预期实现面）

- `src/rules/` （新模块：工件 schema、读写、`eval_predicate` 纯函数、谓词抽取）
- `src/worker.rs`（新增 `compile_rules` job 分发）+ `src/db/job.rs`（job 枚举）
- `src/cli/types.rs` / `src/cli/actions/` / `src/cli/dispatch.rs`（`enforce`、`compiled-rules` 子命令）
- `src/observe.rs` 或新 `src/enforce.rs`（enforce 入口，纯谓词、只读文件）
- `src/install/config.rs`（`build_hooks` 增补 enforce hook 挂载）
- `src/memory_candidate/apply.rs`（supersede/suppress 后 enqueue rebuild）
- `src/doctor/`（编译状态报告）
- `src/migrations/v035_compiled_rules_state.sql`
- `src/eval/`（repeated-correction fixture + 延迟基准）
- README / `plugins/remem/README.md` / `plugins/remem/skills/remem/SKILL.md`（手动更新，描述 enforce 行为）

## 8. 拆分实现 issue（per-module）

1. **schema/工件**：`src/rules/` 模块 + `v035` migration + `eval_predicate` 纯函数与谓词模型（含单测）。
2. **worker/编译**：`compile_rules` job + 候选选取 + 谓词抽取（启发式，LLM 抽取列为可选后续）+ 原子 rebuild + 状态写入。
3. **hooks/evaluator**：`remem enforce` 子命令 + `build_hooks` 挂载 + 无 DB/无 LLM 断言 + 延迟基准。
4. **CLI/doctor**：`compiled-rules list/disable/expire` + doctor 报告 + supersede→rebuild 触发。
5. **eval/fixture**：repeated-correction 套件 + 违反率对比 + p95 延迟对比。

## 9. 验证

完成前：

```bash
cargo fmt --check
cargo check
cargo test
```

涉及 plugin / 运行时改动时还需（见 AGENTS.md）：

```bash
python3 scripts/ci/check_plugin_version_sync.py
node --test plugins/remem/scripts/remem-runtime.test.js plugins/remem/apps/remem/server.test.js
```

针对本 spec 的重点测试：

```bash
cargo test rules::            # 谓词求值 + 工件读写
cargo test worker::           # compile_rules job
cargo test eval::             # repeated-correction fixture + 延迟基准
```

W-16：完成声明必须附本轮新命令输出，不得引用旧的 pass。

## 10. Rollback Plan

若 enforce 引入延迟回归或误 block：
1. 从 `build_hooks` 移除 enforce hook 挂载（injection 与 observe 不受影响）。
2. 保留 schema / 工件（惰性，无副作用）。
3. 重跑 `cargo check` + evaluator/worker 重点测试。

不得通过弱化 duplicate gate、静默吞掉编译错误、或删除 memory 抽取来回滚。

## 11. Human Gate

本 spec 为 draft。实现开始前需 maintainer 确认：谓词抽取路径（启发式 vs worker-LLM，Q1）、Codex 侧 enforcement 覆盖范围（Q2）、config 默认值（Q3）、block 失败策略（Q4）。
