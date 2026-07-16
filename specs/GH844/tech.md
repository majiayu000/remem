# Tech Spec

## Linked Issue

GH-844

## Product Spec

[`product.md`](product.md)

## Codebase Context

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| GitHub CI | `.github/workflows/ci.yml` | stable Rust 上运行 `cargo clippy -- -D warnings`，不构建 test/example/benchmark targets | 正式 merge gate 的覆盖来源 |
| 本地 PR preflight | `scripts/ci/check_pr_preflight.py` | fast steps 使用与 CI 相同的默认-target clippy 命令 | 必须与远端门禁保持同一契约 |
| Agent 验证命令 | `AGENTS.md` | CI 命令清单仍记录默认-target clippy | 高上下文手工维护文件；实现时需显式同步并人工 review |
| 机械 lint 修复 | `src/cli/actions/procedures.rs`, `src/context/invocation.rs`, `src/context/prompt_submit.rs`, `src/eval/provider_comparison/tests.rs`, `src/migrate/tests_schema_drift.rs`, `src/doctor/mcp_processes.rs` | all-targets 下产生 7 个可由等价标准库表达消除的 lint | 保持断言与测试语义，禁止 suppression |
| 模块布局 lint | `src/install/runtime.rs`, `src/observe/native.rs` | test module 之后仍定义 production item | 需仅移动 item/test module 边界，不改可见性或调用图 |
| 异步测试锁 | `src/observe/hook.rs` | 四个测试共享 `std::sync::Mutex`；其中两个异步测试持有 guard 跨 `.await` | all-targets 报告两个 `await_holding_lock`，也会阻塞 executor thread |
| 版本同步 | `Cargo.toml`, `Cargo.lock`, `plugins/remem/.codex-plugin/plugin.json`, `plugins/remem/runtimes/remem-releases.json`, `npm/remem/package.json` 及版本同步器声明的其他 surfaces | 任何 `src/**` 变更需要版本 bump 并保持发行面一致 | implementation PR 必须满足仓库 version gate |

## 设计方案

### 1. 原子升级门禁

在同一个 implementation PR 中完成现存 lint 修复和门禁升级。先修复但不提交独立 PR，避免
产生没有永久门禁的中间状态；也不先只改 CI，避免 `main` 必然失败。

- `.github/workflows/ci.yml` 使用
  `cargo clippy --all-targets -- -D warnings`。
- `scripts/ci/check_pr_preflight.py` 的 step 名称和 argv 同步使用该命令。
- `AGENTS.md` 的 CI 命令清单显式同步；这是人工编辑，不经生成器修改，并在 PR 中单独提示
  high-context review。
- 不加入 `--all-features`，保持当前 default feature selection 和 release matrix 不变。

### 2. 行为保持的机械 lint 修复

| Lint | 文件 | 设计 |
| --- | --- | --- |
| `manual_contains` | `src/cli/actions/procedures.rs` | 对 `Vec<&str>` 使用 slice `contains`，保留精确字符串匹配 |
| `assertions_on_constants` | `src/context/invocation.rs` | 将编译期可判定的 timeout 下限断言改为 const assertion，保留同一阈值 |
| `cloned_ref_to_slice_refs` | `src/context/prompt_submit.rs` | 使用 `std::slice::from_ref` 传入单元素借用 slice，避免两次无意义 clone |
| `needless_character_iteration` | `src/eval/provider_comparison/tests.rs` | 使用 `str::is_ascii` 的否定表达同一 CJK fixture 断言 |
| `manual_contains` | `src/migrate/tests_schema_drift.rs` | 使用 slice `contains` 保持 migration version 排除语义 |
| `useless_vec` | `src/doctor/mcp_processes.rs` | 测试 fixture 使用固定数组，调用方继续接收 slice |

不得改断言期望值、删除测试或新增 `#[allow(...)]`。

### 3. 保持模块 item 顺序合法

- `src/install/runtime.rs`：把现有 test module 移到本文件所有 production items 之后；测试内容
  与被测函数不变。
- `src/observe/native.rs`：把 `is_native_memory_markdown` production helper 放在 test module
  之前；函数实现、可见性和调用位置不变。

这些调整只改变源码布局，不改变模块导出或运行时控制流。

### 4. 异步环境变量测试锁

`src/observe/hook.rs` 的四个共享 `REMEM_ENABLE_CODEX_BASH_OBSERVE` 测试继续使用同一个静态锁，
但锁改为 Tokio async-aware mutex：

- 静态值使用 Tokio 提供的 const constructor，不引入依赖。
- 两个原同步测试转换为 Tokio async tests，使四个测试都通过 `.lock().await` 获取同一把锁。
- guard 覆盖环境变量修改、被测调用和恢复/断言的现有临界区；异步测试在 `.await` 期间仍保持
  串行，但不持有 `std::sync::MutexGuard`、不阻塞 executor thread。
- 不用缩短 guard 生命周期绕开 lint，因为在被测 async 调用完成前释放锁会重新引入进程环境
  变量竞态。

### 5. 版本与提交边界

implementation PR 采用 `per_step`，但该优化只有一个可发布 feature boundary：

1. 同一 implementation commit 包含 lint remediation、CI/preflight/AGENTS 命令同步和所需版本
   bump。
