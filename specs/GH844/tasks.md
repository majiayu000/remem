# Task Plan

## Linked Issue

GH-844

## Spec Packet

- Product: [`product.md`](product.md)
- Tech: [`tech.md`](tech.md)

## 实现任务

- [ ] `SP844-T1` Owner: implementation agent; Dependencies: human gates; Done when: baseline 与 duplicate evidence 固定；Verify: baseline commands — 固定 implementation baseline 与复现证据
  - Owner: implementation agent
  - Dependencies: maintainer `spec_approval`、GH-844 `ready_to_implement`
  - Files: none
  - Done when:
    - implementation worktree 从开始实现时最新 `origin/main` 创建，记录 head SHA、Rust/Cargo
      版本和 clean status；
    - fresh baseline 证明默认 clippy 通过、all-targets clippy 精确失败 11 项；
    - fresh GitHub duplicate evidence 证明没有引用 GH-844 的其他开放 implementation PR。
  - Verify:
    - `git fetch origin main --prune && git rev-parse origin/main && git status --short --branch`
    - `rustc --version && cargo --version`
    - `cargo clippy -- -D warnings`
    - `cargo clippy --all-targets --message-format short -- -D warnings`
    - `python3 checks/github_duplicate_evidence.py --github-repo majiayu000/remem --issue 844 --json`

- [ ] `SP844-T2` Owner: implementation agent; Dependencies: `SP844-T1`; Done when: 11 个 lint 以行为保持方式消除；Verify: focused tests + all-targets clippy — 消除现存 all-target lint
  - Owner: implementation agent
  - Dependencies: `SP844-T1`
  - Files:
    - `src/cli/actions/procedures.rs`
    - `src/context/invocation.rs`
    - `src/context/prompt_submit.rs`
    - `src/eval/provider_comparison/tests.rs`
    - `src/install/runtime.rs`
    - `src/migrate/tests_schema_drift.rs`
    - `src/observe/hook.rs`
    - `src/observe/native.rs`
    - `src/doctor/mcp_processes.rs`
  - Done when:
    - 7 个机械 lint 以等价标准库表达消除；
    - 两个 test module item-order lint 通过仅移动定义位置消除；
    - 四个 `REMEM_ENABLE_CODEX_BASH_OBSERVE` 测试共享一把 Tokio async mutex，两个同步测试
      转为 async test，完整环境变量临界区继续串行；
    - 没有新增 `allow`、依赖、测试删除、断言弱化或无关行为修改。
  - Verify:
    - `cargo test observe::hook::tests::codex_bash_observe -- --nocapture`
    - `cargo test observe::hook::tests::observe_ -- --nocapture`
    - 对 tech spec 列出的其他 affected-module tests 运行 focused filters
    - `cargo clippy --all-targets -- -D warnings`
    - `git diff -- src/` 人工检查测试语义与临界区

- [ ] `SP844-T3` Owner: implementation agent; Dependencies: `SP844-T2`; Done when: CI/preflight/AGENTS 命令一致；Verify: command parity checks — 同步正式 clippy 门禁
  - Owner: implementation agent
  - Dependencies: `SP844-T2`
  - Files:
    - `.github/workflows/ci.yml`
    - `scripts/ci/check_pr_preflight.py`
    - `AGENTS.md`
    - 必要时仅限现有 preflight/CI command parity 测试文件
  - Done when:
    - CI、preflight step name/argv、`AGENTS.md` 都使用
      `cargo clippy --all-targets -- -D warnings`；
    - 命令直接传播非零退出码，不增加 fallback、warning-only 或 skip；
    - `AGENTS.md` 修改在 PR body 中标记为 high-context manual review；
    - 不加入 `--all-features` 或改变其他 CI matrix。
  - Verify:
    - `rg -n 'cargo clippy --' .github/workflows/ci.yml scripts/ci/check_pr_preflight.py AGENTS.md`
    - `python3 scripts/ci/test_specrail_gate_wiring.py`
    - 与 command parity 相关的现有/新增 focused Python tests
    - `cargo clippy --all-targets -- -D warnings`

