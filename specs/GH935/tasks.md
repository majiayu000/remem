# Task Plan

## Linked Issue

GH-935

## Spec Packet

- Product: `specs/GH935/product.md`
- Tech: `specs/GH935/tech.md`
- Locale: zh-CN
- Status: Draft；所有任务均未开始。

## Human Gates First

- [ ] `SP935-T1` Owner: maintainer; Done when: exact packet 获得 spec approval、GH-935 转为 `ready_to_implement`，且实现 coordinator 对当时 exact head 完成 fresh route evidence； Verify: exact-head implement route gate 返回 `allowed`； Covers: none（人工治理门禁）。
  - Owner: maintainer
  - Dependencies: none
  - Covers: none — 这是实施前的人工治理门禁，不实现产品行为。
  - Done when:
    - maintainer 审阅 canonical product/tech exact diff；
    - 已满足的 `ready_to_spec` 只证明可写 spec；maintainer 另行记录
      `spec_approval` 并把 issue 置为 `ready_to_implement`；
    - implementation coordinator 针对准备实现的 exact head 和当时 default
      base 收集 route gate 所要求的 fresh evidence；不复用本 spec lane 的
      历史输出；
    - `implement` route gate 对 exact head 返回 `allowed`；
    - security reviewer 明确接受宿主 auth、host-read sandbox、hidden tests
      和 public-claim 边界。
  - Verify:

    ```bash
    python3 checks/route_gate.py --repo . --route implement \
      --issue 935 --state ready_to_implement \
      <fresh-exact-head-evidence-arguments-required-by-current-workflow> --json
    ```

- [ ] `SP935-T1A` Owner: governance-security lane; Done when: machine-sensitive registry prerequisite 经独立 security review 合入 default branch，future approved-tech classification 为 `enforcement_sensitive=true`； Verify: workflow check 与 planned-tech classifier 通过； Covers: B-009, B-031, B-032。
  - Owner: governance-security lane
  - Dependencies: SP935-T1
  - Covers: B-009, B-031, B-032
  - File ownership:
    - `workflow.yaml`
  - Done when:
    - `enforcement.sensitive_registry` 明确覆盖 `specs/GH935/*`、live approval
      schema/registry、`src/eval/cross_host/{approval.rs,claim_gate.rs}`、
      claim-verdict schema/result 与 GH-935 public-claim authority；
    - 该治理变更以独立、`enforcement_sensitive=true` 的 PR 接受 security
      review 并 merge；后续 implementation PR 不能用 author declaration 替代；
    - 本 complete planned manifest 的 approved-tech classifier 返回 true，
      当前只改 `specs/GH935/*` 的 spec PR 仍返回 false。
  - Verify:

    ```bash
    python3 checks/check_workflow.py --repo .
    python3 scripts/ci/check_pr_tier.py --self-test
    ```

## 实现任务

- [ ] `SP935-T2` Owner: fixture-contract lane; Done when: versioned schemas 与 24 个 deterministic tasks 全部 `ready`； Verify: schema self-test 与 `run_dry.py` 通过； Covers: B-001, B-002, B-003, B-004, B-005, B-030。
  - Owner: fixture-contract lane
  - Dependencies: SP935-T1A
  - Covers: B-001, B-002, B-003, B-004, B-005, B-030
  - File ownership:
    - `eval/cross-host/benchmark-charter.json`
    - `eval/cross-host/live-run-approvals.json`
    - `eval/cross-host/schemas/`
    - `eval/cross-host/examples/`
    - `eval/cross-host/scripts/{schema_validate.py,run_dry.py}`
    - `eval/cross-host/tasks/`
  - Done when:
    - task/run/report/live-run-approval/source-seal schemas versioned 且 fail closed；
    - `eval/cross-host/live-run-approvals.json` 是 append-only、默认分支可信
      registry；entry 的 stable `approval_key` 由不含自身、review node 或 merge
      commit 的 canonical policy digest、approval PR number 与 approved head
      tree OID 派生，不能由执行者自选；
    - 24 个 task 逐个包含 deterministic repo fixture、至少两个 chronological
      source episodes、source-seal contract、
      hidden tests、score commands、gold facts、allowed/forbidden paths；
    - 每个 task 为 `ready` 且 `todo: []`；
    - 两个方向各覆盖 12 个必需 category，无 fabricated placeholder；
    - v1 skeleton/old artifacts 被明确拒绝或由测试覆盖的 converter 转换。
  - Verify:

    ```bash
    python3 eval/cross-host/scripts/schema_validate.py --self-test
    python3 eval/cross-host/scripts/run_dry.py
    jq -e '.status == "executable_no_runs"' \
      eval/cross-host/benchmark-charter.json
    test "$(jq -r '.status' eval/cross-host/tasks/*/*.json | \
      awk '$0 != "ready" {bad++} END {print bad+0}')" = 0
    test "$(find eval/cross-host/tasks -type f -name '*.json' | wc -l | tr -d ' ')" = 24
    ```

