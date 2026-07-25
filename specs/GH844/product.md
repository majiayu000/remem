# Product Spec

## Linked Issue

GH-844

## 用户问题

remem 的正式 CI 和本地 PR preflight 目前只对 Cargo 默认 targets 运行
`clippy -D warnings`。测试、example 或 benchmark target 中新增的 lint 因而可以在正式门禁
保持绿色时长期存在。当前 `origin/main` 已能确定性复现 11 个只在 all-targets 检查中出现的
lint，其中包括异步测试跨越 `.await` 持有同步 mutex guard 的可靠性风险。

维护者和贡献者因此不能把绿色 clippy 结果理解为“仓库全部可编译 targets 都满足 lint
契约”，本地 preflight 与历史 PR 中手工运行的更强命令也可能给出不同结论。这会增加合并后
才发现测试基础设施问题、或不同贡献者重复处理工具链漂移的概率。

## 目标

- 让正式 CI 与本地 PR preflight 对同一组 Cargo targets 执行一致的严格 clippy 门禁。
- 让 test、example 和 benchmark target 的 lint 与默认 targets 一样能够阻止合并。
- 在启用更完整门禁的同一个原子变更中清理当前已知 lint，避免把 `main` 置于必然失败状态。
- 保持现有运行时、测试语义和用户可见功能不变。

## 非目标

- 不改变 remem 的存储 schema、API、hook 协议、检索、提取或上下文行为。
- 不把 clippy 扩展为 `--all-features`，也不改变 feature matrix 或 release target matrix。
- 不通过 lint suppression、降低 lint severity 或放宽断言来获得绿色结果。
- 不借此拆分 oversized files、重构无关模块或优化测试运行时间。
- 不固定新的 Rust toolchain 版本，也不改变当前 stable toolchain 策略。

## Behavior Invariants

1. `B-001`：正式 CI 和本地 PR preflight 必须对 Cargo all targets 运行 clippy，并把所有
   warnings 当作错误；相同源码、toolchain 和 feature selection 下，两条门禁必须得到相同的
   lint 结论。
2. `B-002`：lib、bin、test、example 和 benchmark target 中任何未被现有全仓规则明确允许的
   clippy warning 都必须让门禁失败；不能因为 warning 只存在于测试代码而被忽略。
3. `B-003`：升级门禁时，当前已知的 11 个 all-target lint 必须通过行为保持的修正消除；不得
   新增 `allow`、降低 `-D warnings` 或跳过相关 target。
4. `B-004`：依赖进程环境变量的异步测试必须继续串行化对共享变量的读写，同时不得跨越
   `.await` 持有标准库同步 mutex guard；测试失败、提前返回或 panic 后不得把锁永久留在占用
   状态。
5. `B-005`：修正前后，现有测试验证的 CLI dispatch、context rendering、provider fixture、
   migration fixture、install/uninstall、native-memory 过滤和 MCP process diagnostics 行为保持
   一致；不得以删除或弱化测试覆盖换取 lint 通过。
6. `B-006`：all-targets 门禁失败时，CI 与本地 preflight 必须保留 Cargo/clippy 的原始失败
   状态和可定位诊断；不得把失败转为 warning、忽略退出码或静默继续。
7. `B-007`：默认 target clippy 仍是 all-targets 结果的子集；扩大覆盖不能使原本会失败的
   production-target lint 被排除。
8. `B-008`：没有 example 或 benchmark target、存在 ignored test、或某个 target 不运行测试
   都不影响 lint 覆盖；只要 Cargo 能构建该 target，它就属于门禁范围。

## 验收标准

- [ ] 在变更前基线可复现：默认 clippy 通过，而 all-targets clippy 报告 11 个错误。
- [ ] 变更后正式 CI 与本地 PR preflight 都执行 all-targets clippy，并保持 `-D warnings`。
- [ ] 变更后 all-targets clippy 通过，且没有新增 lint suppression 或跳过 target。
- [ ] 两个环境变量相关异步测试仍验证原行为，并满足 `B-004`。
- [ ] focused tests、格式检查、编译检查和完整测试套件通过。
- [ ] spec PR 与 implementation PR 分离，implementation 在 spec approval 和
      `ready_to_implement` 之后才开始。

## 边界情况

- stable Rust 新增 lint：all-targets 门禁应明确失败，由后续独立修复处理，不能自动降级。
- 某个 test target 需要 dev-dependency：门禁使用 Cargo 正常的 target 解析与依赖选择；依赖
  构建失败同样是失败，不伪装成 lint 通过。
- 异步测试在 `.await` 期间被取消或 panic：锁必须由 guard 生命周期释放，后续测试仍可继续。
- 并行运行多个环境变量测试：共享变量操作保持串行，不因改用 async-aware synchronization
  产生竞态。
- no-default-features、跨平台 release target 和 all-features：继续由既有 build/release matrix
  管理，本变更不声称扩大这些维度的覆盖。
- 权限、网络、loading 和 accessibility：该变更只调整本地与 CI 验证契约，不新增这些用户
  交互状态，因此不适用。

## 发布说明

这是贡献者与 CI 可靠性改进，不改变最终用户运行时行为，不需要数据迁移或用户配置变更。
实现合并后，贡献文档和 PR 交接应使用新的 all-targets clippy 命令。

本文件只定义产品契约。`spec_approval`、`ready_to_implement`、最终 PR review、merge 和 release
继续保留为 human gates。