- [ ] `SP844-T4` Owner: implementation agent; Dependencies: `SP844-T2`, `SP844-T3`; Done when: 版本 surfaces 一致且形成原子 commit；Verify: version gates — 完成版本同步与原子 implementation commit
  - Owner: implementation agent
  - Dependencies: `SP844-T2`, `SP844-T3`
  - Files: 由 `remem-plugin-version-sync` contract 确定的全部版本 surfaces
  - Done when:
    - source version 按仓库 policy bump；
    - Cargo、Codex plugin、release manifest、npm wrapper、server/CHANGELOG 等检查器声明的
      surfaces 完全一致；
    - lint remediation、门禁升级、高上下文命令同步和版本 bump 作为同一个可发布 feature
      boundary 提交，不产生可独立落地的红色中间 commit。
  - Verify:
    - `python3 scripts/ci/check_plugin_version_sync.py`
    - `python3 scripts/ci/check_version_bump.py <base-sha> HEAD`
    - `git show --stat --oneline HEAD`

- [ ] `SP844-T5` Owner: implementation agent; Dependencies: `SP844-T4`; Done when: 完整验证通过并创建 implementation PR；Verify: full preflight + PR gate — 完整验证、实现 PR 与人工 gate handoff
  - Owner: implementation agent
  - Dependencies: `SP844-T4`
  - Files: none，除非验证发现与 GH-844 直接相关的生产修复；不得修改测试断言来消除失败
  - Done when:
    - focused、格式、编译、默认 clippy、all-targets clippy、全量测试和完整 preflight fresh
      通过；
    - implementation PR 从 implementation branch 提交，body 使用 `Closes #844` 并链接
      `specs/GH844/`；
    - PR 记录 head SHA、基线/after 对比、all-target clippy wall time、版本同步结果和
      `AGENTS.md` high-context review 提示；
    - agent review 只能作为 advisory evidence，请求 human final review，不提供 final approval、
      不 merge。
  - Verify:
    - `cargo fmt --check`
    - `cargo check`
    - `cargo clippy -- -D warnings`
    - `cargo clippy --all-targets -- -D warnings`
    - `cargo test`
    - `python3 scripts/ci/check_pr_preflight.py --base origin/main --pr-body-file /tmp/pr-body.md`
    - `git diff --check origin/main...HEAD`
    - `python3 checks/github_pr_evidence.py --github-repo majiayu000/remem --pr <pr-number> --json`
    - `python3 checks/pr_gate.py --repo . --evidence <pr-evidence.json>`

## 并行拆分

本优化不并行实现。`SP844-T2` 与 `SP844-T3` 必须在同一 integration owner 下顺序完成，因为
先单独落地任一任务都会产生“修复但无永久门禁”或“门禁确定性变红”的不完整状态。
`SP844-T4` 又修改与 source diff 绑定的共享版本 surfaces。没有可安全分配给其他 writable lane
的独立文件集合，因此不启动子 agent 或 worktree 并行写入。

## 验证

- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir=specs/GH844`
- `python3 checks/route_gate.py --repo . --route implement --issue 844 --evidence <issue-evidence.json> --duplicate-evidence <duplicate-evidence.json> --json` 返回允许实现；若返回 `needs_human` 或 `blocked`，停止。
- Product `B-001` 至 `B-008` 均在 implementation PR 中有 fresh command 或 diff evidence。
- 没有测试删除、assertion 弱化、lint suppression、新依赖或超出 GH-844 的重构。
- spec approval、`ready_to_implement`、human final review、merge 和 release gates 均保留。

## Handoff Notes

- Spec baseline: `origin/main@6e4734ccbeb11b279db1528c99099a6361facda0`，Rust 1.97.0。
- Baseline evidence: `cargo check`、默认 clippy、完整 `cargo test` 通过；all-targets clippy 精确
  复现 11 项。issue 已更正为 11 项分布于 9 个 Rust 文件。
- GH-844 当前只有 `ready_to_spec`；本 task plan 不代表 `spec_approval`，也不授权添加
  `ready_to_implement`。
- implementation 前必须重新 fetch，因为 spec review 期间 `origin/main` 可能前进；若 lint
  数量或文件集合变化，先更新 GH-844/spec，而不是把新问题静默并入。
- implementation 修改 `src/**`，必须使用 `remem-plugin-version-sync` skill 并执行其完整同步
  contract。
- `AGENTS.md` 是 high-context 文件，修改必须是显式人工 edit，并在最终 PR review 中单独核对。
- Commit policy: `per_step`；本优化只有一个原子 feature boundary，因此实现验证通过后形成一个
  implementation commit。spec PR 使用独立 commit 与 PR。
- 选定 locale 为 `zh-CN`；稳定 IDs、路径和命令保持 English。