- [ ] `SP935-T3` Owner: host-isolation lane; Done when: Claude/Codex 共用 fail-closed host isolation 且 dry-run 零 spawn； Verify: host-isolation/coding-bench/cross-host isolation tests 通过； Covers: B-009, B-010, B-011, B-015, B-016, B-017, B-032。
  - Owner: host-isolation lane
  - Dependencies: SP935-T1A
  - Covers: B-009, B-010, B-011, B-015, B-016, B-017, B-032
  - File ownership:
    - `src/eval/host_isolation.rs`
    - `src/eval/coding_bench/{isolation.rs,runner.rs,tests.rs}`
    - `src/eval/cross_host/isolation.rs`
  - Done when:
    - coding bench 与 cross-host 共用一个 env allowlist、private-root、
      host-read sandbox、timeout/process cleanup primitive；
    - Claude Code/Codex adapters 使用独立 HOME/config/session roots，GitHub
      authority phase 与 host/provider phase 分离，后者不可读取 authority
      credential；
    - source/target 的 phase-private condition roots 保持不同；只有
      `remem_shared` 可串行挂载当前 run 独有且位于两侧 private roots 之外的
      transfer store，任何其他 shared/cross-run path 都 fail closed；
    - source/target 严格串行复用同一 run-scoped canonical absolute workspace
      path，target 前从 approved fixture 重置并保持 project ID；same-name decoy
      使用不同 canonical path/project ID；
    - credential bootstrap 最小化且 credential bytes 不进入 artifacts；
    - 非 macOS 在没有等价 deny-host-read 证明时 fail closed；
    - dry-run/verify call graph 不 spawn 宿主或网络。
  - Verify:

    ```bash
    cargo test eval::host_isolation
    cargo test eval::coding_bench
    cargo test eval::cross_host::tests::isolation
    ```

- [ ] `SP935-T4` Owner: condition lane; Done when: primary conditions 与 native ablation 边界闭集且 remem 走真实 pipeline； Verify: condition/pipeline/export/native focused tests 通过； Covers: B-008, B-012, B-013, B-014, B-015, B-016, B-029。
  - Owner: condition lane
  - Dependencies: SP935-T2, SP935-T3
  - Covers: B-008, B-012, B-013, B-014, B-015, B-016, B-029
  - File ownership:
    - `src/eval/cross_host/condition.rs`
    - `src/eval/cross_host/fixture.rs`
  - Done when:
    - condition surface 是闭集并由 manifest/hash 审计；
    - `remem_shared` 走 automatic capture→extraction→promotion→normal
      retrieval，测试证明未调用 direct seed/save/preload shortcut；
    - surface manifest 证明 `remem_shared` 唯一共同路径是 run-scoped transfer
      store，source session/phase-private/cross-run path 的负例全部拒绝；
    - `target_host_native` 在 prompt 揭示前通过目标宿主真实 preparation/import
      protocol 产生可读 native state 并记录成本；空/不可读 native state
      fail closed，不能退化成 `no_memory`；
    - `exported_file` 在第一 episode 后生成、后续每 episode 更新，经所有
      condition 共用的 host-neutral context-envelope 消费，target-blind 冻结并
      分别记录 generation/maintenance cost；
    - native import with arm 仅将 importer 产出的 candidate 经 target-blind
      独立 review/promotion 后激活，保留 `host_native_import` origin 与
      non-canonical trust；without control 走相同 reviewer schedule；
    - diagnostic data 不进入 primary denominator。
  - Verify:

    ```bash
    cargo test eval::cross_host::tests::conditions
    cargo test eval::cross_host::tests::real_capture_pipeline
    cargo test eval::cross_host::tests::exported_file_cost
    cargo test eval::cross_host::tests::native_import_ablation
    ```

