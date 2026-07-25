# Task Plan

Status: Planning only；不授权 runtime 实现
Date: 2026-07-23

## Linked Issue

GH-824

- Epic: GH-821
- Real-host evidence gate: GH-822
- Canonical host/hook contract dependency: GH-823；安装侧 `hosts.cursor`
  defaults/normalization、receipt 与 cleanup 由 GH-824 拥有
- Current workflow state: `ready_to_spec`

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`

本文件只固定依赖、人工门、文件 ownership、完成证据和执行顺序。创建或通过检查
`tasks.md` 不等于 spec approval，不允许提前写 Cursor runtime、测试夹具或隐藏实现。

## 实现任务

- [ ] `SP824-T0` — 依赖与人工批准门 — Owner: maintainer / real-host operator；Done when: 见下；Verify: 见下
  - Dependencies: none；Covers: B-002, B-005, B-018, B-019, B-021
  - Done when:
    1. GH-822 的 PR #914 exact head
       `c0802c42c3fc22770aecb0b7b2eec88f117f795c` 已提供 Cursor 3.12.17、隔离
       workspace、sanitized payload 与真实模型 marker evidence；人工确认采用后，
       `postToolUse: proven`、`sessionStart: blocked`、`stop/preCompact: unknown` 的
       capability 边界不得被压成统一成功状态；
    2. GH-823 product/tech amendment 已消费 PR #914 的 event/payload/status/loop 与
       capability evidence，获批准且实现已落地；runtime 提供 canonical `cursor`
       identity、closed-set hook-host parser、批准的 hook contract，以及明确的
       `session-init --host cursor` unsupported capability；GH-824 自己拥有安装侧
       `hosts.cursor` defaults/normalization、receipt 与 cleanup；若要注册
       observe bundle，GH-823 还必须提供覆盖所有 delivered failure tool names 的
       total capture-or-explicit-zero-write policy 与 failure precedence，否则
       `postToolUse`、`postToolUseFailure` 和 MCP-specific observe 全部从 builder 省略；
    3. GH-824 product/tech 获人工批准，issue 进入 `ready_to_implement`，当前 evidence
       下的 implementation route gate 返回 `allowed`。
  - Verify:
    - `python3 checks/github_issue_evidence.py --github-repo majiayu000/remem --issue 822 --json > /tmp/gh822-issue-evidence.json`
    - `python3 checks/github_issue_evidence.py --github-repo majiayu000/remem --issue 823 --json > /tmp/gh823-issue-evidence.json`
    - `python3 checks/github_issue_evidence.py --github-repo majiayu000/remem --issue 824 --json > /tmp/gh824-issue-evidence.json`
    - `python3 checks/github_duplicate_evidence.py --github-repo majiayu000/remem --issue 824 --json > /tmp/gh824-duplicate-evidence.json`
    - 审查 `/tmp/gh822-issue-evidence.json` 对应的 PR #914 real-host artifact、人工结论与
      exact evidence head；审查 `/tmp/gh823-issue-evidence.json` 对应的批准 amendment、
      合入实现 SHA
      和 focused verification。两者都不得只靠 issue number 或本地文字声明满足。
    - `python3 checks/check_workflow.py --repo . --spec-dir=specs/GH823`
    - 审查 GH-823 implementation PR 在开始实现前保存的 `route=implement` durable artifact：
      其 issue evidence 必须包含 trusted `ready_to_implement` label 与 `approved_spec`，
      decision 为 `allowed`，且 artifact 绑定 approved spec merge SHA、implementation PR
      起始 head 和完整 planned paths。GH-823 状态已推进后不得事后重跑 `write_spec` 或
      `implement` 冒充历史门。
    - `gh pr view <GH823_IMPL_PR> --repo majiayu000/remem --json state,headRefOid,mergeCommit,statusCheckRollup,url`
    - 上述 live PR evidence 必须显示 `MERGED`，merge commit 位于 trusted default branch，
      exact implementation head CI green，且与 durable pre-implementation gate artifact/
      approved-spec revision一致；只看 issue number、当前 label 或本地 packet 均不满足。
    - `python3 checks/route_gate.py --repo . --route implement --issue 824 --evidence /tmp/gh824-issue-evidence.json --duplicate-evidence /tmp/gh824-duplicate-evidence.json --artifact product_spec=specs/GH824/product.md --artifact tech_spec=specs/GH824/tech.md --artifact task_plan=specs/GH824/tasks.md --json`
    - implementation route 必须从 GH-824 trusted readiness label evidence 与当前
      duplicate-work evidence 得到 `allowed`；不得用调用方自报 `--state`/`--label`
      替代人工 readiness/spec approval。
  - Blocking rule: 任一条件未满足时，`SP824-T1` 至 `SP824-T4` 全部 blocked；不得把
    packet check、Draft task plan 或静态 Cursor JSON fixture 当作实现授权。

- [ ] `SP824-T1` — strict schema、managed ownership 与 immutable plan — Owner: schema/ownership implementation agent；Done when: 见下；Verify: 见下
  - Dependencies: `SP824-T0`；Covers: B-002, B-003, B-004, B-005, B-006, B-007,
    B-008, B-009, B-014, B-015, B-017, B-021
  - Writable ownership: Cursor host parser/plan module、Cursor user-path helper，以及
    GH-824 install-side `hosts.cursor` defaults/normalization、versioned install receipt
    schema/cleanup 与直接相关的 focused tests；只消费 GH-823 已合入的 canonical
    host/hook 合同，不在本任务重定义 canonical host 或 hook payload。
  - Done when: typed parser 校验完整 hooks v1 event/entry tree 与 MCP root；current
    builder 生成精确 Cursor MCP `{type:"stdio",command,args:["mcp"]}` 与批准 hook entry；整份 MCP
    文档的每个 foreign server 都必须匹配冻结的 stdio 或 remote exact variant，不能因
    key 非 `remem` 就跳过 malformed/unknown shape；POSIX
    binary path 使用共享 shell_quote；unsupported Windows/UNC 在 Auto 中 non-fatal skip
    且继续其他 host，在显式 Cursor/All 中于任何写入前 fail closed；`hosts.cursor`
    缺失字段精确 materialize codex/strict/true/cursor defaults，合法显式值保留、非法值拒绝；
    `capture_adapter` 只接受 cursor；整个 observe bundle 仅在 GH-823 total
    failure/precedence policy proven 后注册，否则 success/failure/MCP observe 全部缺席；
    total policy 只对未批准的 delivered failure tool zero-write，成功的未知 non-MCP
    tool 仍按 GH-823 generic contract verbatim capture；
    MCP-specific ownership 仅在 approved stable opaque specific-event per-call ID 存在时可选；
    该分支下 generic MCP delivery zero-write、只注册 afterMCPExecution，且同一 generation
    两次相同 MCP tool 调用产生不同 canonical key、各自 replay 命中原 key、双投递只产生一个
    capture；否则只能安装 generic ownership；current equality、receipt-bound old-path exact
    digest 和有限 legacy allowlist 是
    唯一 managed ownership 路径；同一 validated plan snapshot 的 foreign JSON semantic
    projection 保持，final comparison 观察到的新 external version 保留并 abort；所有
    operation/mode 共用无副作用的 `CursorConfigPlan`，malformed、collision、unsupported
    contract 在写前 fail closed。
  - Verify:
    - `cargo test install -- --nocapture`
    - whole-document 21-event command/prompt schema、event-specific matcher/
      loop_limit/failClosed、MCP remem exact-shape/extra-field、foreign stdio
      explicit-type 与 documented-example type-omitted compatibility、remote auth accepted
      variants、malformed/mixed/unknown rejection、
      runtime defaults/
      explicit-value validation/capture-adapter rejection、space/single-quote
      path/Auto-Windows-skip/explicit-Windows-rejection、observe-bundle absent/total-policy、
      generic-vs-specific MCP 同 generation 同 tool 双调用/各自 replay/dual-delivery、
      receipt tamper/old-path upgrade、
      false-positive ownership、foreign projection、idempotent install/uninstall focused
      fixtures 全绿。

- [ ] `SP824-T2` — secure staged writer 与两文件协调安装 — Owner: transaction implementation agent；Done when: 见下；Verify: 见下
  - Dependencies: `SP824-T1`；Covers: B-001, B-002, B-009, B-010, B-011, B-012,
    B-013, B-014, B-015, B-017, B-020, B-022
  - Writable ownership: secure staged writer、Cursor install/uninstall coordinator、target
    dispatch/runtime receipt integration 及其 failpoint tests；开始前由 T1 owner 冻结并
    移交 parser/plan interface，不与 T1 并行修改 shared files。
  - Done when: Cursor/Auto/All target 语义准确；两个 Cursor 文件、canonical runtime
    config/receipt 与所有 selected host 先统一 preflight；Cursor plan 必须在
    `ensure_runtime_store_ready()`、host config、token、data-dir 或任一 host write 前
    成功，首次运行的 Cursor preflight failure 保持 store/key/db/token/config 零副作用；
    再以 owner-only-before-first-byte temp staged apply；
    每次 replace/restore 紧邻前做 final comparison，保留此时已可观察到的外部编辑；
    replace 后 read-back，后续失败执行逆序 compensating rollback/read-back；明确
    compare→rename 不是 CAS，post-comparison/pre-rename 的非协作编辑可能被覆盖且
    不可检测，这是不可由 read-back/doctor/rollback 消除的 residual user-data-loss risk；
    rollback 失败或可观察 drift 显式返回 `partial_state` 与 doctor action，secret 不进入日志。
  - Verify:
    - `cargo test install -- --nocapture`
    - missing/permissive/ACL target、write/sync/rename/dir-sync/cleanup、每个 replace、
      rollback 与 rollback-verify failpoint matrix 全绿；
    - 每个 Cursor/runtime config target 的 concurrent edit 三阶段矩阵全绿：
      final comparison 前 mutate 必须 abort 并保留外部 bytes；
      comparison 后/rename 前 mutate 必须确定性展示 residual overwrite 窗口，且测试/
      输出不声称保留或检测；rename 后/read-back 前 mutate 必须以
      `partial_state`、精确路径和 doctor action 非零失败。rollback restore 使用
      同一矩阵；
    - Claude/Codex focused install regression 不降低既有断言。

- [ ] `SP824-T3` — dry-run、doctor 与可恢复诊断 — Owner: diagnostic implementation agent；Done when: 见下；Verify: 见下
  - Dependencies: `SP824-T2`；Covers: B-001, B-009, B-015, B-016, B-017, B-018,
    B-019, B-020, B-021, B-022
  - Writable ownership: Cursor plan renderer、CLI target/output glue、doctor Cursor state
    classifier 与 focused tests；只读取 T1/T2 frozen parser/plan/coordinator interface，
    shared file 如需变更必须在 T2 完成后显式移交。
  - Done when: dry-run 显示两个绝对路径及 redacted action，且不创建目录/temp/runtime
    store/key/db/token、不改 metadata；doctor 分开报告 detected/configured/mode/
    malformed/partial/drift/collision/effective 与 `session_init: unsupported`，human-readable
    输出包含精确 `session-init: not supported on cursor`，intentional hooks-only 不误报 partial；
    human/JSON 同时报告 `hook_failure_policy: host_continues`，不得把 remem-side
    zero-write 错报成 Cursor host 会阻断 prompt；
    `effective` 只由已批准、版本/capability 匹配的 GH-822 real-host evidence 判定，
    doctor 至少分别报告 `postToolUse_delivery: proven`、
    `postToolUse_managed_context: not_configured`、`sessionStart`、`stop` 与 `preCompact`；
    observe capture entry 不得被当成 postToolUse context producer；
    multi-host Cursor 失败不会被其他 host 成功覆盖。
  - Verify:
    - `cargo test doctor -- --nocapture`
    - `cargo test install -- --nocapture`
    - isolated HOME dry-run zero-side-effect、doctor state matrix（含所有 Cursor 状态下
      `session_init: unsupported` 与精确 human-readable line）、uninstall-before-downgrade
      golden output 和 Claude/Codex doctor regression 全绿。

- [ ] `SP824-T4` — 全量验证、文档与 implementation handoff — Owner: verification agent；Done when: 见下；Verify: 见下
  - Dependencies: `SP824-T1`, `SP824-T2`, `SP824-T3`；Covers: B-001, B-002,
    B-003, B-004, B-005, B-006, B-007, B-008, B-009, B-010, B-011, B-012,
    B-013, B-014, B-015, B-016, B-017, B-018, B-019, B-020, B-021, B-022
  - Writable ownership: user-facing Cursor install/doctor documentation、current behavior
    contract/status entry 和 implementation PR evidence；不修改 production code 或测试
    assertion。
  - Done when: PRODUCT 的负例矩阵、TECH 的 B-001..B-022 mapping 与真实实现逐项有
    fresh evidence；README/release-facing 文案只声称 user-level 配置、同一 snapshot 及
    final-comparison 可观察版本的 semantic foreign preservation、可观察 concurrent edit 保护边界、
    compare→rename residual user-data-loss risk、compensating rollback 和 GH-822 证明范围；
    不得再宣称所有 foreign data 保留或所有并发编辑 fail closed；
    implementation PR 使用正确 closing semantics，并等待 final human review/merge gate。
  - Verify:
    - `cargo fmt --check`
    - `cargo check`
    - `cargo test install`
    - `cargo test doctor`
    - `cargo test`
    - `python3 checks/check_workflow.py --repo . --spec-dir=specs/GH824`
    - `python3 scripts/ci/check_pr_preflight.py --base origin/main --pr-body-file /tmp/pr-body.md`

## 并行拆分

实现任务默认串行：T1 固化 parser/plan，T2 消费并冻结 transaction interface，T3 再接收
shared CLI/doctor ownership，T4 最后验证。T1/T2/T3 都可能触及 Cursor host 集成面，
未显式移交前不得并行写 shared files。只读评审可并行，但不得把评审通过当作 T0 证据。

## Draft Packet 验证

```bash
git diff --check
PYTHONPATH=checks python3 -c 'from pathlib import Path; from sensitive_enforcement import parse_planned_changes_manifest; m=parse_planned_changes_manifest(Path("specs/GH824/tech.md").read_bytes()); assert m["version"] == 1 and m["issue"] == 824 and m["complete"] is True'
python3 checks/check_workflow.py --repo .
python3 checks/check_workflow.py --repo . --spec-dir=specs/GH823
python3 checks/check_workflow.py --repo . --spec-dir=specs/GH824
python3 checks/github_issue_evidence.py --github-repo majiayu000/remem \
  --issue 823 --json > /tmp/gh823-issue-evidence.json
