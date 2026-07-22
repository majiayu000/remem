# Task Plan

## Linked Issue

GH-813

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`

## 实现任务

- [x] `SP813-T1` Owner: agent; Dependencies: spec PR approval; Files: `docs/specs/preference-rule-compilation/PRODUCT.md`, `docs/specs/preference-rule-compilation/TECH.md`, `specs/GH671/product.md`, `specs/GH671/tech.md`; Done when: GH-671 authoritative contract 与 SpecRail packet 枚举完整 eligibility allowlist、两个独立 risk 来源和 unknown-value fail-closed 要求；Verify: GH671/GH813 workflow spec checks、`git diff --check`。本任务由 spec PR #814 exact head `c1e79e7e0cb880e2081d31b5c0bae63b4ee30cc8` 完成，merged as `e4867a0517df0d0e8487ac20d702d7a5444c321a`，CI `check` SUCCESS。
- [ ] `SP813-T2` Owner: agent; Dependencies: `SP813-T1` `SP813-T5`; Files: `src/rules/compiler.rs`, `src/rules/compiler/tests.rs`, `src/rules/compiler/eligibility_tests.rs`, `src/rules/compiler/sweep_tests.rs`; Done when: 对新的 enforcement-sensitive 实现，machine-readable sensitive gate 必须在实现开始前生效；compiler 以 typed policy 集中执行 fail-closed eligibility，修正 global-owner 过宽问题，project owner 严格按 `target_project` → `owner_key` → legacy `project` 解析且高优先级错配不能被低优先级掩盖，完整正例、逐维反例、两个独立 risk、closed-enum coverage、unknown values 和关键交叉状态均由行为测试覆盖，且不使用 SQL 字符串 snapshot；Verify: 历史 PR #906 已在 T5 生效前合并，属于本 issue 正在治理的顺序违规，不能追溯性改写为“实现前已通过 gate”。T5 落地后，把 first-parent range `2e23eb64e2cf710d36e48e77e2ed414de7fe741e..f89c5c96d1c6a01d819625b2c6bae51f573d1cf9`、approved spec merge SHA 和由该 range 机械提取的完整 changed-path list 绑定到 JSON artifact；使用 `PYTHONPATH=checks python3 -c` 调用 `classify_sensitive_changes(load_pack(repo), repo, ["CHANGELOG.md", "Cargo.lock", "Cargo.toml", "docs/specs/preference-rule-compilation/PRODUCT.md", "docs/specs/preference-rule-compilation/TECH.md", "npm/remem/package.json", "plugins/remem/.codex-plugin/plugin.json", "plugins/remem/runtimes/remem-releases.json", "server.json", "specs/GH671/tasks.md", "src/rules/compiler.rs", "src/rules/compiler/eligibility_tests.rs", "src/rules/compiler/sweep_tests.rs", "src/rules/compiler/tests.rs"], ["specs/GH813/product.md", "specs/GH813/tech.md"], source="github_changed_files")` 与 `classification_from_approved_tech(..., issue=813, base_sha=<approved-spec-merge-sha>)`，两份 machine-readable 输出都必须为 `enforcement_sensitive=true`。classification 输入必须与 range 的 `git diff --name-only` 集合完全相等，不能只分类 compiler 子集；artifact 必须分字段保存 approved spec merge SHA、replay-time live `origin/main` SHA、exact changed-path range、完整 path list、集合相等 verdict 与两份 classification 输出。该 artifact 只补偿证明历史 diff 在现行 registry 下会被识别为 sensitive；不得对 #906 运行或解释当前完整 `implement` route decision，因为 readiness label、duplicate-work、branch ownership 和 evidence freshness 都是 prospective pre-implementation controls，把当前状态回填到历史 PR 会制造虚假合规。完整 required route gate 的 live 证据归 `SP813-T5`，不得用 `check_pr_tier.py` 代替 classification。最后对 #906 exact head 运行 `cargo test rules::compiler -- --nocapture`, `cargo fmt --check`, `cargo check` 和独立代码 review。只有两份分类、行为测试与审查均通过时才可作为 compensating evidence 勾选 T2；该回放不改变 #906 当时的非合规事实。
- [x] `SP813-T3` Owner: upstream SpecRail contributor; Dependencies: `SP813-T1`; Files: upstream workflow/checkpoint/PR-evidence schema、sensitive registry input、review artifact schema、review JSON gate、GitHub evidence/thread rollup、pre-merge PR gate、executable closure audit/merge-wrapper 及其 tests; Done when: machine-readable `enforcement_sensitive` 分类被 route/pr gate 强制；独立审查证据要求 terminal state、reviewer lane、exact head、开始/完成时间、非空结果、显式 clean/non-blocking verdict，并证明 current-head artifact 没有 blocking/actionable finding，`changes_requested`、其他 blocking verdict 及没有对应 GitHub thread 的 artifact-only actionable finding 均阻止 merge-ready；新 head artifact 完整 carry forward 上一 head findings 的稳定 ID、resolved/unresolved/obsolete 状态，以及 resolved/obsolete 的关闭证据，unresolved 继续阻断；resolved actionable thread 验证 resolver identity/role，只有原 reviewer、带可验证 re-review evidence 的 successor reviewer lane 或获授权 human maintainer 能清除阻塞，implementer/orchestrator/coordinator/unknown 不能清除阻塞；pre-merge gate 不要求未来 dispatch evidence，dispatch 后的 wrapper/audit 验证 gate-before-dispatch 同-head 顺序并为外部违规 merge 输出 schema-valid machine-readable violation/`required_follow_up` payload；上游只拥有检测与 payload contract，不直接创建 remem GitHub issue；self-review 只有在存在已验证 reviewer-lane failure、同一 PR/head 的独立人类授权且后续 human final review 仍必需时才有效，其余失败/取消/过期/时序状态全部 fail closed；Verify: `python3 -m pytest tests/test_pr_gate.py tests/test_github_pr_evidence.py tests/test_review_json_gate.py tests/test_runtime_ledger_gate.py tests/test_closure_audit.py`，负例覆盖缺失/冲突 sensitive 标记、sensitive 无批准 spec、`changes_requested`、其他 blocking verdict、current-head artifact-only actionable finding、无 lane failure、无同 head 人类授权、缺 human final review、stale head、prior finding 缺失/仍 unresolved/无关闭证据、implementer/orchestrator/coordinator/unknown resolver、successor reviewer 缺 re-review evidence、dispatch-before-gate 和外部 merge 缺证据链，并有 clean/non-blocking + zero-blocking-findings、完整 finding carry-forward、合法 resolver 与 pre-merge 无 dispatch evidence 的正例。上游 GH97 已由 PR #103/#115/#116 完成；维护者于 2026-07-21 授权 remem 固定包含这些能力的 exact SHA `0f903abe1794899071a9f19a4c46af1ce81129d3`。
  T3 maintainer-asserted live evidence（human final review 必须重新查询）：upstream SHA `0f903abe1794899071a9f19a4c46af1ce81129d3`；PR #103/#115/#116 均 merged 且各自 `workflow-check` SUCCESS；clean checkout 在 2026-07-21 运行 T3 指定的五个 pytest 文件并得到 `254 passed in 7.12s`；SHA-specific durable authorization 为 GH-813 comment `issuecomment-5030044760`。
  T3 responsibility boundary: upstream closure audit 只消费并验证调用方提供的标准化 merge/gate
  evidence；GitHub commit pagination、evidence completeness、trusted-base selection 和 checkout
  binding 明确属于 remem-local `SP813-T5`。这不是对已完成 T3 SHA 的追溯性扩展，且 T5 在这些
  adapter 证据与负例通过前不得完成。
- [x] `SP813-T4` Owner: agent; Dependencies: `SP813-T3` upstream release or durable maintainer-authorized exact SHA; Files: `checks/specrail-sync.lock.json`、由 `scripts/sync-specrail-checks.sh` 管理的 synced files、remem sync tests; Done when: remem 固定并同步已发布的上游门禁或维护者授权的 exact SHA，未对 vendored checks 留下手工漂移；exact-SHA authorization 必须记录 actor、full SHA、scope、time 和同仓库 durable URL，且实现证据中的 SHA 必须完全一致；Verify: 当前 gate 不解析 authorization comment。human final review 必须 live read-back durable URL 并人工核对 actor/scope/time/full-SHA；自动检查只通过 `scripts/sync-specrail-checks.sh --verify`、相关 Python tests 和 `python3 checks/check_workflow.py --repo .` 验证 lock SHA、tracked closure、hash 与 workflow。History: PR #892 首次同步后因 schema contract 不兼容导致 red main，PR #893 revert；GH-894 spec #895 后由 PR #897 exact head `5ca704454e618473e74ebb6c8c15ecbf77158797` 修复并以 maintainer-asserted CI run `29683610679` SUCCESS 重新落地。Implementation evidence: exact-SHA authorization was live-read from `issuecomment-5030044760`; sync lock verification, schema contract, and workflow checks passed in PR #908.
- [x] `SP813-T5` Owner: remem workflow integration agent; Dependencies: `SP813-T4`; Files: `workflow.yaml`, `.github/workflows/ci.yml`, `.github/pull_request_template.md`, `CONTRIBUTING.md`, `CHANGELOG.md`, `Cargo.toml`, `Cargo.lock`, `npm/remem/package.json`, `plugins/remem/.codex-plugin/plugin.json`, `plugins/remem/runtimes/remem-releases.json`, `server.json`, `schemas/sensitive_implement_gate_result.schema.json`, `scripts/ci/run_sensitive_implement_gate.py`, `scripts/ci/test_run_sensitive_implement_gate.py`, repo-local workflow controller/adapter and integration tests; Done when: remem 注册 machine-readable sensitive spec/path，CI 调用同步后的 gate/audit，在任何 compiler 实现开始前让 `SP813-T2` 路径被识别为 `enforcement_sensitive=true` 且要求 approved spec，`enforcement_sensitive` 无 fast path、exact-head review/prior-findings/thread-resolver 顺序、advisory 与 server-side protection 边界已记录；用户可见 compiler correctness fix 按仓库 version-bump policy 只做一次一致的 unreleased source-version staging，所有发布元数据保持同步且不实际发布；由有 GitHub 写权限的 remem controller 消费上游 `required_follow_up` payload，通过 GitHub Issues API 按 `repository + pr_number + final_head_sha + violation_code` 稳定幂等键创建、重新打开或复用 durable issue，并回读 issue number、URL、open state 与幂等键写入 closure evidence。只有 artifact、写入失败或回读失败均保持 closure blocked，不得声称 follow-up 已创建；Verify: remem-local wrapper 接收 `--github-repo <owner/name> --issue <number> --pr <number> --head-sha <full-sha>`，校验 local origin，live 回读同 repo/open PR/exact head/head ref/linked issue，并自行查询 maintainer readiness label event 的 actor/timestamp；wrapper 还必须用 live GitHub repository permission/role API 证明 actor 类型为非 Bot/App/agent 的 `User`，且是 owner 或拥有 `admin|maintain` 权限，保存 actor type/role/permission/authority source；无权限、未知权限或 API 失败均阻断。`github_issue_evidence.py` 只提供其实际 schema 字段，不得被误写成 actor/time/authority 来源。wrapper 校验 `state_source=label`、`state_trusted=true`、duplicate-work schema 与 300 秒 freshness，保存 repository/remote identity 和原始 duplicate artifact；它只排除已验证的当前 PR + exact head 以及该 PR API payload 唯一确定的 remote head branch 自引用，记录 exemption 与原始/过滤后 PR/branch artifacts/hashes，任何其他 PR/remote branch 仍阻断。wrapper 对 route 输入分别计算 pre/post SHA-256，并只用固定数组 argv 调用 `route_gate.py --route implement --issue <issue> --mode required --evidence <issue-evidence> --duplicate-evidence <filtered-duplicate-evidence> --json`；wrapper 不接受且子进程不得出现 `--state`/`--label`。gate 后任一 hash 变化即阻断；输出前重新读取当前 active readiness event，并要求 event identity、actor type、owner/admin/maintain 权限与 authority source 和首次验证一致；schema-valid current-head durable result 保存 wrapper identity/timestamps、repository/remote/PR/head/head-ref binding、首末 label event/authority、self-exemption、fixed argv、pre/post hashes、trust/freshness verdict 与 nested gate JSON，并要求 `decision=allowed`、`missing=[]`、`sensitive_classification.enforcement_sensitive=true`。CI 与 PR evidence adapter 只接受该 wrapper result，裸 route-gate JSON 即使 allowed 也无效。wrong repo/remote/PR/head/head-ref、Bot/App/agent 或非 maintainer labeler、active label event 被替换、非当前冲突 PR、其他匹配远端分支、stale evidence 或未保留的 branch cleanup/ownership decision 均阻断；冲突前后 evidence 与 human actor/time/rationale/decision URL 必须进入 durable artifact，agent 不得删除分支制造绿灯。workflow/schema/gate integration tests 覆盖上述正负例；closure-audit/controller fixtures 覆盖首次创建、重复运行复用同一 issue、已关闭 issue 重新打开、artifact-only、API 写入/回读失败，且持久化正例能由 fake GitHub adapter 回读匹配的 open issue；`python3 scripts/ci/check_plugin_version_sync.py`、`python3 checks/check_workflow.py --repo .`、`git diff --check`。这些是高上下文文件，必须单独复核，不能由生成器静默修改。
  - `SP813-T5` authoritative Files addendum: `.github/workflows/sensitive-governance.yml`、
    `.github/workflows/closure-audit.yml`、`scripts/ci/test_sensitive_governance_workflow.py` 和
    closure workflow/controller focused tests 由 T5 owner 负责实现和验证，不能因
    planned-change manifest 已列出这些路径就省略任务所有权。
  - `SP813-T5` authoritative prospective-evidence addendum: wrapper 必须证明 implementation PR
    属于同一 repository 且非 fork，并要求 head repository/ref 对应的 remote-tracking ref 存在且
    解析为 exact head；exemption artifact 保存 ref 与解析后的 commit SHA，missing/stale/mismatched
    remote-tracking commit 一律 fail closed。输出 `allowed` 前必须再次 live 读取 PR open/state、
    repository/head repository、head ref、exact head、linked issue 与 body sensitive evidence，逐字段
    对比首次保存值，任何 identity/body/state drift 都阻断；focused tests 必须覆盖上述负例和首末
    PR evidence 完全一致的正例，不能只复核 readiness label event。
  - `SP813-T5` authoritative changed-file addendum: prospective PR file collector 必须遍历全部可用
    GitHub API pages，把 collected count 与 live PR `changed_files` total 绑定；超过 API 可证明上限、
    count mismatch、截断、分页/API error 或无法证明 completeness 时必须 fail closed，部分 file
    list 不得进入 classifier。`scripts/ci/test_sensitive_governance_workflow.py` 与 workflow focused
    tests 由 T5 owner 覆盖这些负例。
  - `SP813-T5` authoritative closure-evidence addendum: PR commit collector 必须遍历全部 API
    pages，把 collected count 与 live PR total count 绑定，并在分页截断、超限、count 漂移、API
    error 或无法证明 completeness 时 fail closed。trusted-base selector 必须使用完整 commit set、
    merge structure 和 merge-method evidence 排除 PR-controlled parent；multi-commit rebase、partial
    commit evidence 与缺失 dispatch-time live-base snapshot 的负例都必须阻断。上述 acquisition 和
    checkout selection 是 T5 的 required adapter gate，不由 T3/T4 sync 自动满足。
  - `SP813-T5` authoritative Verify addendum: 完成 version staging 前先执行
    `git fetch --prune origin`，将 `<live-base-sha>` 固定为当次
    `git rev-parse origin/main` 返回的完整 SHA 并持久化到验证证据；必须先以
    `git merge-base --is-ancestor <live-base-sha> HEAD` 证明 implementation head 已包含该 live
    merge target，再运行 `python3 scripts/ci/check_version_bump.py <live-base-sha> HEAD`。若
    `origin/main` 在 final gate 前变化，必须重新合入 live base、重新采集 SHA 并重跑；该 gate 与
    `python3 scripts/ci/check_plugin_version_sync.py` 均通过后才能勾选 T5。
- [ ] `SP813-T6` Owner: human repository administrator; Dependencies: spec approval and protection policy decision; Files: GitHub repository settings plus an external trust root; Done when: 明确记录是否需要不可绕过保护；普通 GitHub Actions required status check 只算 advisory/accidental-bypass mitigation。若需要不可绕过保护，个人仓库安装仓库外独立 GitHub App 并将 ruleset expected source 固定到该 App，或迁入组织后启用受保护治理仓库的 organization required workflow；留下 live API、来源绑定、外部 closure audit 与拒绝缺少可信 gate 的 merge 证据。agent 不执行权限变更、批准或合并；Verify: live GitHub branch-protection/ruleset/source API 与一次缺少可信来源 check/workflow 时被拒绝的 merge 证据，或明确记录保持 advisory-only 的管理员决定。
- [ ] `SP813-T7` Owner: agent; Dependencies: `SP813-T1` `SP813-T2` `SP813-T3` `SP813-T4` `SP813-T5` and recorded `SP813-T6` decision; Files: GH-813 closure evidence only; Done when: Product acceptance criteria 全部有 exact-head 证据，CI、review、thread、protection capability 和 closure audit 结果被记录；若存在 `B-010` violation，closure evidence 还必须含由 GitHub API 回读且与稳定幂等键匹配的 open follow-up issue number/URL/state，只有本地 artifact 时 #813 不得关闭；Verify: `cargo fmt --check`, `cargo check`, focused tests, `cargo test`, workflow checks, SpecRail sync verify、live PR evidence gate，以及对 follow-up issue 的 live GitHub API read-back。

## 并行拆分

- `SP813-T3` 位于上游 SpecRail；`SP813-T4` 必须等待其发布或带 actor/scope/time/full-SHA
  的 durable maintainer authorization，`SP813-T5` 再完成 remem registry/CI 集成。未来
  compiler 变更只有在敏感门已生效后才能开始。历史 PR #906 已违反该顺序，因此本计划
  不再把它描述为尚未开始；T2 的未勾选状态表示补偿性 classification、行为测试和独立
  review 尚未完成，而不是声称可以追溯重写实现时序。
- `SP813-T6` 是独立的人类管理员 lane，可并行做政策决策，但不能由 agent 代办。
- repo-local `pull_request_target` prospective workflow 每次运行必须通过 GitHub API fresh 查询
  live default/base-ref snapshot，并 checkout 该 trusted-base exact SHA；PR payload 的
  `base.sha` 只用于一致性比较，不能作为长生命周期 PR 的可信代码 checkout。closure workflow
  则从受信 merge event/API 的 default-branch merge commit、PR commit 集合和 merge-method evidence
  共同证明实际 pre-merge default-base；first-parent ancestry 本身不构成证明，多 commit rebase 的
  末 commit first parent 明确视为 PR-controlled。无法证明单一真实 pre-merge parent 时，只接受
  merge dispatch 前已持久化且绑定 PR/head 的 live base snapshot，否则 fail closed；不接受 PR
  payload `base.sha`。两者只提供 advisory 与补偿审计，不能替代
  `SP813-T6` 的外部 App/org required workflow。
- 同一 lane 内禁止两个 agent 同时写 `src/rules/compiler.rs`、`CONTRIBUTING.md` 或 synced
  SpecRail 文件；需要并行时必须先声明不重叠的文件所有权。

## 验证

- `git diff --check`
- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH813`
- `python3 checks/route_gate.py --repo . --route write_spec --issue 813 --state triaged --json`
- `scripts/sync-specrail-checks.sh --verify`
- 实现阶段：`cargo fmt --check`
- 实现阶段：`cargo check`
- 实现阶段：`cargo test rules::compiler -- --nocapture`
- 提交前：`cargo test`
- 合并准备：在 final head 上收集独立 review artifact、GitHub PR evidence、thread rollup 与
  PR gate 输出；不得使用旧 head 或调用方裸 `review_source` 作为完成证明。

## Handoff Notes

本 packet 只定义 #813 的修复与补偿性验收契约，不批准新的 runtime 实现。当前 `main`
已通过 PR #906 包含 global-owner correctness fix，但该实现先于 T5 门禁，必须如实保留为
历史顺序违规。未来同类实现必须先完成 `SP813-T3` → `SP813-T4` → `SP813-T5` 并证明
compiler 路径已受敏感门约束；#906 只能通过 T2 指定的 exact-range classification replay、
行为测试与独立 review 提供补偿证据，不能追溯性宣称当时合规。`checks/github_pr_evidence.py`、
`checks/pr_gate.py`、review schema、closure audit 等由上游 SpecRail 同步，不能在 remem 中
永久手改。2026-07-21 live API 仍显示 GitHub `main` 无 branch protection/ruleset；维护者
已选择 required-check ruleset，但 GitHub Actions 同源 check 不绑定特定 workflow/event；
即使设置后也只能宣称 advisory/accidental-bypass mitigation。独立 GitHub App expected
source 或组织 required workflow 的设置和拒绝证据仍必须由人类管理员完成。在此之前
仓库内 gate 只能宣称 advisory detection。spec PR 使用 `Refs #813`；只有 `SP813-T7`
满足后才可使用 `Closes #813`。