- [ ] `SP935-T5` Owner: runner lane; Done when: 288-tuple plan、immutable artifacts、resume、hidden scoring 与 attribution 完整； Verify: runner focused tests、scanner self-test 与 288 dry-run 通过； Covers: B-006, B-007, B-008, B-017, B-018, B-019, B-020, B-021。
  - Owner: runner lane
  - Dependencies: SP935-T2, SP935-T3, SP935-T4
  - Covers: B-006, B-007, B-008, B-017, B-018, B-019, B-020, B-021
  - File ownership:
    - `src/eval/cross_host/{approval.rs,runner.rs,score.rs,types.rs,tests.rs}`
    - `src/eval/cross_host.rs`
    - `src/eval.rs`
    - `.gitignore`
    - `eval/cross-host/scripts/scan_artifacts.py`
    - `eval/cross-host/evidence/cross-host-v1/{primary-run-records.jsonl,native-ablation-run-records.jsonl,source-manifest.json}`
  - Done when:
    - primary dry-run 精确产生 288 个唯一 tuple，native ablation dry-run
      精确产生 144 个唯一 tuple；
    - 72 个 `(direction,task,run_index)` 各只执行一次 source episode sequence；
      source terminate/drain/immutable seal 先于 fanout 和 target launch，全部
      conditions 绑定同一 seal；
    - artifact 使用 atomic write、stable `matrix_key`、unique `attempt_id`
      和 content hash；
    - pre-target retry 保留失败，target-started outcome 不可被成功重跑替换；
    - resume 只补缺失 tuple，并拒绝 duplicate/hash drift/partial artifacts；
    - hidden tests 只在 agent 退出后注入；
    - attribution 每阶段必须是可解析 `present(ref)` 或 typed
      `absent_due_to(failure)`；合法上游 failure 的下游 absence 保留 failed run
      于 denominator，无 failure 的缺 ref 被拒绝，origin/scope/validity 一致；
    - scanner 覆盖 HOME/session/auth/private/hidden/cross-run 泄漏。
    - evidence writer 输出 scanner-passed primary/native-ablation JSONL bundles
      与 schema-valid source manifest，绑定全部 attempt/matrix/scanner/code/
      fixture/config/approval hashes及 candidate-report input hash；raw/private/
      secret paths 不进入 git。
    - runner 在任何 live spawn 前通过 GitHub API 验证 default-branch approval
      registry、merged approval PR、APPROVED maintainer review、canonical digest、
      exact code/fixture/config/model/host hashes、allowed tuple set 与
      host/LLM/cost hard caps；smoke plan 恰好一个 tuple 且永久排除公开分母。
    - 每条命令生成独立 `execution_id`，但 stable `approval_key` 对应
      protected remote ledger 在每个 billable call 前以 non-force fast-forward
      CAS durable reserve worst-case host/LLM/cost，完成后 settlement；crash/
      abandon 仍计费，跨 clone/resume/execution ID/并发共享累计预算，拒绝
      rollback、replay、non-FF history、reconciliation drift 或拆单重置。
  - Verify:

    ```bash
    cargo test eval::cross_host::tests::run_plan
    cargo test eval::cross_host::tests::attempt_history
    cargo test eval::cross_host::tests::resume
    cargo test eval::cross_host::tests::attribution
    python3 eval/cross-host/scripts/scan_artifacts.py --self-test
    cargo run -- bench cross-host run --root eval/cross-host \
      --runs-per-condition 3 --matrix primary --dry-run \
      --json-out /tmp/remem-cross-host-plan.json
    jq -e '.planned_runs == 288' /tmp/remem-cross-host-plan.json
    cargo run -- bench cross-host run --root eval/cross-host \
      --runs-per-condition 3 --matrix native-import-ablation --dry-run \
      --json-out /tmp/remem-cross-host-ablation-plan.json
    jq -e '.planned_runs == 144' /tmp/remem-cross-host-ablation-plan.json
    ```

