# GH671 Product Spec: 把高置信度纠偏编译成 hook 强制的确定性检查

Issue: https://github.com/majiayu000/remem/issues/671
Route: write_spec
Locale: zh-CN
Status: Draft, needs human approval before implementation
Related: #383, #617-620

## 1. 背景

remem 当前把 preference / correction 作为注入文本（injected prose）送进上下文：`SessionStart` 注入 memory context，`UserPromptSubmit` / `PostToolUse` 在 Claude Code 上做 fast-path 捕获。preference 目前只以「建议」形式存在——模型读到 "用 bun 不要 npm" 之后，仍然可以在下一个 Bash 工具调用里跑 `npm install`。

TRACE（arXiv 2606.13174, 2026）观察到：纯 memory-recall 系统里，57.5% 的可判定 preference 检查仍然被违反；而把 correction 编译成运行时强制检查后，分布外场景的违反率从 100% 降到 2.0%。

remem 在结构上是独特的：它同时拥有 **memory store**（preference 带 `reinforcement_count`、lesson metadata，见 `src/memory_candidate/apply.rs` / migration `v017_memory_lessons.sql`）和 **hook surface**（Claude Code 上 SessionStart / UserPromptSubmit / PostToolUse / Stop，Codex 上 SessionStart / Stop）。没有竞品同时握住这两侧。今天这两侧没有打通：memory 只会变成注入 prose，不会变成 hook 侧的强制门。

## 2. 问题

1. 注入文本是 advisory 的：模型可以读到 preference 又违反它，remem 没有任何确定性拦截。
2. remem 明明拥有 hook 热路径，却没有利用它对「已经被反复纠正过 N 次」的稳定 preference 做零成本、零 LLM 的确定性判定。
3. 违反发生后缺乏可观测证据：用户看不到「这条 preference 被违反了几次」，也无法把一条稳定 preference 升级成强制门。

## 3. 目标

P1. 把满足以下三个条件的 preference / correction 编译成规则工件（rule artifact，例如 `~/.remem/compiled_rules/<project>.json`）：
- (a) 被强化 `>= N` 次（`reinforcement_count >= N`，N 可配置，默认 3）；
- (b) machine-checkable：存在一个对工具输入的确定性谓词（例如命令匹配 `npm install` 而 preference 要求 bun）；
- (c) low-risk。

P2. 提供一个 hook 侧 evaluator：**无 LLM、热路径无 DB 写**，在 `PostToolUse` / `UserPromptSubmit` 事件上对照编译规则求值，默认发 warning，可按单条规则 opt-in 升级为 block。

P3. 每条编译规则携带 provenance：来源 memory id、`reinforcement_count`、编译时间戳。`remem` CLI 能 list / disable / expire 编译规则。

P4. 编译过程运行在后台 worker（sleep-time job），**绝不**在 hook 里跑。

P5. 强制是 additive 的：编译检查不替代 preference injection，两者并存。

P6. 当一条规则的来源 memory 被 supersede / suppress 时，规则自动 decompile（不再生效）。

P7. 编译检查不会改变 hook 的 p95 延迟（在噪声范围内）。

P8. `remem doctor` 报告编译规则数量与最近一次编译状态。

## 4. Non-Goals

N1. 不编译自由形式 / 有歧义的 preference（这类继续走 injection-only）。

N2. 任何 hook 热路径都不做 LLM 调用。

N3. 不替代 preference injection（enforcement 是叠加，不是替换）。

N4. v1 不做跨项目规则共享（compiled rule 按 project 隔离）。

N5. 不为消除违反而删除 / 弱化 memory 抽取或自动捕获（遵守仓库 Non-Negotiables）。

N6. v1 不引入超出白名单谓词模型的任意正则 / 任意脚本判定（保持窄且确定）。

## 5. 行为不变量

B1. 只有同时满足 P1 (a)(b)(c) 的 preference 才会被编译；不满足的一律保持 injection-only，不静默降级成弱判定。

B2. hook 侧 evaluator 只读编译好的规则工件，不打开 DB、不调用 LLM。

B3. 默认 action 是 warn；block 必须是单条规则显式 opt-in。

B4. 每条编译规则可追溯到唯一来源 memory，并带 `reinforcement_count` 与编译时间戳。

B5. `remem compiled-rules disable <id>` 立即生效（写工件），无需重启 worker 或 host。

B6. 来源 memory 变为 stale / suppressed 后，对应规则不再对用户产生 block/warn（自动 decompile）。

B7. 规则工件缺失属于正常态（= 无规则 = no-op）；工件存在但损坏时，必须 error-level 记录并可被 doctor 观测，不能假装成功。

## 6. 验收标准

AC1. 提供 repeated-correction fixture 套件（至少覆盖 "use bun not npm"、"no Co-Authored-By" 两个场景），证明 compiled-check 相比 injection-only 的违反率显著下降。

AC2. p95 hook 延迟在开启编译检查前后无实质变化（在测量噪声内），并有测量证据。

AC3. `remem compiled-rules list` 输出每条规则的 provenance（source memory id、reinforcement count、compiled_at）。

AC4. `remem compiled-rules disable <id>` 后，下一次同类工具调用不再触发该规则，且无需重启（B5）。

AC5. 一条来源 memory 被 supersede / suppress 后，其编译规则自动失效（B6），有测试覆盖。

AC6. `remem doctor` 输出编译规则计数与最近一次编译状态 / 时间戳。

AC7. 编译只在后台 worker job 中发生；有测试证明 hook 热路径不产生编译、不调用 LLM、不写 DB（针对 evaluator 路径）。

AC8. block 型规则默认不存在；仅当单条规则显式 opt-in 时才会阻断工具调用。

AC9. 实现完成前必须有本轮新验证输出（`cargo fmt --check && cargo check && cargo test` 加相关 fixture/eval），不得引用旧的 test pass。

## 7. Open Questions

Q1. 谓词抽取（把 "use bun not npm" 变成结构化 predicate）在 worker 里用启发式映射还是窄 LLM 抽取？两者都在 worker，不违反 N2，但需确认 v1 选哪条并划定白名单谓词种类。

Q2. block 型规则在 Codex 侧是否可行——Codex 目前只在 SessionStart / Stop / Bash-observe 有 hook 面，PostToolUse enforcement 是否 v1 只覆盖 Claude Code？

Q3. `N`（reinforcement 阈值）、评估的 host 集合、以及 low-risk 判定标准应放在 `~/.remem/config.toml` 的哪个段，默认值如何取。

Q4. block 规则在工件损坏 / 不可读时应 fail-open 还是 fail-closed？（warn 建议 fail-open + error log；block 需要人工确认默认策略。）
