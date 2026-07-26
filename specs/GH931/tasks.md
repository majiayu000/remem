# Task Plan

## Linked Issue

GH-931

## Spec Packet

- Product: `specs/GH931/product.md`
- Tech: `specs/GH931/tech.md`
- Locale: zh-CN
- Status: Draft；本 spec lane 未开始 implementation 或 live run。

## Human Gates First

- [ ] `SP931-T1` Owner: maintainer/security owner; Done when: exact packet 获得 spec approval 且 GH-931 转为 `ready_to_implement`； Verify: approval/state evidence 可供 post-registry gate 使用； Covers: B-030。
  - Owner: maintainer/security owner
  - Dependencies: none
  - Covers: B-030
  - Done when:
    - maintainer 审阅 product/tech exact diff 并记录 `spec_approval`；
    - issue 从已满足的 `ready_to_spec` 转为 `ready_to_implement`；
    - security owner 批准 `scripts/ci/check_public_claims.py` 与 public-claim
      boundary 的 planned-path classification；
    - 不把尚未落地的 T1A sensitive registry 当成 implement-gate baseline；
      implement route 必须由 T1B 在 T1A merge 后重新收集。
  - Verify:

    ```bash
    test -n "$SPEC_APPROVAL_EVIDENCE"
    ```

- [ ] `SP931-T1A` Owner: governance-security lane; Done when: machine-sensitive registry prerequisite 经独立 security review 合入 default branch，future approved-tech classification 为 `enforcement_sensitive=true`； Verify: workflow check 与 planned-tech classifier 通过； Covers: B-020, B-030。
  - Owner: governance-security lane
  - Dependencies: SP931-T1
  - Covers: B-020, B-030
  - File ownership:
    - `workflow.yaml`
  - Done when:
    - `enforcement.sensitive_registry` 明确覆盖 `specs/GH931/*`、
      `eval/coding-bench/schemas/live-run-approval.schema.json`、
      `eval/coding-bench/live-run-approvals.json`、
      `src/eval/coding_bench/approval.rs` 与 GH-931 public-claim authority；
    - 该治理变更以独立、`enforcement_sensitive=true` 的 PR 接受 security
      review 并 merge，不能由后续 implementation PR 自报 sensitive；
    - 对本 complete planned manifest 的 approved-tech classifier 返回 true，
      对当前只改 `specs/GH931/*` 的 spec PR 仍返回 false。
  - Verify:

    ```bash
    python3 checks/check_workflow.py --repo .
    python3 scripts/ci/check_pr_tier.py --self-test
    ```

- [ ] `SP931-T1B` Owner: implementation coordinator; Done when: T1A merge 后针对 post-registry default base 与 exact implementation head 收集 fresh route evidence，implement gate 返回 `allowed`； Verify: fresh post-governance implement route gate； Covers: B-030。
  - Owner: implementation coordinator
  - Dependencies: SP931-T1A
  - Covers: B-030
  - Done when:
    - fresh evidence 的 trusted base 含已 merge 的 T1A registry/rules；
    - evidence 绑定准备启动 T2/T3 的 exact implementation head；
    - 不复用 T1 或 spec PR 的 pre-registry route output；
    - `implement` route gate 返回 `allowed` 后才可启动任何 implementation lane。
  - Verify:

    ```bash
    python3 checks/route_gate.py --repo . --route implement \
      --issue 931 --state ready_to_implement \
      <fresh-post-T1A-exact-head-evidence-arguments> --json
    ```

## 实现任务