- [ ] `SP935-T6` Owner: report-claim lane; Done when: direction report、paired bootstrap、cost、stop-loss 与 hash-bound claim gate 完整； Verify: report/bootstrap/claim tests 与 public claim self-test 通过； Covers: B-022, B-023, B-024, B-025, B-026, B-027, B-028, B-029, B-031。
  - Owner: report-claim lane
  - Dependencies: SP935-T5
  - Covers: B-022, B-023, B-024, B-025, B-026, B-027, B-028, B-029, B-031
  - File ownership:
    - `src/eval/cross_host/{report.rs,bootstrap.rs,claim_gate.rs}`
    - `eval/cross-host/schemas/cross-host-report.schema.json`
    - `eval/cross-host/schemas/cross-host-claim-verdict.schema.json`
    - `eval/cross-host/schemas/evidence-source-manifest.schema.json`
    - `eval/cross-host/claims-registry.json`
    - `eval/cross-host/reports/{cross-host-v1.json,cross-host-v1.md,cross-host-v1-gate.json}`
    - `scripts/ci/check_public_claims.py`
  - Done when:
    - report builder 先从 verified source manifest/bundles 生成 immutable
      direction-first candidate report，公开分子、分母、missing_count 和
      aggregate，且 candidate 不含自引用 verdict；
    - exported cost 与 native-import ablation 分开报告；
    - native-import ablation completeness 固定为 144；
    - fixed-seed task-cluster paired bootstrap 可重复；
    - CI 含 0 只输出 directional/insufficient；
    - leak stop-loss 分母固定为 288；`memory_hurt`/`stale_memory_followed`
      分母固定为每方向 36、aggregate 72，并按 B-027 的 attribution-causal
      predicates 计算；缺 required attribution 为 INSUFFICIENT；
    - 五项 stop-loss 边界和“gain + leak = FAIL”负例通过；
    - claim gate 消费 candidate report hash，另写并保留 PASS/FAIL/INSUFFICIENT
      gate result；`bench cross-host gate` 是唯一 result-writer，registry/
      report/gate hash/wording 四者绑定；
    - FAIL/INSUFFICIENT 不删除或改写 candidate report/evidence；
    - public claim checker 在非 PASS 时拒绝正向 README wording。
  - Verify:

    ```bash
    cargo test eval::cross_host::tests::report
    cargo test eval::cross_host::tests::evidence_bundle
    cargo test eval::cross_host::tests::paired_bootstrap
    cargo test eval::cross_host::tests::claim_gate
    cargo run -- bench cross-host gate --root eval/cross-host \
      --registry eval/cross-host/claims-registry.json \
      --report eval/cross-host/reports/cross-host-v1.json \
      --json-out /tmp/remem-cross-host-gate.json
    python3 eval/claims/claim_gate.py check \
      eval/cross-host/claims-registry.json
    python3 scripts/ci/check_public_claims.py --self-test
    ```

- [ ] `SP935-T7` Owner: integration-doc lane; Done when: CLI/docs/version 接线且仍准确声明无 run/无公开结论； Verify: CLI dry-run/verify、version sync 与 public claim check 通过； Covers: B-001, B-030, B-031。
  - Owner: integration-doc lane
  - Dependencies: SP935-T2, SP935-T3, SP935-T4, SP935-T5, SP935-T6
  - Covers: B-001, B-030, B-031
  - File ownership:
    - `src/cli/eval_types.rs`
    - `src/cli/actions/eval.rs`
    - `eval/cross-host/README.md`
    - `docs/ARCHITECTURE.md`
    - `docs/specs/README.md`
    - `docs/specs/GH935/{PRODUCT.md,TECH.md}`
    - `docs/specs/public-memory-benchmark/{PRODUCT.md,TECH.md}`
    - `README.md`
    - `README.zh-CN.md`
    - `CHANGELOG.md`
    - version-sync files declared in `tech.md`
  - Done when:
    - `bench cross-host run|verify|report|gate` 接线且所有 output path 显式；
    - docs 保持 `executable_no_runs`/“无公开结论”，不引用 dry-run 数字为结果；
    - version-sync 文件使用同一新版本；
    - 当前 integration/version PR 的 touched paths 全部属于 planned-path
      manifest，且截至 T7 应完成的 implementation paths 已由 linked PR file
      lists 覆盖；只允许把 report/live-evidence paths 明确 defer 到 T10/T11，
      不在 T7 要求单个 PR diff 等于累计 manifest。
  - Verify:

    ```bash
    cargo run -- bench cross-host run --root eval/cross-host \
      --runs-per-condition 3 --matrix primary --dry-run \
      --json-out /tmp/remem-cross-host-plan.json
    cargo run -- bench cross-host verify --root eval/cross-host \
      --json-out /tmp/remem-cross-host-verify.json
    python3 scripts/ci/check_plugin_version_sync.py
    python3 scripts/ci/check_public_claims.py
    ```