python3 checks/github_duplicate_evidence.py --github-repo majiayu000/remem \
  --issue 823 --json > /tmp/gh823-duplicate-evidence.json
python3 checks/route_gate.py --repo . --route write_spec --issue 823 \
  --evidence /tmp/gh823-issue-evidence.json \
  --duplicate-evidence /tmp/gh823-duplicate-evidence.json \
  --artifact product_spec=specs/GH823/product.md \
  --artifact tech_spec=specs/GH823/tech.md \
  --artifact task_plan=specs/GH823/tasks.md --json
python3 checks/github_issue_evidence.py --github-repo majiayu000/remem \
  --issue 824 --json > /tmp/gh824-issue-evidence.json
python3 checks/github_duplicate_evidence.py --github-repo majiayu000/remem \
  --issue 824 --json > /tmp/gh824-duplicate-evidence.json
python3 checks/route_gate.py --repo . --route write_spec --issue 824 \
  --evidence /tmp/gh824-issue-evidence.json \
  --duplicate-evidence /tmp/gh824-duplicate-evidence.json \
  --artifact product_spec=specs/GH824/product.md \
  --artifact tech_spec=specs/GH824/tech.md \
  --artifact task_plan=specs/GH824/tasks.md --json
```

以上命令只证明 planning packet 结构有效。它们不满足 `SP824-T0`，也不授权 runtime。

## Handoff Notes

- 当前优先级是人工审查并采用 PR #914 real-host evidence，然后完成并合入消费该 evidence
  的 GH-823 amendment/canonical host-hook runtime contract；GH-824 人工批准必须发生在这些
  事实可审查之后。
- 如果 GH-822 与官方文档或 GH-823 Draft 冲突，先更新并重新批准 product/tech/tasks，
  不允许实现 agent 猜测 event、payload、timeout 或 `effective`；remem managed v1
  entry 的 type/prompt/matcher/failClosed/loop_limit/独立 args 字段明确禁止，
  但 B-003 仍完整验证并保留合法 foreign entry。
- 所有生产实现、fixture 编码和 runtime 写入都从 `SP824-T0` 之后开始；本 spec-only PR
  只交付规划文档。