- [ ] `SP931-T2` Owner: condition-contract lane; Done when: condition registry、Rust IDs、schemas 与 legacy artifact policy 收敛且无 alias； Verify: validator、ID parse tests 与 primary 144-plan tests 通过； Covers: B-001-B-006, B-013。
  - Owner: condition-contract lane
  - Dependencies: SP931-T1B
  - Covers: B-001, B-002, B-003, B-004, B-005, B-006, B-013
  - File ownership:
    - `eval/coding-bench/{benchmark-charter.json,conditions.json,validate_schemas.py}`
    - `eval/coding-bench/schemas/{conditions.schema.json,flagship-run.schema.json,flagship-report.schema.json}`
    - `eval/coding-bench/schemas/live-run-approval.schema.json`
    - `eval/coding-bench/live-run-approvals.json`
    - `eval/coding-bench/fixtures/tasks.json`
    - `src/eval/coding_bench/{types.rs,run_plan.rs,fixture.rs}`
  - Done when:
    - primary/diagnostic ID 闭集与 Rust/JSON 一致；
    - old bare IDs parse fail，无 compatibility alias；
    - old committed reports 被明确识别为 legacy 且不能进入 flagship report；
    - fixed-seed primary dry plan 精确 144，pair hashes 完整；
    - 每个 required history episode 有 answer-bearing sanitized `raw_events`；
      validator 拒绝只在 summary/gold `memories` 中存在答案的 fixture；
    - closed `curator_input_projection` schema 只允许 chronological raw-event
      字段；validator 拒绝 gold/expected/target/hidden/scorer 字段；
    - pair identity 分别绑定 actual remem/agent executables 与 target/extraction/
      enrichment/promotion/retrieval profiles，不用单个 `model` hash 代替；
    - schema 对 missing/duplicate/hash drift/extra keys fail closed。
  - Verify:

    ```bash
    python3 eval/coding-bench/validate_schemas.py
    cargo test eval::coding_bench::run_plan
    cargo test eval::coding_bench::fixture
    cargo run -- bench coding --suite issue385-v1 --matrix primary \
      --dry-run --json-out /tmp/gh931-plan.json
    jq -e '.planned_runs == 144' /tmp/gh931-plan.json
    ```

- [ ] `SP931-T3` Owner: isolation-artifact lane; Done when: 每 run 私有边界、immutable attempts、resume、hidden scoring 与 dry-run 零外部调用完整； Verify: isolation/artifact/score fault tests 通过； Covers: B-014-B-020。
  - Owner: isolation-artifact lane
  - Dependencies: SP931-T1B
  - Covers: B-014, B-015, B-016, B-017, B-018, B-019, B-020
  - File ownership:
    - `.gitignore`
    - `src/eval/coding_bench/{approval.rs,isolation.rs,artifact.rs,score.rs}`
  - Done when:
    - HOME/CODEX_HOME/DB/repo/artifact roots 对每 run/condition 唯一，service/
      coordinator 使用独立 OS principal；agent/tool sandbox 只看 task repo，
      看不到 auth/DB/ledger/artifact/private roots，只经 broker 获取 context；
    - agent 与所有 tool subprocess deny outbound network；只能通过不支持任意
      fetch/URL/tool tunneling 的 service-side provider broker，public repo/
      DNS/loopback/metadata canary 均不可达；
    - streaming redactor 在任何 local/bundle write 前检测 credential，命中即
      fail closed 且原 bytes 不落盘；
    - scorer 从 clean fixture + validated patch 建立 agent 不可访问的独立 tree，
      拒绝 symlink/hardlink/device/path collision，hidden oracle/bootstrap
      read-only，score command 使用 argument array；
    - temp-write/fsync/atomic rename、unique attempt、resume-only-missing 生效；
    - timeout/crash/cleanup/scanner/partial/duplicate/hash drift 都有负例；
    - non-self-referential `approval_key` 只由 repo identity、pre-merge policy
      digest 与 PR number 派生，明确排除承载 key 的 blob/tree/commit；隔离
      authority-only GitHub phase 完成 merge/review/blob attestation 后，才允许
      读取 provider/host auth；
    - protected remote ledger ref 以 non-force fast-forward CAS 做跨 clone
      reservation；每次 billable call 前 durable reserve worst-case budget，
      settlement 后追加，crash/abandoned reservation 仍计费；
    - approval 绑定 ledger genesis + ruleset ID/hash；fresh 验证 active
      non-bypassable no-delete/no-force/no-bypass protection 及完整 genesis
      ancestry，保护/audit drift fail closed；
    - target spawn 前将 reservation-bound `target_started` CAS append 到同一
      anchored remote ledger；recovery 将无 terminal 的 started attempt CAS 封为
      `abandoned_after_target_start`/resolved=0，禁止重跑；
    - forged/expired/dismissed/drift/hash/tuple/cap approval，sibling reservation
      race、forced/rollback ledger、resume/new clone/execution ID replay 与累计
      超额均在 provider/agent call 前失败；
    - dry-run mock 证明零 auth/provider/network/agent spawn。
  - Verify:

    ```bash
    cargo test eval::coding_bench::isolation
    cargo test eval::coding_bench::artifact
    cargo test eval::coding_bench::approval
    cargo test eval::coding_bench::score
    ```