## 验证与执行

- [ ] `SP935-T8` Owner: independent reviewer lane; Done when: 全部 invariants、security boundaries 与 repository gates 有 fresh 证据； Verify: schema/scanner/dry-run、fmt/check/clippy/test 全通过； Covers: B-001…B-032。
  - Owner: independent reviewer lane
  - Dependencies: SP935-T7
  - Covers: B-001, B-002, B-003, B-004, B-005, B-006, B-007, B-008,
    B-009, B-010, B-011, B-012, B-013, B-014, B-015, B-016, B-017,
    B-018, B-019, B-020, B-021, B-022, B-023, B-024, B-025, B-026,
    B-027, B-028, B-029, B-030, B-031, B-032
  - Done when:
    - independent reviewer 检查 auth/sandbox/process cleanup、OS command
      array arguments、secret redaction、hidden fixture 与 claim gate；
    - focused tests、full Rust gates 和 preflight 使用本 session fresh 输出；
    - 没有实现任务或测试被弱化来通过 gate。
  - Verify:

    ```bash
    python3 eval/cross-host/scripts/schema_validate.py --self-test
    python3 eval/cross-host/scripts/scan_artifacts.py --self-test
    python3 eval/cross-host/scripts/run_dry.py
    cargo fmt --check
    cargo check
    cargo clippy --all-targets -- -D warnings
    cargo test
    python3 scripts/ci/check_plugin_version_sync.py
    python3 scripts/ci/check_public_claims.py
    gh pr view "$PR_NUMBER" --json body --jq .body > /tmp/pr-body.md
    python3 scripts/ci/check_pr_preflight.py --base origin/main \
      --pr-body-file /tmp/pr-body.md
    ```