2. 测试通过后才提交；不把“先让 CI 变红”或“先修但不加 gate”拆成可独立落地的 commits。
3. spec PR 只包含 `specs/GH844/`，使用 `Refs #844`，不修改版本。

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| `B-001` | CI + preflight command parity | 静态断言/脚本测试检查两处 argv；执行 fast preflight 与 all-targets clippy |
| `B-002` | Cargo `--all-targets` gate | `cargo clippy --all-targets -- -D warnings`；验证变更前 11 项、变更后 0 项 |
| `B-003` | 9 个 Rust lint 文件 | `git diff` 检查无新增 lint allow；all-targets clippy 通过 |
| `B-004` | `src/observe/hook.rs` async lock | focused hook tests + all-targets clippy；检查共享锁覆盖完整 env 临界区 |
| `B-005` | 现有测试覆盖的各行为面 | 对 9 个文件运行 focused tests，再运行完整 `cargo test` |
| `B-006` | CI/preflight 直接执行 Cargo | preflight 失败传播现有脚本测试；命令不加 `|| true`、warning fallback 或退出码吞噬 |
| `B-007` | all-targets 是默认 targets 超集 | 默认 clippy 与 all-targets clippy 均通过 |
| `B-008` | Cargo target discovery | all-targets clippy 成功构建仓库当前 lib/bin/test/benchmark target；ignored benchmark 仍被 lint |

## 数据流

该变更不新增运行时数据流、持久化或网络调用。验证数据流为：

```text
PR source
  -> local preflight / GitHub CI
  -> cargo clippy --all-targets -- -D warnings
  -> Cargo discovers current default-feature targets
  -> rustc/clippy diagnostics
  -> exit 0 允许后续门禁；非 0 阻止 PR
```

环境变量测试中的同步流为：

```text
test task
  -> await shared Tokio mutex
  -> mutate REMEM_ENABLE_CODEX_BASH_OBSERVE
  -> await observe_input when applicable
  -> restore/remove variable and assert
  -> guard drop wakes next test task
```

不写数据库 schema，不新增持久化字段。相关 focused hook tests 仍使用隔离的
`ScopedTestDataDir`。

## 备选方案

- **保持默认-target CI，要求贡献者手工运行 all-targets**：拒绝。历史证据表明手工命令不能
  形成可靠 merge gate，当前 11 项已经绕过正式 CI。
- **只修 11 项，不升级 CI**：拒绝。无法防止同类 test-target lint 回归。
- **只升级 CI，后续再修 lint**：拒绝。会让 `main` 和所有 PR 立即进入确定性失败状态。
- **为 11 项添加 `#[allow]`**：拒绝。隐藏问题并削弱新增门禁的价值。
- **释放同步锁后再 `.await`**：拒绝。虽然可能消除 lint，但会让进程级环境变量在并行测试中
  发生竞态。
- **加入 `serial_test` 或其他测试依赖**：拒绝。Tokio 已是现有依赖，当前范围无需新增包。
- **升级到 `--all-targets --all-features`**：暂不采用。feature matrix 是不同优化议题，需要单独
  证据和影响评估。

## 风险

- Security: 不触及生产鉴权、secret 处理或外部输入。环境变量测试仍需确保临界区完整，避免
  并行测试读到错误开关值。
- Compatibility: 不改变公共 API 或运行时行为。stable Rust 新增 test-target lint 后，CI 会更早
  失败，这是预期的 fail-closed 行为。
- Performance: CI 会多编译 test/example/benchmark targets；仓库完整 `cargo test` 已编译这些
  主要 targets，Rust cache 应限制增量成本。实现 PR 记录实际 clippy wall time 作为验证证据。
- Maintenance: CI、preflight 和 `AGENTS.md` 有三处命令副本，必须同 PR 同步；未来若再调整
  clippy matrix，应保持三处一致。
- Test integrity: 机械替换和 item 移动不得更改断言或覆盖范围；异步锁不得通过缩短临界区
  制造隐蔽竞态。

## 测试计划

- [ ] Baseline proof:
  `cargo clippy -- -D warnings` 通过；
  `cargo clippy --all-targets --message-format short -- -D warnings` 精确报告 11 项。
- [ ] Focused hook tests:
  `cargo test observe::hook::tests::codex_bash_observe -- --nocapture` 与
  `cargo test observe::hook::tests::observe_ -- --nocapture`。
- [ ] Focused affected-module tests：覆盖 procedures、context invocation/prompt submit、provider
  comparison、install runtime、migration schema drift、native observe 和 MCP process diagnostics。
- [ ] `python3 scripts/ci/test_specrail_gate_wiring.py`。
- [ ] `python3 scripts/ci/check_plugin_version_sync.py`。
- [ ] `cargo fmt --check`。
- [ ] `cargo check`。
- [ ] `cargo clippy -- -D warnings`。
- [ ] `cargo clippy --all-targets -- -D warnings`。
- [ ] `cargo test`。
- [ ] `python3 scripts/ci/check_pr_preflight.py --base origin/main --pr-body-file /tmp/pr-body.md`，其中
  `/tmp/pr-body.md` 与 implementation PR body 一致。
- [ ] `git diff --check origin/main...HEAD`，并人工确认没有新增 `allow`、测试删除或无关重构。

## 回滚方案

若实现尚未合并，关闭 implementation PR 即可；spec 保留为审计记录。若实现合并后发现
all-targets 门禁产生不可接受且无法通过 cache 缓解的 CI 成本，回滚必须整体回滚 implementation
commit，使门禁命令、11 项 remediation、版本同步和 `AGENTS.md` 一起恢复，不能只撤掉门禁而
保留“已完成优化”的声明。

运行时无需 feature flag、数据迁移或恢复步骤。任何回滚仍须走独立 issue/PR、CI、review 和
merge 人工门禁。

本文件不构成 `spec_approval`。只有 maintainer 批准 spec 并把 GH-844 置于
`ready_to_implement` 后，才能开始 implementation。