- [ ] `SP931-T4` Owner: primary-condition lane; Done when: no-memory control 严格隔离，fixture raw history 通过 production capture/extraction/promotion/retrieval、budgeted curator 接入 runner，且所有 shortcut/condition contamination 被拒绝； Verify: no-memory isolation、isolated E2E、curator adapter 与 provider/drain/ref negative tests 通过； Covers: B-007-B-013, B-017, B-021-B-022。
  - Owner: primary-condition lane
  - Dependencies: SP931-T2, SP931-T3, SP931-T5
  - Covers: B-007, B-008, B-009, B-010, B-011, B-012, B-013, B-017, B-021,
    B-022
  - File ownership:
    - `src/eval/coding_bench/{condition.rs,failure.rs}`
    - `src/eval/coding_bench/tests.rs`（仅本 task 的 E2E/failure sections；若单文件
      ownership 冲突，coordinator 先拆出独立 test module 并更新 manifest）
  - Done when:
    - `no_memory` 正负测试证明 hooks/MCP/SessionStart/repo memory file/
      host-native surface 全无，任一额外 surface 使 run invalid；
    - answer-bearing raw episode 经 production capture/extraction worker/
      promotion/retrieval，gold-only fixture 被 validator 拒绝；
    - E2E path 不调用 seed/save/full-detail preload；
    - provider/drain/promotion/retrieval failure typed 且无降级；
    - capture→use refs 同 run/project 可追溯；
    - budgeted condition 验证 blind curator log/freeze hash/budget/actor type，
      target 只看到冻结后的 MEMORY.md；
    - curator 唯一输入是 schema-allowlisted、hashed
      `curator_input_projection`；artifact 保存 projection，verifier 可从
      fixture 重建，gold/expected/target/hidden/scorer 字段注入均失败；
    - remem-side manual review/promotion 与 curator 使用同一 task/session cost
      denominator；缺 treatment log 时 maintenance claim insufficient；
    - overlapping failures 按 earliest causal stage 选唯一 root，downstream
      consequences 单列；无法判定时显式 suite error。
  - Verify:

    ```bash
    cargo test eval::coding_bench::tests::no_memory_isolation
    cargo test eval::coding_bench::tests::remem_e2e
    cargo test eval::coding_bench::tests::curated_file_budgeted
    cargo test eval::coding_bench::tests::failure_stage
    ```

- [ ] `SP931-T5` Owner: budgeted-curator-contract lane; Done when: blind curator、freeze hash、人工成本与 budget enforcement 的协议/schema 完整且可供 T4 实现； Verify: curator validator 与 target-leak/hash/budget contract negatives 通过； Covers: B-011-B-013。
  - Owner: budgeted-curator-contract lane
  - Dependencies: SP931-T2, SP931-T3
  - Covers: B-011, B-012, B-013
  - File ownership:
    - `eval/coding-bench/curated-file-budgeted-protocol.md`
    - `eval/coding-bench/schemas/curator-log.schema.json`
    - `eval/coding-bench/examples/curator-log.example.json`
  - Done when:
    - curator 输入是可从 fixture 重建且 byte-identical 的 allowlisted
      `curator_input_projection`，无 target/hidden/gold/expected/scorer 字段；
    - frozen MEMORY.md hash 与 log 一致；
    - elapsed/characters/tokens/edits/deletes/conflicts 完整；
    - human 与 automated curator actor 分开，后者不进入 human-cost claim；
    - validator 对 target leak、missing log、hash mismatch、over budget 均
      invalid；T4 按此 contract 在其独占 `condition.rs` 中接线。
  - Verify:

    ```bash
    python3 eval/coding-bench/validate_schemas.py
    ```