- [ ] `SP935-T9` Owner: maintainer/security owner; Done when: smoke 与 full live-run 的 auth/network/cost/security 授权分别记录； Verify: trusted approval registry 与真实 smoke execution/verification； Covers: B-009, B-032。
  - Owner: maintainer/security owner
  - Dependencies: SP935-T8
  - Covers: B-009, B-032
  - Done when:
    - maintainer 明确批准 live Claude/Codex、auth bootstrap、network/LLM cost、
      运行预算、模型/version locks 和 artifact 存放位置；
    - 先批准每方向一个 smoke tuple；smoke 通过隔离/attribution/cleanup 人工
      审查后，再单独批准完整 primary/native-ablation 执行；
    - smoke artifacts 明确不进入 public denominator。
    - smoke approval entry 通过
      `eval/cross-host/schemas/live-run-approval.schema.json`，经独立 maintainer
      APPROVED review 的 PR merge 到 default branch，绑定 exact
      head/fixture/config/model/host versions、两个允许 tuple、有效期、credential
      bootstrap reference 与 host/LLM/cost hard caps；entry 不含 credential
      bytes；pre-merge `approval_key` 只由 approval PR number、approved head
      tree OID 与不含自身/review/merge 字段的 canonical policy digest 派生。
      runner 必须先完成 authority-only remote validation，再清除 GitHub
      credential；每个 billable call 前 remote ledger reservation 已 durable，
      crash/跨 clone/并发预算测试均通过。
  - Verify:

    ```bash
    test -n "$CROSS_HOST_SMOKE_APPROVAL_KEY"
    test -n "$CROSS_HOST_MAX_HOST_CALLS"
    test -n "$CROSS_HOST_MAX_LLM_CALLS"
    test -n "$CROSS_HOST_MAX_ESTIMATED_COST_USD"
    cargo run --release -- bench cross-host run --root eval/cross-host \
      --matrix smoke --direction claude_to_codex \
      --task-id cc2cx-architecture-decision --condition remem_shared \
      --run-index 0 --approval-key "$CROSS_HOST_SMOKE_APPROVAL_KEY" \
      --confirm-live-run --max-host-calls "$CROSS_HOST_MAX_HOST_CALLS" \
      --max-llm-calls "$CROSS_HOST_MAX_LLM_CALLS" \
      --max-estimated-cost-usd "$CROSS_HOST_MAX_ESTIMATED_COST_USD" \
      --json-out /tmp/remem-cross-host-smoke-cc2cx.json
    cargo run --release -- bench cross-host run --root eval/cross-host \
      --matrix smoke --direction codex_to_claude \
      --task-id cx2cc-architecture-decision --condition remem_shared \
      --run-index 0 --approval-key "$CROSS_HOST_SMOKE_APPROVAL_KEY" \
      --confirm-live-run --max-host-calls "$CROSS_HOST_MAX_HOST_CALLS" \
      --max-llm-calls "$CROSS_HOST_MAX_LLM_CALLS" \
      --max-estimated-cost-usd "$CROSS_HOST_MAX_ESTIMATED_COST_USD" \
      --json-out /tmp/remem-cross-host-smoke-cx2cc.json
    cargo run --release -- bench cross-host verify --root eval/cross-host \
      --input /tmp/remem-cross-host-smoke-cc2cx.json \
      --approval-key "$CROSS_HOST_SMOKE_APPROVAL_KEY" \
      --expected-matrix smoke --expected-valid-runs 1 \
      --json-out /tmp/remem-cross-host-smoke-cc2cx-verify.json
    cargo run --release -- bench cross-host verify --root eval/cross-host \
      --input /tmp/remem-cross-host-smoke-cx2cc.json \
      --approval-key "$CROSS_HOST_SMOKE_APPROVAL_KEY" \
      --expected-matrix smoke --expected-valid-runs 1 \
      --json-out /tmp/remem-cross-host-smoke-cx2cc-verify.json
    ```

- [ ] `SP935-T10` Owner: authorized benchmark operator; Done when: 288 个 primary tuples 均有 immutable scanner-passed artifact； Verify: final verifier 报告 `valid_runs == 288`； Covers: B-006, B-008…B-024, B-027, B-028。
  - Owner: authorized benchmark operator
  - Dependencies: SP935-T9
  - Covers: B-006, B-008, B-009, B-010, B-011, B-012, B-013, B-014,
    B-015, B-016, B-017, B-018, B-019, B-020, B-021, B-022, B-023,
    B-024, B-027, B-028
  - Done when:
    - report-claim lane 先 handoff immutable 288-tuple plan/source-seal/approved
      binary+fixture+profile hashes；operator 只拥有 live execution outputs，
      不修改 runner/report code 或 approved inputs；
    - 两个方向、24 tasks、四 primary conditions、每 tuple 3 runs 均产生
      scanner-passed immutable artifact；
    - verifier 证明 288 个唯一有效 tuple，失败 run 留在分母；
    - 无真实 HOME/session/auth/hidden/private leak；
    - execution checkpoint 支持 resume 且记录所有失败 attempt。
    - scanner-passed primary records 写入 committed bundle，source manifest
      绑定 288 matrix keys、全部 attempts 与 report input hash；raw/private/
      credential evidence 不进入 git。
  - Verify:

    ```bash
    test -n "$CROSS_HOST_PRIMARY_APPROVAL_KEY"
    test -n "$CROSS_HOST_MAX_HOST_CALLS"
    test -n "$CROSS_HOST_MAX_LLM_CALLS"
    test -n "$CROSS_HOST_MAX_ESTIMATED_COST_USD"
    cargo run --release -- bench cross-host run --root eval/cross-host \
      --runs-per-condition 3 --matrix primary \
      --approval-key "$CROSS_HOST_PRIMARY_APPROVAL_KEY" --confirm-live-run \
      --max-host-calls "$CROSS_HOST_MAX_HOST_CALLS" \
      --max-llm-calls "$CROSS_HOST_MAX_LLM_CALLS" \
      --max-estimated-cost-usd "$CROSS_HOST_MAX_ESTIMATED_COST_USD" \
      --json-out /tmp/remem-cross-host-primary.json
    cargo run --release -- bench cross-host verify --root eval/cross-host \
      --json-out /tmp/remem-cross-host-primary-verify.json
    jq -e '.passed == true and .matrix.primary.valid_runs == 288' \
      /tmp/remem-cross-host-primary-verify.json
    cargo run --release -- bench cross-host verify --root eval/cross-host \
      --evidence-manifest eval/cross-host/evidence/cross-host-v1/source-manifest.json \
      --json-out /tmp/remem-cross-host-evidence-verify.json
    ```