- [ ] `SP931-T6` Owner: runner-report lane; Done when: runner 执行 primary/diagnostic、144 completeness、paired bootstrap、cost/stop-loss 与 hash-bound claims； Verify: runner/report/claim focused tests 通过； Covers: B-004-B-006, B-017-B-029。
  - Owner: runner-report lane
  - Dependencies: SP931-T4, SP931-T5
  - Covers: B-004, B-005, B-006, B-017, B-018, B-019, B-020, B-021,
    B-022, B-023, B-024, B-025, B-026, B-027, B-028, B-029
  - File ownership:
    - `src/eval/coding_bench/runner.rs`
    - `eval/claims/{registry.json,claims-registry.schema.json,claim_gate.py}`
    - `eval/coding-bench/reports/{flagship-e2e-v1.json,flagship-e2e-v1.md}`
    - `eval/coding-bench/evidence/flagship-e2e-v1/{run-records.jsonl,source-manifest.json}`
    - `eval/coding-bench/schemas/evidence-source-manifest.schema.json`
    - report/bootstrap implementation must remain in an existing planned
      coding-bench file or update manifest/spec before adding a module
  - Done when:
    - complete matrix/attempt/executable/profile pair-hash validation 在汇总前执行；
    - 每 task 三-run binary mean、target-started failure=0、pre-target missing=
      insufficient 与 fixed-seed 16-task percentile bootstrap 可复现；
    - cost、failure denominator、null missing 和 attribution 完整；
    - immutable registration projection 在任何 official-fixture smoke 前锁定；
      mutable result bindings 不改变 run digest，outcome 后改 projection invalid；
    - superiority lower bound、non-inferiority lower bound >=-3pp、treatment-side
      review cost、固定 48-run stop-loss denominator 与 attribution-missing
      insufficient 全有边界测试；
    - 144 个 scanner-passed sanitized run records 和 source manifest 完整，
      report 可仅从该 bundle 重算且 input hash 一致；
    - 未执行 live matrix 时 report/registry 保持 `INSUFFICIENT`。
  - Verify:

    ```bash
    cargo test eval::coding_bench::tests::runner
    cargo test eval::coding_bench::tests::report
    cargo test eval::coding_bench::tests::paired_bootstrap
    cargo test eval::coding_bench::tests::evidence_bundle
    python3 eval/claims/claim_gate.py --self-test
    python3 eval/claims/claim_gate.py check
    ```

- [ ] `SP931-T7` Owner: integration-doc lane; Done when: CLI/docs/public gate/version 接线且仍准确声明无 official result； Verify: CLI parse、public claim、version sync 与 repository gates 通过； Covers: B-001-B-003, B-020, B-028-B-030。
  - Owner: integration-doc lane
  - Dependencies: SP931-T6
  - Covers: B-001, B-002, B-003, B-020, B-028, B-029, B-030
  - File ownership:
    - `src/cli/{eval_types.rs,tests_eval.rs}`
    - `src/cli/actions/eval.rs`
    - `scripts/ci/check_public_claims.py`
    - `eval/coding-bench/README.md`
    - `docs/ARCHITECTURE.md`
    - `docs/specs/README.md`
    - `docs/specs/GH931/{PRODUCT.md,TECH.md}`
    - `docs/specs/issue385-coding-agent-ab/{PRODUCT.md,TECH.md}`
    - `docs/specs/public-memory-benchmark/{PRODUCT.md,TECH.md}`
    - `README.md`, `README.zh-CN.md`, `CHANGELOG.md`
    - version-sync files in `tech.md`
  - Done when:
    - CLI 默认 primary、显式 diagnostic、live confirm/hard caps 和 output paths；
    - dry-run 写可验证 JSON 且零外部调用；
    - docs 保持 scaffold/implementation/official evidence 状态分离；
    - non-PASS + positive public wording 被 CI 拒绝；PASS 时任何不等于
      maintainer-approved exact wording/report link 的幅度、范围或改写也被拒绝；
    - version surfaces 同步。
  - Verify:

    ```bash
    cargo test cli::tests_eval
    python3 scripts/ci/check_public_claims.py --self-test
    python3 scripts/ci/check_public_claims.py
    python3 scripts/ci/check_plugin_version_sync.py
    ```

## 验证与执行