- [ ] `SP935-T11` Owner: authorized benchmark operator + report-claim lane; Done when: native ablation、direction reports、bootstrap、stop-loss 与 claim verdict 可复算； Verify: report verifier、claim gate 与 public claim check 通过； Covers: B-007, B-022…B-031。
  - Owner: authorized benchmark operator + report-claim lane
  - Dependencies: SP935-T10
  - Covers: B-007, B-022, B-023, B-024, B-025, B-026, B-027, B-028,
    B-029, B-031
  - Done when:
    - report-claim lane handoff immutable 144-tuple ablation plan、approved inputs
      与 report/gate schemas；operator 仅生成 live ablation outputs，随后
      report-claim lane 独占 candidate report、gate 和 current-doc updates；
    - with/without native import 在两个方向形成完整 144-tuple paired evidence；
    - native-ablation sanitized records 追加到独立 committed bundle，完成
      source manifest 后，先生成 immutable direction-specific candidate
      JSON/Markdown report 与 hash；
    - exported cost、native contribution、bootstrap CI、全部 stop-loss 和
      claim verdict 可复算；
    - claim gate 消费 candidate report 后另写
      `cross-host-v1-gate.json`；PASS/FAIL/INSUFFICIENT 都保留 candidate
      report、gate result 与 evidence；
    - 只有 gate PASS 时才由独立 human wording review 更新 README；否则保持
      无公开正向结论，但不得删除 FAIL/INSUFFICIENT evidence。
    - 无论 verdict 为 PASS/FAIL/INSUFFICIENT，都更新 `eval/cross-host/README.md`、
      `docs/specs/README.md`、`docs/specs/GH935/{PRODUCT.md,TECH.md}` 和
      `docs/specs/public-memory-benchmark/{PRODUCT.md,TECH.md}` 的真实运行状态与
      report/gate links；不得继续声明 `executable_no_runs`。
  - Verify:

    ```bash
    test -n "$CROSS_HOST_ABLATION_APPROVAL_KEY"
    test -n "$CROSS_HOST_MAX_HOST_CALLS"
    test -n "$CROSS_HOST_MAX_LLM_CALLS"
    test -n "$CROSS_HOST_MAX_ESTIMATED_COST_USD"
    cargo run --release -- bench cross-host run --root eval/cross-host \
      --runs-per-condition 3 --matrix native-import-ablation \
      --approval-key "$CROSS_HOST_ABLATION_APPROVAL_KEY" --confirm-live-run \
      --max-host-calls "$CROSS_HOST_MAX_HOST_CALLS" \
      --max-llm-calls "$CROSS_HOST_MAX_LLM_CALLS" \
      --max-estimated-cost-usd "$CROSS_HOST_MAX_ESTIMATED_COST_USD" \
      --json-out /tmp/remem-cross-host-native-ablation.json
    cargo run --release -- bench cross-host report --root eval/cross-host \
      --json-out eval/cross-host/reports/cross-host-v1.json \
      --markdown-out eval/cross-host/reports/cross-host-v1.md
    cargo run --release -- bench cross-host verify --root eval/cross-host \
      --json-out /tmp/remem-cross-host-final-verify.json
    cargo run --release -- bench cross-host gate --root eval/cross-host \
      --registry eval/cross-host/claims-registry.json \
      --report eval/cross-host/reports/cross-host-v1.json \
      --json-out eval/cross-host/reports/cross-host-v1-gate.json
    python3 scripts/ci/check_public_claims.py
    ```

- [ ] `SP935-T12` Owner: maintainer; Done when: final review、CI、threads、PR gate 与 issue/merge/release 决策完成； Verify: exact-head GitHub evidence 与 `pr_gate.py`； Covers: B-031, B-032。
  - Owner: maintainer
  - Dependencies: SP935-T11
  - Covers: B-031, B-032
  - Done when:
    - exact-head independent review、CI、review threads 和 `pr_gate` 全部满足；
    - maintainer 单独决定 README wording、merge、是否关闭 #935 与 release；
    - 若 matrix、ablation、stop-loss 或 claim 任一不完整，#935 保持 open，
      不以 infrastructure/partial PR 关闭。
    - closure audit 从固定 baseline 收集 #937、spec PR、implementation PR 和
      report/evidence PR 的 touched-file union；该 union 与 complete planned-path
      manifest 精确相等，任一遗漏/额外路径都阻止关闭并要求先更新 spec。
  - Verify:

    ```bash
    test -n "$PR_NUMBER"
    gh pr view "$PR_NUMBER" --json headRefOid,mergeable,mergeStateStatus,reviewDecision,statusCheckRollup
    gh pr view "$PR_NUMBER" --json body --jq .body > /tmp/pr-body.md
    python3 checks/github_pr_evidence.py --repo . --github-repo \
      "$(gh repo view --json nameWithOwner --jq .nameWithOwner)" \
      --pr "$PR_NUMBER" --issue 935 --review-manifest /tmp/pr-body.md \
      --json > /tmp/pr-evidence.json
    python3 checks/pr_gate.py --repo . --evidence /tmp/pr-evidence.json \
      --mode required --json
    ```

## 并行拆分

仅在 SP935-T1A 解除后允许：

- SP935-T1A 必须在 T1 后以独立 sensitive-registry PR 落地；它 merge 前不得
  启动 T2-T12。
- `fixture-contract lane`（SP935-T2）与 `host-isolation lane`（SP935-T3）
  可并行；前者只写 `eval/cross-host` contract/tasks，后者只写列明的 Rust
  isolation 文件。
- SP935-T4 必须等待 T2/T3，并独占 `condition.rs`/`fixture.rs`。
- SP935-T5 在 T4 后串行，独占 runner/artifact/scanner 文件。
- SP935-T6 在 T5 后串行，独占 report/bootstrap/claim 文件。
- SP935-T7 在所有实现 lane 完成后串行处理共享 CLI/docs/version 文件。
- live execution（T10/T11）不得与任何可改变 code/fixture/config hash 的 lane
  并行。

任何 agent 不得写其他 lane 的文件；共享文件需要由依赖图中的后置单一 owner
处理，禁止两个 agent 同时修改。

## Handoff Notes

- `origin/main@5627a74942a41f51bdc03518fce726dbf1b46098` 上的当前事实：
  charter 为 `infrastructure_only_no_runs`，24/24 task 为
  `skeleton_todo`，report artifact 为 0。
- GH-935 已获 `ready_to_spec`，fresh write-spec route gate 为 `allowed`；
  尚缺 exact packet 的 `spec_approval` 和后续 `ready_to_implement`。
- 本 lane 未把一个不带 implementation evidence 的预检快照包装成“当前
  implement gate 结论”。SP935-T1 必须在未来实现 head/base 确定后重新收集
  route 所需证据。
- 本 packet 只规划未来实现；没有运行 benchmark、没有生成 report、没有修改
  source/docs/runtime，也没有授予 host auth、network/LLM cost、live-run、
  public-claim、merge 或 release 权限。
- Product invariant set：
  `B-001`…`B-032`。
- Task coverage union：
  `B-001`…`B-032`；无遗漏。
- PR #937/merge `60c228ceaf02d881da1de2bacf6d13433e18dccb`
  是基础设施历史证据，不是 outcome evidence。