- [ ] `SP931-T8` Owner: independent reviewer lane; Done when: invariants、shortcut/security boundaries 与所有 repository gates 有 fresh exact-head evidence； Verify: offline/focused/full gates 全通过； Covers: B-001-B-030。
  - Owner: independent reviewer lane
  - Dependencies: SP931-T7
  - Covers: B-001-B-030
  - Done when:
    - 独立 reviewer 检查 E2E 未 seed/preload、curator blind、auth/hidden isolation、
      attempt denominator、statistics 和 public wording；
    - fresh offline/focused/full commands 全绿；
    - review artifact 绑定 exact head，findings 全部闭环；
    - 不弱化测试或门禁。
  - Verify:

    ```bash
    python3 eval/coding-bench/validate_schemas.py
    python3 eval/claims/claim_gate.py check
    cargo run -- bench coding --suite issue385-v1 --matrix primary \
      --dry-run --json-out /tmp/gh931-plan.json
    jq -e '.planned_runs == 144' /tmp/gh931-plan.json
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

- [ ] `SP931-T9` Owner: maintainer/security/cost owner; Done when: smoke 与 full live agent/provider/auth/cost 分别获得限额授权； Verify: trusted approval 绑定 exact hashes/tuples/caps； Covers: B-015, B-020, B-028, B-030。
  - Owner: maintainer/security/cost owner
  - Dependencies: SP931-T8
  - Covers: B-015, B-020, B-028, B-030
  - Done when:
    - immutable registration projection 在任何 official-fixture live smoke 前
      已锁定；maintainer 再批准每个 primary condition 一个 smoke tuple；
    - smoke 的 isolation/capture/curator/cleanup/artifact 经人工复核后，再单独
      批准 144-run official matrix；
    - approval 绑定 exact code/fixture/registration、actual remem/agent binaries、
      target/extraction/enrichment/promotion/retrieval profiles、timeout、tuple
      selectors、expiry、max agent/LLM calls 和 max estimated cost；
    - credential bytes 不进入 approval/artifact；
    - smoke 永久排除于 official denominator。
    - smoke/full approval entry 通过
      `eval/coding-bench/schemas/live-run-approval.schema.json`，经独立
      maintainer APPROVED review 的 PR merge 到 default branch；entry 绑定
      exact hashes/tuple selectors/有效期/caps 且不含 credential bytes；
      `approval_key` 只由 repo identity + policy digest + approval PR number
      派生，承载 key 的 blob/tree/commit 与 review/merge attestation 均不进入
      preimage；approval 另绑定 ledger genesis/ruleset；
    - runner 的 negative suite 已证明 caller 自选 key、未 merge/未 APPROVED/
      过期 registry、authority credential scope 扩大、跨 clone/`execution_id`
      replay、pre-call crash、abandoned reservation、ledger rollback 与拆单超额
      均在 provider/host/agent call 前失败。
  - Verify:

    ```bash
    test -n "$GH931_LIVE_APPROVAL_KEY"
    test -n "$GH931_MAX_AGENT_CALLS"
    test -n "$GH931_MAX_LLM_CALLS"
    test -n "$GH931_MAX_ESTIMATED_COST_USD"
    cargo test eval::coding_bench::approval
    cargo test eval::coding_bench::tests::cumulative_usage_ledger
    cargo test eval::coding_bench::tests::pre_call_reservation_crash
    cargo test eval::coding_bench::tests::started_attempt_recovery
    cargo test eval::coding_bench::tests::ledger_protection_anchor
    ```

- [ ] `SP931-T10` Owner: authorized benchmark operator; Done when: 144 个 primary tuple 都有 immutable verified artifacts； Verify: final verifier 报告 `valid_primary_runs == 144`； Covers: B-004-B-006, B-014-B-024。
  - Owner: authorized benchmark operator
  - Dependencies: SP931-T9
  - Covers: B-004, B-005, B-006, B-014, B-015, B-016, B-017, B-018,
    B-019, B-020, B-021, B-022, B-023, B-024
  - File ownership（T6 implementation 完成后顺序移交）:
    - `eval/coding-bench/evidence/flagship-e2e-v1/run-records.jsonl`
    - `eval/coding-bench/evidence/flagship-e2e-v1/source-manifest.json`
  - Done when:
    - 16 tasks × 3 primary × 3 runs 的每个 tuple 都有 verified artifact；
    - failed outcomes 留在分母，attempt history 保留；
    - no secret/HOME/hidden/cross-run leak；
    - run 使用 locked registration projection、actual approved executable/profile
      digests 和 protected remote reservation ledger；
    - report 可从 artifacts 重建。
    - scanner-passed sanitized attempts/runs 写入 committed
      `run-records.jsonl`，`source-manifest.json` 绑定全部 attempt、matrix、
      scanner、code/fixture/registry 与 report-input hashes；raw/private/secret
      evidence 不进入 git。
  - Verify:

    ```bash
    test -x "$GH931_APPROVED_REMEM_BIN"
    test "$(shasum -a 256 "$GH931_APPROVED_REMEM_BIN" | awk '{print $1}')" = \
      "$GH931_APPROVED_REMEM_SHA256"
    "$GH931_APPROVED_REMEM_BIN" bench coding --suite issue385-v1 --matrix primary \
      --approval-key "$GH931_LIVE_APPROVAL_KEY" --confirm-live-run \
      --max-agent-calls "$GH931_MAX_AGENT_CALLS" \
      --max-llm-calls "$GH931_MAX_LLM_CALLS" \
      --max-estimated-cost-usd "$GH931_MAX_ESTIMATED_COST_USD" \
      --json-out /tmp/gh931-primary.json
    jq -e '.valid_primary_runs == 144' /tmp/gh931-primary.json
    cargo run -- bench coding-report \
      --input eval/coding-bench/evidence/flagship-e2e-v1 \
      --json-out /tmp/gh931-recomputed-report.json
    ```

- [ ] `SP931-T11` Owner: report-claim lane + maintainer; Done when: paired report、cost、stop-loss、wording、review/merge/issue/release 决策完整； Verify: claim/public checks 与 exact-head PR gate 通过； Covers: B-023-B-030。
  - Owner: report-claim lane + maintainer
  - Dependencies: SP931-T10
  - Covers: B-023, B-024, B-025, B-026, B-027, B-028, B-029, B-030
  - File ownership（T10 evidence seal 后顺序移交）:
    - `eval/coding-bench/reports/{flagship-e2e-v1.json,flagship-e2e-v1.md}`
    - `eval/claims/registry.json`（仅 `result_bindings`）
    - `README.md`, `README.zh-CN.md`, `CHANGELOG.md`（仅 PASS + exact wording approval）
  - Done when:
    - JSON/Markdown report、committed sanitized run-record bundle、
      source manifest/hash 与 registry 一致；
    - paired CI、maintenance cost、memory-harm stop-loss 可复算；
    - registration projection 保持 byte-identical；只更新 result bindings；
    - PASS/FAIL/INSUFFICIENT 都保留 report；只有 PASS 才由 maintainer 批准
      exact public wording，public files 必须逐字匹配并链接相同 report；
    - independent review、CI、threads/PR gate 与 closure audit 完成；
    - maintainer 单独决定 merge、是否关闭 #931 与 release。
  - Verify:

    ```bash
    python3 eval/claims/claim_gate.py check
    python3 scripts/ci/check_public_claims.py
    test -n "$PR_NUMBER"
    python3 checks/github_pr_evidence.py --github-repo majiayu000/remem \
      --repo . --pr "$PR_NUMBER" --issue 931 --review-manifest "$REVIEW_MANIFEST" \
      --json > /tmp/gh931-pr-evidence.json
    python3 checks/pr_gate.py --repo . \
      --evidence /tmp/gh931-pr-evidence.json --mode required --json
    ```

## 并行拆分

- SP931-T1A 必须在 T1 后独立落地；T1B 必须在 T1A merge 后用 post-registry
  base/exact head 重新通过 implement gate；T1B 前不得启动 T2-T11。
- SP931-T2 与 SP931-T3 在 T1B 后可并行，文件所有权不重叠。
- SP931-T5 必须等待 T2/T3 完成，再独占 curator contract 文件；T4 等待 T5
  完成后独占 `condition.rs`/`failure.rs` 与指定 test sections。
- T6 等待 T4/T5 后独占 runner/report/claim integration。
- T7 串行处理共享 CLI/docs/version/enforcement-sensitive 文件。
- live T10 不得与任何改变 code/fixture/registration/executable/profile hash
  的 lane 并行；T10 seal evidence 后才把 result-path write ownership 移交 T11。
- 每个 agent 只能写任务列明的文件；新增路径先更新 tech manifest/spec approval。

## Handoff Notes

- `origin/main@5627a74942a41f51bdc03518fce726dbf1b46098`：
  PR #936 scaffold 已合并；schema/claim self-tests fresh pass。
- 当前 Rust dry-run 仍输出旧
  `no_memory/remem/curated_file` 144 plan；这只是 legacy runner truth，不是
  primary flagship evidence。
- GH-931 已获 `ready_to_spec`；仍缺 exact packet `spec_approval`、
  `ready_to_implement` 和 enforcement-sensitive implement evidence。
- 本 packet 没有执行 live benchmark，没有使用 provider/auth，没有生成
  flagship report，也没有授权 public claim、merge、close 或 release。
- Product invariants `B-001`…`B-030` 全部由 T2-T11 覆盖。
