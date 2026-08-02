# Task Plan

## Linked Issue

GH-931

## Spec Packet

- Current product contract: `docs/specs/GH931/PRODUCT.md`
- Current tech contract: `docs/specs/GH931/TECH.md`
- Historical product rationale: `specs/GH931/product.md`
- Historical tech rationale: `specs/GH931/tech.md`
- Locale: zh-CN
- Status: Historical planning evidence；按当前治理它不是 active workflow、
  readiness label 或 execution prerequisite，也不机械授权/阻止 implementation；
  本计划描述的 security/live/final-review 人工前置条件仍未满足，未开始
  implementation 或 live run。

## Human Gates First

- [ ] `SP931-T1` Owner: maintainer + independent security owner; Done when:
  current `docs/specs/GH931/{PRODUCT,TECH}.md` contract 与 supporting historical
  product/tech/tasks planning diff 经人工批准并 merge 到 default branch； Verify:
  review/merge evidence 绑定 exact current-contract base/head/path set，任何
  contract drift 都需重新审批； Covers: B-020, B-030。
  - Owner: maintainer + independent security owner
  - Dependencies: none
  - Covers: B-020, B-030
  - Done when:
    - 非作者 maintainer 与独立 security owner 都对同一 exact
      `docs/specs/GH931/{PRODUCT,TECH}.md` current-contract diff 提交未
      dismissed 的 `APPROVED` review；root packet 只提供 supporting rationale，
      其状态或 merge 本身不构成 workflow gate；
    - 审批记录绑定 repository、base/head commit、完整 path set 与 diff digest，
      并明确覆盖 public-claim boundary、approval schema/registry、authority
      broker、remote ledger、credential isolation、provider hard limits 与 cost
      caps；
    - current-contract spec PR 只能由 maintainer 人工 merge；implementation、
      live run、final PR review、merge 与 release 仍分别保留人工 gate；
    - implementation coordinator fresh 验证批准的 exact current-contract head
      已进入
      `origin` default branch；审批后的 path/content drift 一律使批准失效，
      T2/T3 必须等待重新审批。
  - Verify:
    ```bash
    gh pr view "$SPEC_PR_NUMBER" \
      --json baseRefOid,headRefOid,mergeCommit,mergedAt,reviews,state
    git fetch origin main
    git merge-base --is-ancestor "$GH931_APPROVED_SPEC_HEAD_SHA" origin/main
    test "$(git rev-parse "$GH931_APPROVED_SPEC_HEAD_SHA^{tree}")" = \
      "$GH931_APPROVED_SPEC_TREE_SHA"
    ```
## 实现任务

- [ ] `SP931-T2` Owner: condition-contract lane; Done when: condition registry、Rust IDs、schemas 与 legacy artifact policy 收敛且无 alias； Verify: validator、ID parse tests 与 primary 144-plan tests 通过； Covers: B-001-B-006, B-013。
  - Owner: condition-contract lane
  - Dependencies: SP931-T1
  - Covers: B-001, B-002, B-003, B-004, B-005, B-006, B-013
  - File ownership:
    - `eval/coding-bench/{benchmark-charter.json,conditions.json,validate_schemas.py}`
    - `eval/coding-bench/schemas/{conditions.schema.json,flagship-run.schema.json,flagship-report.schema.json}`
    - `eval/coding-bench/schemas/live-run-approval.schema.json`
    - `eval/coding-bench/live-run-approvals.json`（仅 closed empty registry
      scaffold 与 validator fixtures；T8 完成后顺序移交 T9 写 live entries）
    - `eval/coding-bench/fixtures/tasks.json`
    - `src/eval/coding_bench/{types.rs,run_plan.rs,fixture.rs}`
    - `src/eval/coding_bench/{artifact.rs,condition.rs,runner.rs,tests.rs}`
      （仅 closed-ID/exhaustive-match migration；完成后依次移交 T3/T4/T6）
  - Done when:
    - primary/diagnostic ID 闭集与 Rust/JSON 及全部 exhaustive consumer 一致，
      T2 的 focused tests 与 `cargo check` 不留下等待后续 task 修复的红 build；
    - old bare IDs parse fail，无 compatibility alias；
    - old committed reports 被明确识别为 legacy 且不能进入 flagship report；
    - primary dry plan 精确 144；schema 要求 registered `evaluation_as_of`/
      virtual-clock policy、condition-order seed/PRNG version、完整 tuple
      permutation/digest，planner 重算不一致或读取真实 clock 均失败；
    - 每个 required history episode 有 answer-bearing sanitized `raw_events`；
      projection 按注册的 nested arrays 派生连续 `source_ordinal=0..N-1`，
      timestamp 非递减/同秒可用、event ID 不排序；ordinal 入 call content 且
      call index/row ID 严增，validator 拒绝顺序/ordinal/gold-only drift；
    - validator 按 closed schema/example registry 通用发现；T3A 后续增加
      checkpoint schema/example 不需回写 T2 文件；
    - closed `curator_input_projection` schema 只允许 chronological raw-event
      字段；validator 拒绝 gold/expected/target/hidden/scorer 字段；
    - T2 独占的 `flagship-run.schema.json` 要求 closed review projection/hashes、
      supervisor timing、pre-target/target fields 与 receipt-free RFC 8785 JCS
      terminal payload；payload 拒绝 terminal/checkpoint receipt 及自身 digest
      派生字段，detached source manifest 另绑 receipts；T4/T6 不修改该 schema；
    - live approval schema 要求 canonical USD pricing snapshot、SKU/rates、
      per-call token ceilings/rounding/provider hard-limit capability fields，
      拒绝 caller-supplied cost；同时要求 policy-derived
      `run_phase`/`matrix_namespace`、exact ledger ref/genesis、ledger-writer
      GitHub App installation/repository identity、signature algorithm/key ID/
      public-key digest、update-authority 与 no-bypass integrity 两个 ruleset
      ID/hash，以及 Sigstore TUF `TrustedRoot`/`SigningConfig` digests、Rekor
      operator/log IDs、accepted API/DSSE versions、minimum reviewed bundle/
      checkpoint；unknown/missing writer/ruleset/TUF/Rekor fields fail closed；
    - pair identity 分别绑定 OS-anchored host supervisor、actual remem/agent
      executables 与 target/extraction/enrichment/promotion/retrieval profiles，
      不用单个 `model` hash 代替；
    - schema 对 missing/duplicate/hash drift/extra keys fail closed。
  - Verify:
    ```bash
    python3 eval/coding-bench/validate_schemas.py
    cargo test eval::coding_bench::run_plan
    cargo test eval::coding_bench::run_plan::tests::registered_clock_and_order
    cargo test eval::coding_bench::fixture
    cargo check
    cargo run -- bench coding --suite issue385-v1 --matrix primary \
      --dry-run --json-out /tmp/gh931-plan.json
    jq -e '.planned_runs == 144' /tmp/gh931-plan.json
    ```
- [ ] `SP931-T3` Owner: isolation-artifact lane; Done when: 每 run 私有边界、immutable attempts、resume、hidden scoring 与 dry-run 零外部调用完整； Verify: isolation/artifact/score fault tests 通过； Covers: B-014-B-020。
  - Owner: isolation-artifact lane
  - Dependencies: SP931-T2
  - Covers: B-014, B-015, B-016, B-017, B-018, B-019, B-020
  - File ownership:
    - `.gitignore`
    - `Cargo.toml`（仅新增 reviewed supervisor bin target；完成后移交 T3A）
    - `src/bin/remem-bench-supervisor.rs`
    - `src/eval/coding_bench.rs`（T2 后接收；声明已有文件对应的 child module）
    - `src/eval/coding_bench/{approval.rs,isolation.rs,artifact.rs,provider_adapter.rs,score.rs,verified_exec.rs}`
  - Done when:
    - HOME/CODEX_HOME/DB/repo/artifact roots 对每 run/condition 唯一，service/
      coordinator 使用独立 OS principal；agent/tool sandbox 只看 task repo，
      看不到 auth/DB/ledger/artifact/private roots；context 只走 audited MCP/
      SessionStart frames，provider 只走下述 private adapter；
    - target namespace 拒绝 DNS、public/RFC1918/metadata 与 Unix sockets。
      pinned Codex process 仅可连 private loopback provider adapter；adapter 无
      network，只经 sandbox 前创建的 bounded inherited pipes 转发 fixed-schema
      provider RPC。tool subprocess 连 loopback 也被 OS policy 拒绝；若平台/
      pinned agent 不能证明该进程级隔离，feasibility gate fail closed；
    - streaming redactor 在任何 local/bundle write 前检测 credential，命中即
      fail closed 且原 bytes 不落盘；
    - hidden scorer 使用独立 OS principal/process/read-only tree；controller 永不
      import/exec patch，无 hidden mount 的 code worker 只走 bounded、schema-checked
      RFC 8785 JCS RPC。stdout/exit0/visible tests/self-report 不能定 PASS；
      monkeypatch/shared interpreter、RPC/异常失败及 symlink/hardlink/device/
      path collision 均 fail closed；
    - temp-write/fsync/atomic rename、unique attempt、resume-only-missing 生效；
    - timeout/crash/cleanup/scanner/partial/duplicate/hash drift 都有负例；
    - non-self-referential `approval_key` 只由 repo identity、pre-merge policy
      digest 与 PR number 派生，明确排除承载 key 的 blob/tree/commit；隔离
      authority broker 与 provider/host principal 分开，GitHub credential 不向
      后者暴露；
    - T3 原子创建 `approval.rs`/`provider_adapter.rs`/`verified_exec.rs` 并在
      parent 声明对应 module；T3A 接收 parent 后才原子创建/声明 checkpoint，
      任一 task 结束时 crate 都可编译。T3 不再修改已移交 T4/T6 的
      condition/runner/tests；
    - provider adapter 使用 current pinned Codex HTTP `model_provider` contract；
      capability test 证明 agent model traffic 成功而 agent browser/public fetch
      与每类 tool subprocess 的 public/loopback/Unix-socket attempts 均失败；
    - 固定 `/usr/local/libexec/remem-bench-supervisor` 由独立 security owner
      以 root-owned immutable service provision；approval 绑定其 OS code-sign/
      fs-verity/unit measurement 与 attestation public key，私钥对 caller 不可读。
      supervisor 自 authoritative approval 取 digest，不能收 caller expected
      digest；它对 CAS leaf 做 `openat(O_NOFOLLOW)`、owner/mount/fstat/same-fd
      hash，并用 Linux
      `execveat(AT_EMPTY_PATH)` 或 reviewed platform-equivalent same-handle exec；
      无安全等价 primitive 的平台不允许 live run；
    - wrong supervisor measurement/key/path、伪造 attestation 与 direct remem
      invocation 均在 authority/provider/agent access 前失败；
    - approval schema 绑定 USD canonical pricing snapshot、provider/model SKU、
      effective timestamp、input/output/cache/tool rates、每 call-kind token
      ceilings 与向上取整；broker 忽略/拒绝 caller cost，使用 checked arithmetic
      计算 conservative reservation，unknown/drift/overflow fail closed；
    - dispatch 前用 canonical tokenizer 验证 serialized input/cache/tool tokens，
      provider hard-limit 强制 output/reasoning/cache/tool ceilings；SKU/API
      不能强制或无法超限前终止则不 dispatch；
    - protected remote ledger ref 以 non-force fast-forward CAS 做跨 clone
      reservation；每次 billable call 前 durable reserve worst-case budget，
      settlement 后追加，crash/abandoned reservation 仍计费；
    - exact ref 的 update-authority ruleset 只把 approval-pinned ledger-writer
      GitHub App 列为 `Restrict updates` 唯一 bypass actor；第二个 integrity
      ruleset 无 bypass actor并对所有 actor restrict deletion、block force push、
      require signed commits。两个 ID/hash/active target 都逐次验证；
    - 每个 record 验证 writer signature；每次 remote CAS 后必须取得 T3A 定义的
      verified Rekor bundle。missing/wrong/revoked writer、任一 ruleset drift 或
      Rekor/TUF rollback 或 proof inconsistency 都在 transition/dispatch/network-freshness
      前失败；
    - approval 绑定 ledger genesis、sole writer App、两个 rulesets 与 TUF/Rekor
      trust；每个 reservation/transition/terminal seal 前 fresh 验证 approval
      expiry、merge/review、rulesets、writer signature、latest Rekor bundle/
      consistency proof 与完整 genesis ancestry，任一 protection/writer/log/
      history drift fail closed；
    - curator/reviewer/preparation 前将 `pre_target_work_started` CAS append；
      supervisor monotonic timing/output digests durable，abandon/crash 形成
      `abandoned_before_target`、保守计时并封闭 run index，禁止 fresh-clone 重做；
    - target spawn 前将 reservation-bound `target_started` CAS append 到同一
      anchored remote ledger；recovery 将无 terminal 的 started attempt CAS 封为
      `abandoned_after_target_start`/resolved=0，禁止重跑；
    - scanner 后 supervisor 先 hash receipt-free JCS payload，再 CAS seal matrix
      key + payload/cost/timing/frozen digests；source manifest 随后 detached 绑定
      attestation/checkpoint，payload→ledger→checkpoint→mapping 不完整不可
      resume/report；
    - forged/expired/dismissed/drift/hash/tuple/cap approval，sibling reservation
      race、forced/rollback ledger、resume/new clone/execution ID replay 与累计
      超额均在 provider/agent call 前失败；
    - dry-run mock 证明零 auth/provider/network/agent spawn。
  - Verify:
    ```bash
    cargo test eval::coding_bench::isolation
    cargo test eval::coding_bench::artifact
    cargo test eval::coding_bench::approval
    cargo test eval::coding_bench::provider_adapter
    cargo test eval::coding_bench::verified_exec
    cargo test eval::coding_bench::score
    ```
- [ ] `SP931-T3A` Owner: transparency-checkpoint lane; Done when: Sigstore
  Rekor public-good transparency log protocol、TUF trust/shard rotation、
  inclusion/consistency proof 与 crash recovery 可离线复验； Verify:
  checkpoint schema/client fixtures 与 failure tests 通过； Covers: B-018, B-020。
  - Owner: transparency-checkpoint lane
  - Dependencies: SP931-T3
  - Covers: B-018, B-020
  - File ownership:
    - `eval/coding-bench/checkpoint-protocol.md`
    - `eval/coding-bench/schemas/checkpoint-receipt.schema.json`
    - `eval/coding-bench/examples/checkpoint-receipt.example.json`
    - `eval/coding-bench/fixtures/rekor-v2/{trusted-root.json,signing-config.json,bundle.json}`
    - `Cargo.toml`, `Cargo.lock`（从 T3 接收，仅改 reviewed dependency；后交 T7）
    - `src/eval/coding_bench.rs`（从 T3 顺序接收，仅与下列文件原子声明）
    - `src/eval/coding_bench/checkpoint.rs`
    - `src/eval/coding_bench/checkpoint/{client.rs,proof.rs,tests.rs,trust.rs}`
  - Done when:
    - `checkpoint.rs` 与 parent 的 `mod checkpoint;` 同一提交落地并通过
      `cargo check`，不存在声明先于文件的 build-red handoff；
    - TUF/signature/Merkle 使用 security-reviewed、version/checksum-locked Rust
      dependencies 与 committed offline conformance fixtures；禁止 shell-out、
      自写 crypto 或把全部 client/trust/proof/tests 堆进单个 >800-line module；
    - protocol 固定 digest-only signed DSSE payload 和 accepted Sigstore/Rekor
      API/entry/bundle versions；不上传 prompt、artifact、credential 或 private
      evidence；
    - active Rekor shard URL、operator/log IDs 与 verification keys 只从
      approval-pinned TUF `TrustedRoot`/`SigningConfig` 验证发现，支持 reviewed
      key rotation 与 inactive shard verification，不 hard-code endpoint；
    - client 验证 payload/writer signature、inclusion proof、signed checkpoint、
      从 minimum reviewed checkpoint 的 consistency proof、previous bundle
      digest 与严格递增 log index；不错误要求全局 index 连续；
    - Git CAS 成功但 Rekor submit/receipt 持久化中断时，只能为 exact signed tip
      查找或补交同 digest entry；verified bundle 前不 dispatch。duplicate
      sequence + different payload、ledger fork、checkpoint rollback/proof
      inconsistency、stale TUF、
      unknown operator/log/API 与 unavailable service 全部 fail closed；
      无独立 witness/gossip 时，恶意 Rekor operator 的 self-consistent split
      view 明确列为 residual risk，不得被 report 写成已检测；
    - schema/example 覆盖 TUF metadata digests、operator/log identity、entry UUID/
      log index、inclusion/consistency proof、signed checkpoint、payload/
      previous-bundle digests，并固定
      `view_assurance=operator_consistency_only`；offline verifier 不信任 live
      endpoint 自报 key。
  - Verify:
    ```bash
    python3 eval/coding-bench/validate_schemas.py
    cargo test eval::coding_bench::checkpoint
    cargo check
    ```
- [ ] `SP931-T4` Owner: primary-condition + production-clock lane; Done when:
  no-memory control 严格隔离，fixture raw history 通过 production
  capture/extraction/promotion/retrieval、budgeted curator 接入，且完整
  SessionStart/PromptSubmit/MCP 语义链只消费 registered clock； Covers:
  B-005, B-007-B-013, B-017, B-021-B-022。
  - Owner: primary-condition lane
  - Dependencies: SP931-T2, SP931-T3, SP931-T5
  - Covers: B-005, B-007, B-008, B-009, B-010, B-011, B-012, B-013, B-017,
    B-021, B-022
  - File ownership:
    - `eval/coding-bench/{evaluation-clock-scope.json,production-drain-scope.json}`
    - `scripts/ci/check_evaluation_clock_scope.py`
    - `src/eval/coding_bench/{condition.rs,failure.rs}`
    - `src/{clock.rs,lib.rs,context.rs}`
    - `src/context/{types.rs,render.rs,render_inputs.rs,query.rs,audit.rs,fact_labels.rs,hybrid_context.rs,prompt_submit.rs}`
    - `src/context/render/eval.rs`
    - `src/mcp/{server.rs,server/runtime.rs,server/search_tools.rs,server/context_tools.rs}`
    - `src/mcp/{mod.rs,types.rs,server/benchmark.rs,server/errors.rs}`
    - `src/memory/{types.rs,lesson.rs,preference.rs}`
    - `src/memory/{preference/query.rs,preference/render.rs,service/search.rs,service/types.rs,store/read.rs}`
    - `src/db/{capture.rs,capture/extraction_task.rs,extraction/enqueue.rs}`
    - `src/{extraction_worker.rs,observation_extract.rs,memory_candidate.rs}`
    - `src/memory_candidate/{apply.rs,review.rs,review/approval.rs}`
    - `src/memory/{dedup.rs,dedup/access.rs,dedup/funnel.rs,dedup/hash.rs,lifecycle.rs,graph_contract.rs,edge.rs}`
    - `src/memory/{procedure/mod.rs,procedure/trace_store.rs,store/write.rs}`
    - `src/graph_candidate/{mod.rs,review.rs,source.rs,conflict_bridge.rs,tests.rs}`
    - `src/graph_candidate/tests/review_regressions.rs`
    - `src/db/observation.rs`
    - `src/retrieval/{search.rs,search/memory.rs,search_multihop.rs,rerank.rs,vector.rs,vector_candidates.rs,memory_search.rs,entity.rs,entity/search.rs,temporal.rs}`
    - `src/retrieval/search/memory/{runner.rs,listing.rs,text.rs,source_anchor.rs,usage_rank.rs}`
    - `src/retrieval/search/memory/text/graph.rs`
    - `src/retrieval/search_multihop/{search.rs,expand.rs}`
    - `src/retrieval/memory_search/{fts.rs,like.rs}`
    - `src/retrieval/entity/search/{runner.rs,lookup.rs,sql.rs}`
    - `src/retrieval/graph/{query.rs,traverse.rs}`
    - `src/retrieval/temporal/{parse.rs,fact_keys.rs,fact_labels.rs,search.rs}`
  - Done when:
    - `no_memory` 正负测试证明 hooks/MCP/SessionStart/repo memory file/
      host-native surface 全无，任一额外 surface 使 run invalid；
    - answer-bearing raw episode 经 production capture/extraction worker/
      promotion/retrieval，gold-only fixture 被 validator 拒绝；
    - E2E path 不调用 seed/save/full-detail preload；
    - provider/drain/promotion/retrieval failure typed 且无降级；
    - fixture 用 stable event ID 只作 identity，nested-array-derived ordinal 入 call content 并使 call/row ID 严增；
      以 `task_kind=ObservationExtract` 进入 production
      capture，顺序/timestamp 负例失败；单个 normal `run_next` worker 只允许
      ObservationExtract→MemoryCandidate→GraphCandidate 并 drain 至 quiescent。
      exact replay、unexpected kind、failed/residual/capped task 全部 invalid；
    - immutable `EvaluationClock` 不在 env/global/target request；normal outer
      adapter 每 request snapshot 一次 system time，benchmark 只从 run identity
      注入 `evaluation_as_of`；
    - SessionStart 的 condition→render/eval→render_inputs→query→hybrid/fact/
      lesson/preference→audit，以及 MCP server→service→queryless/multihop/search→
      temporal/fact/entity/vector/graph/usage/source-anchor/rerank/explain 与
      get-detail/access-feedback 全程显式传同一 clock；
    - memory expiry SQL 改为 bound epoch；semantic scope 内无 `Utc/Local/
      SystemTime::now` 或 SQLite `now`。expiry、summary age、lesson stale-after、
      relative date/current year、fact/graph validity、usage、audit/explain 和
      subsequent access ranking 的跨层测试固定输出；
    - capture/candidate/promotion/dedup/TTL/validity/graph 的 semantic writes
      使用同一 clock；queue lease/retry 与 operation audit 可用 real clock，但
      inventory/test 必须证明不影响 target-visible selection/content/order；
    - benchmark MCP `tools/list` 精确为 `search`/`get_observations`；search 禁止
      raw fallback，detail 只收本连接 search 签发的 `source=memory` IDs。其他
      tool/alias/unknown field、observation source 与 access-write failure 均在
      泄漏/修改 DB 前显式失败，capability 不跨 run；
    - scope inventory 区分 evaluation clock 与真实 approval/budget/timeout/
      monotonic/纯写入 audit clock；security clock 不可被 virtualize，后者也不可
      影响 target-visible content/order；
    - capture→use refs 同 run/project 可追溯；
    - budgeted condition 验证 blind curator log/freeze hash/budget/actor type，
      target 只看到 committed content-addressed record 中 hash 匹配的 frozen
      `MEMORY.md` exact bytes；
    - curator 唯一输入是 schema-allowlisted、hashed
      `curator_input_projection`；artifact 保存 projection，verifier 可从
      fixture 重建，gold/expected/target/hidden/scorer 字段注入均失败；
    - remem-side manual review/promotion 与 curator 使用同一 task/session cost
      denominator；缺 treatment log 时 maintenance claim insufficient；
    - treatment reviewer 只看到可重建、hashed、gold-free 的
      `treatment_review_input_projection`；同 task 三个 run 的 control/treatment
      preparation 在首个 target reveal 前 batch-freeze，或 ledger 证明后续 actor
      未暴露；target/gold/hidden/scorer/outcome 注入、暴露 actor 复用或
      post-reveal edit 失败；
    - overlapping failures 按 earliest causal stage 选唯一 root，downstream
      consequences 单列；无法判定时显式 suite error。
  - Verify:
    ```bash
    python3 scripts/ci/check_evaluation_clock_scope.py
    cargo test eval::coding_bench::condition::tests::no_memory_isolation
    cargo test eval::coding_bench::condition::tests::remem_e2e
    cargo test eval::coding_bench::condition::tests::curated_file_budgeted
    cargo test eval::coding_bench::condition::tests::cross_run_blindness
    cargo test eval::coding_bench::condition::tests::production_clock_injection
    cargo test eval::coding_bench::condition::tests::benchmark_mcp_closed
    cargo test eval::coding_bench::failure::tests
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
    - protocol 要求同 task 三 repetitions 在任何 target reveal 前 batch-freeze，
      或由 ledger-bound stable actor assignment 证明 curator 未暴露；
    - frozen `MEMORY.md` sanitized exact bytes 写入 content-addressed evidence，
      digest 与 log/target mount 一致；
    - elapsed 只能来自 trusted supervisor monotonic start/end receipt，绑定
      interaction/projection/output；actor 自报 elapsed 被拒绝，characters/
      tokens/edits/deletes/conflicts 完整；
    - human 与 automated curator actor 分开，后者不进入 human-cost claim；
    - validator 对 target leak、missing log、hash mismatch、over budget 均
      invalid；T4 按此 contract 在其独占 `condition.rs` 中接线。
  - Verify:
    ```bash
    python3 eval/coding-bench/validate_schemas.py
    ```

- [ ] `SP931-T6` Owner: runner-report lane; Done when: runner 执行 primary/diagnostic、144 completeness、paired bootstrap、cost/stop-loss 与 hash-bound claims； Verify: runner/report/claim focused tests 通过； Covers: B-004-B-006, B-017-B-029。
  - Owner: runner-report lane
  - Dependencies: SP931-T3A, SP931-T4, SP931-T5
  - Covers: B-004, B-005, B-006, B-017, B-018, B-019, B-020, B-021,
    B-022, B-023, B-024, B-025, B-026, B-027, B-028, B-029
  - File ownership:
    - `src/eval/coding_bench/runner.rs`
    - `src/eval/coding_bench/tests.rs`
    - `eval/claims/{claims-registry.schema.json,claim_gate.py}`
    - `eval/coding-bench/reports/{flagship-e2e-v1.json,flagship-e2e-v1.md}`
    - `eval/coding-bench/evidence/flagship-e2e-v1/{frozen-control-content.jsonl,run-records.jsonl,source-manifest.json}`
    - `eval/coding-bench/schemas/evidence-source-manifest.schema.json`
    - report/bootstrap implementation must remain in an existing planned
      coding-bench file or update manifest/spec before adding a module
  - Done when:
    - runner 是 `approval.rs`、`checkpoint.rs`、condition producer 与 report
      verifier 的唯一 integration owner；T3/T3A/T4 不共享 runner/tests write；
    - complete matrix/attempt/executable/profile pair-hash validation 在汇总前执行；
    - 每 task 三-run binary mean、target-started failure=0、pre-target missing=
      insufficient 与 fixed-seed 16-task percentile bootstrap 可复现；
    - cost、failure denominator、null missing 和 attribution 完整；
    - registry schema/gate 使用 synthetic projection 验证 clock/order/binary
      fields 与 immutable/mutable split；T6 不写 final `registry.json`、不在 T7
      final binary 产生前锁定 live registration；
    - treatment review input/output hashes、supervisor timing、task-batch freeze
      与 target-reveal ordering 在 report 前验证；
    - superiority lower bound、non-inferiority lower bound >=-3pp、treatment-side
      review cost、固定 48-run stop-loss denominator 与 attribution-missing
      insufficient 全有边界测试；
    - 144 个 receipt-free JCS records、frozen control 与 detached source manifest 完整；每个 payload digest 依次匹配 ledger attestation/checkpoint/mapping；
    - 默认 report 只从 execution-time governed evidence 完全离线重算且不声称 current freshness；
    - 显式 network freshness audit 输出不改 report 的 signed detached receipt；network denial/stale/wrong-report/drift 失败，publish/closure/release 要求 exact receipt 未过期；
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
    - `eval/claims/registry.json`（仅 final `registration_projection` freeze；
      T6 schema/gate 完成且 final binary build 后顺序移交）
    - `src/cli/{eval_types.rs,tests_eval.rs}`
    - `src/cli/actions/eval.rs`
    - `scripts/ci/check_public_claims.py`
    - `eval/coding-bench/README.md`
    - `docs/ARCHITECTURE.md`
    - `docs/specs/README.md`
    - `docs/specs/issue385-coding-agent-ab/{PRODUCT.md,TECH.md}`
    - `docs/specs/public-memory-benchmark/{PRODUCT.md,TECH.md}`
    - `README.md`, `README.zh-CN.md`, `CHANGELOG.md`
    - version-sync files in `tech.md`（Cargo files 从 T3A 顺序接收；只改
      package version，不改变 dependency set）
  - Done when:
    - CLI 默认 primary、显式 diagnostic、live confirm/hard caps 和 output paths；
    - live smoke 只用现有 `--condition` + `--task` +
      `--runs-per-condition 1` 精确选择单 tuple；`--task-set smoke` 的
      condition×task 笛卡尔积不能作为三-tuple live authorization；
    - dry-run 写可验证 JSON 且零外部调用；
    - docs 保持 scaffold/implementation/official evidence 状态分离；
    - T7 不修改 T1 批准的 current-contract files；实现若发现合同必须变化，
      立即停止并以 exact diff 重新执行 T1 人工 maintainer/security 审批后续作；
    - non-PASS + positive public wording 被 CI 拒绝；PASS 时任何不等于
      maintainer-approved exact wording/report link 的幅度、范围或改写也被拒绝；
    - version surfaces 同步，并从 exact final implementation head reproducibly
      build/记录 host supervisor、remem 与 agent binary hashes；supervisor
      由 OS/security-owner measurement + attestation key 建立 trust，binaries 装到
      security-owned、read-only content-addressed mount，拒绝 symlink；
    - 每次 verify/smoke/official/report/freshness invocation 都从 no-follow executing-file
      handle 自验 approval/manifest-pinned digest，并在 artifact 记录 digest +
      stable file identity；平台不能保证“验证的同一 handle 被执行”时 fail closed，
      不能依赖一次 path hash；
    - 完成上述 build 后才冻结 final `registration_projection`，绑定
      executable/profile digests、`evaluation_as_of`/virtual clock、
      condition-order seed/PRNG version/完整 tuple permutation+digest 与 bootstrap
      policy；任何 official-fixture smoke 前保持 immutable。
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
    - 独立 reviewer 检查 E2E source ordinal/call-row order、未 seed/preload、
      curator blind、scorer/code-worker OS/RPC isolation、receipt-free payload 的
      payload→ledger→checkpoint→mapping、offline report 与 detached network
      freshness receipt、clock/same-handle/order/registration/denominator/statistics/
      wording，并覆盖 monkeypatch、network denial、stale/wrong-report negatives；
    - fresh offline/focused/full commands 全绿；
    - review artifact、remote PR head 与 intended PR body 都绑定同一 exact
      head，full
      `check_pr_preflight.py`（不使用 `--fast`）前后 HEAD 不变，findings
      全部闭环；
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
    git fetch origin main
    GH931_PREFLIGHT_HEAD="$(git rev-parse HEAD)"
    test "$(gh pr view "$PR_NUMBER" --json headRefOid --jq .headRefOid)" = \
      "$GH931_PREFLIGHT_HEAD"
    gh pr view "$PR_NUMBER" --json body --jq .body > /tmp/pr-body.md
    python3 scripts/ci/check_pr_preflight.py --base origin/main \
      --pr-body-file /tmp/pr-body.md
    test "$(git rev-parse HEAD)" = "$GH931_PREFLIGHT_HEAD"
    ```

- [ ] `SP931-T9` Owner: maintainer/security/cost owner; Done when: smoke 与 full live agent/provider/auth/cost 分别获得限额授权； Verify: trusted approval 绑定 exact hashes/tuples/caps； Covers: B-015, B-020, B-028, B-030。
  - Owner: maintainer/security/cost owner
  - Dependencies: SP931-T8
  - Covers: B-015, B-020, B-028, B-030
  - File ownership（T2 closed scaffold 在 T8 后顺序移交）:
    - `eval/coding-bench/live-run-approvals.json`（仅写经 review/merge 的
      smoke/full live entries；T2 的 schema 与 validator 不在本 task 修改）
  - Done when:
    - immutable registration projection 在任何 official-fixture live smoke 前
      已锁定；maintainer 再批准每个 primary condition 一个 smoke tuple；
    - T8 对应的 implementation head 已经人工 review 并 merge 到 default
      branch；approved remem binary 从该 exact default-branch code SHA
      reproducibly build，放在 reviewed read-only content-addressed mount。每个
      command 自验 no-follow executing handle 与 approval-pinned digest；
    - smoke 的 isolation/capture/curator/cleanup/artifact 经人工复核后，再单独
      批准 144-run official matrix；
    - approval 绑定 exact code/fixture/registration、actual remem/agent binaries、
      target/extraction/enrichment/promotion/retrieval profiles、timeout、tuple
      selectors、expiry、max agent/LLM calls、max estimated cost、USD pricing
      snapshot/SKU/token ceilings/rounding/provider hard-limit policy、pinned
      evaluation clock 与 preregistered order digest；
    - 三个 smoke tuple 分别使用 approval-policy-derived、互不相交且不属于
      `issue385-v1/official-v1` 的 `matrix_namespace`，并使用不同 run ID、
      attempt ID 与 artifact root；smoke/official canonical key 都包含
      `run_phase`/`matrix_namespace`，smoke 永久不能作为 official tuple key、
      evidence source 或 denominator。所有 smoke/official records 仍写入同一个
      `refs/heads/remem-live-ledger` 并共享累计预算，禁止拆分 ledger/ref；
    - credential bytes 不进入 approval/artifact；
    - smoke/full approval entry 通过
      `eval/coding-bench/schemas/live-run-approval.schema.json`，经独立
      maintainer APPROVED review 的 PR merge 到 default branch；entry 绑定
      exact hashes/tuple selectors/有效期/caps 且不含 credential bytes；
      `approval_key` 只由 repo identity + policy digest + approval PR number
      派生，承载 key 的 blob/tree/commit 与 review/merge attestation 均不进入
      preimage；approval 另绑定 exact ledger ref/genesis、sole writer App
      installation/repository identity、signature algorithm/key ID/public-key
      digest、update-authority/integrity 两个 ruleset ID/hash、pinned TUF trust/
      signing-config digests、Rekor operator/log/API identity 与 minimum verified
      bundle/checkpoint；
    - authority broker 只从 `origin` default branch 的 registry 解析唯一 entry，
      fresh 验证 schema、approval PR head/tree、未 dismissed 的 maintainer
      review、merge/default-branch ancestry，以及 actual code/fixture/
      registration/executable/profile hashes、tuple selectors、expiry 和全部
      call/token/cost caps、policy-derived `run_phase`/`matrix_namespace` 与
      writer/two-ruleset/TUF/Rekor bindings；命令行 key/caps 只是匹配断言，任意字符串、
      missing/duplicate entry 或任何 drift 都不能构成授权且必须在
      provider/host/agent call 前失败；
    - smoke 人工复核完成后，full matrix 使用独立新 entry、新 approval key 与
      namespace；不能扩写/reuse smoke entry。
    - runner 的 negative suite 已证明 caller 自选 key、未 merge/未 APPROVED/
      过期 registry、每-call approval/rulesets/TUF/Rekor/ancestry drift、authority
      credential scope 扩大、token hard-limit unsupported/超限、跨 clone/
      `execution_id` replay、pre-target/pre-call crash、abandoned reservation、
      ledger rollback 与拆单超额均在 provider/host/agent call 前失败。
  - Verify:
    ```bash
    git fetch origin main
    git diff --exit-code origin/main -- \
      eval/coding-bench/schemas/live-run-approval.schema.json \
      eval/coding-bench/live-run-approvals.json
    test -x "$GH931_APPROVED_REMEM_BIN"
    test ! -L "$GH931_APPROVED_REMEM_BIN"
    test -x /usr/local/libexec/remem-bench-supervisor
    test ! -L /usr/local/libexec/remem-bench-supervisor
    gh931_remem() {
      /usr/local/libexec/remem-bench-supervisor run -- \
        "$GH931_APPROVED_REMEM_BIN" "$@"
    }
    git merge-base --is-ancestor "$GH931_APPROVED_CODE_SHA" origin/main
    python3 eval/coding-bench/validate_schemas.py
    cargo test eval::coding_bench::approval
    cargo test eval::coding_bench::tests::smoke_namespace_isolation
    cargo test eval::coding_bench::tests::ledger_writer_authentication
    cargo test eval::coding_bench::checkpoint
    cargo test eval::coding_bench::tests::rekor_rollback_rejected
    cargo test eval::coding_bench::tests::cumulative_usage_ledger
    cargo test eval::coding_bench::tests::pre_call_reservation_crash
    cargo test eval::coding_bench::tests::started_attempt_recovery
    cargo test eval::coding_bench::tests::ledger_protection_anchor
    cargo test eval::coding_bench::tests::pricing_reservation
    cargo test eval::coding_bench::tests::dispatch_token_ceilings
    cargo test eval::coding_bench::tests::per_call_authority_revalidation
    cargo test eval::coding_bench::tests::pre_target_attempt_recovery
    cargo test eval::coding_bench::tests::terminal_ledger_attestation
    gh931_remem bench coding --suite issue385-v1 \
      --condition no_memory --task ticket-key-memory-convention \
      --runs-per-condition 1 --approval-key "$GH931_SMOKE_APPROVAL_KEY" \
      --verify-live-approval-only \
      --json-out /tmp/gh931-smoke-no-memory-validation.json
    jq -e --arg ns "$GH931_SMOKE_NO_MEMORY_NAMESPACE" '
      .authorized == true and .self_verified == true and
      .run_phase == "smoke" and .planned_runs == 1 and
      (.planned_tuples | length) == 1 and
      .planned_tuples[0].matrix_namespace == $ns and
      .planned_tuples[0].run_index == 1 and
      .planned_tuples[0].condition == "no_memory" and
      .planned_tuples[0].task_id == "ticket-key-memory-convention" and
      .provider_calls == 0 and .agent_spawns == 0
    ' /tmp/gh931-smoke-no-memory-validation.json
    gh931_remem bench coding --suite issue385-v1 \
      --condition no_memory --task ticket-key-memory-convention \
      --runs-per-condition 1 \
      --approval-key "$GH931_SMOKE_APPROVAL_KEY" --confirm-live-run \
      --max-agent-calls "$GH931_SMOKE_MAX_AGENT_CALLS" \
      --max-llm-calls "$GH931_SMOKE_MAX_LLM_CALLS" \
      --max-estimated-cost-usd "$GH931_SMOKE_MAX_ESTIMATED_COST_USD" \
      --json-out /tmp/gh931-smoke-no-memory.json
    gh931_remem bench coding --suite issue385-v1 \
      --condition curated_file_budgeted --task slug-normalizer-contract \
      --runs-per-condition 1 --approval-key "$GH931_SMOKE_APPROVAL_KEY" \
      --verify-live-approval-only \
      --json-out /tmp/gh931-smoke-curated-validation.json
    jq -e --arg ns "$GH931_SMOKE_CURATED_NAMESPACE" '
      .authorized == true and .self_verified == true and
      .run_phase == "smoke" and .planned_runs == 1 and
      (.planned_tuples | length) == 1 and
      .planned_tuples[0].matrix_namespace == $ns and
      .planned_tuples[0].run_index == 1 and
      .planned_tuples[0].condition == "curated_file_budgeted" and
      .planned_tuples[0].task_id == "slug-normalizer-contract" and
      .provider_calls == 0 and .agent_spawns == 0
    ' /tmp/gh931-smoke-curated-validation.json
    gh931_remem bench coding --suite issue385-v1 \
      --condition curated_file_budgeted --task slug-normalizer-contract \
      --runs-per-condition 1 \
      --approval-key "$GH931_SMOKE_APPROVAL_KEY" --confirm-live-run \
      --max-agent-calls "$GH931_SMOKE_MAX_AGENT_CALLS" \
      --max-llm-calls "$GH931_SMOKE_MAX_LLM_CALLS" \
      --max-estimated-cost-usd "$GH931_SMOKE_MAX_ESTIMATED_COST_USD" \
      --json-out /tmp/gh931-smoke-curated.json
    gh931_remem bench coding --suite issue385-v1 \
      --condition remem_e2e --task workstream-title-continuity \
      --runs-per-condition 1 --approval-key "$GH931_SMOKE_APPROVAL_KEY" \
      --verify-live-approval-only \
      --json-out /tmp/gh931-smoke-remem-e2e-validation.json
    jq -e --arg ns "$GH931_SMOKE_REMEM_E2E_NAMESPACE" '
      .authorized == true and .self_verified == true and
      .run_phase == "smoke" and .planned_runs == 1 and
      (.planned_tuples | length) == 1 and
      .planned_tuples[0].matrix_namespace == $ns and
      .planned_tuples[0].run_index == 1 and
      .planned_tuples[0].condition == "remem_e2e" and
      .planned_tuples[0].task_id == "workstream-title-continuity" and
      .provider_calls == 0 and .agent_spawns == 0
    ' /tmp/gh931-smoke-remem-e2e-validation.json
    gh931_remem bench coding --suite issue385-v1 \
      --condition remem_e2e --task workstream-title-continuity \
      --runs-per-condition 1 \
      --approval-key "$GH931_SMOKE_APPROVAL_KEY" --confirm-live-run \
      --max-agent-calls "$GH931_SMOKE_MAX_AGENT_CALLS" \
      --max-llm-calls "$GH931_SMOKE_MAX_LLM_CALLS" \
      --max-estimated-cost-usd "$GH931_SMOKE_MAX_ESTIMATED_COST_USD" \
      --json-out /tmp/gh931-smoke-remem-e2e.json
    gh931_remem bench coding --suite issue385-v1 \
      --approval-key "$GH931_FULL_APPROVAL_KEY" \
      --verify-live-approval-only \
      --json-out /tmp/gh931-full-approval-validation.json
    jq -e '.authorized == true and .self_verified == true and
      .run_phase == "official" and
      .matrix_namespace == "issue385-v1/official-v1" and
      (.planned_tuples | length) == 144 and .provider_calls == 0 and
      .agent_spawns == 0' /tmp/gh931-full-approval-validation.json
    ```

- [ ] `SP931-T10` Owner: authorized benchmark operator; Done when: 144 个 primary tuple 都有 immutable verified artifacts； Verify: final verifier 报告 `valid_primary_runs == 144`； Covers: B-004-B-006, B-014-B-024。
  - Owner: authorized benchmark operator
  - Dependencies: SP931-T9
  - Covers: B-004, B-005, B-006, B-014, B-015, B-016, B-017, B-018,
    B-019, B-020, B-021, B-022, B-023, B-024
  - File ownership（T6 implementation 完成后顺序移交）:
    - `eval/coding-bench/evidence/flagship-e2e-v1/frozen-control-content.jsonl`
    - `eval/coding-bench/evidence/flagship-e2e-v1/run-records.jsonl`
    - `eval/coding-bench/evidence/flagship-e2e-v1/source-manifest.json`
  - Done when:
    - 16 tasks × 3 primary × 3 runs 的每个 tuple 都有 verified artifact；
    - failed outcomes 留在分母，attempt history 保留；
    - no secret/HOME/hidden/cross-run leak；
    - run 使用 locked registration projection、actual approved executable/profile
      digests 和 protected remote reservation ledger；
    - official run、report 与 freshness audit 分别从 read-only CAS mount 的 no-follow executing
      handle 自验 approval/manifest digest；一次 path hash 不能复用于后续 command；
    - 每个 tuple 的 receipt-free JCS payload digest 先由 supervisor seal 到
      authoritative ledger，再由 source manifest detached 绑定 attestation/checkpoint；
    - report 从 artifacts + execution receipts 离线重建；current freshness 只来自另行签名 receipt。
    - scanner-passed sanitized attempts/runs 写入 committed
      `run-records.jsonl`，`source-manifest.json` 绑定全部 attempt、matrix、
      scanner、code/fixture/registry、detached terminal/checkpoint receipts 与 report-input
      hashes；frozen control content bundle 绑定 exact sanitized bytes；
      raw/private/secret evidence 不进入 git。
  - Verify:
    ```bash
    test -x "$GH931_APPROVED_REMEM_BIN"
    test ! -L "$GH931_APPROVED_REMEM_BIN"
    test -x /usr/local/libexec/remem-bench-supervisor
    test ! -L /usr/local/libexec/remem-bench-supervisor
    gh931_remem() {
      /usr/local/libexec/remem-bench-supervisor run -- \
        "$GH931_APPROVED_REMEM_BIN" "$@"
    }
    gh931_remem bench coding --suite issue385-v1 --matrix primary \
      --approval-key "$GH931_FULL_APPROVAL_KEY" --confirm-live-run \
      --max-agent-calls "$GH931_FULL_MAX_AGENT_CALLS" \
      --max-llm-calls "$GH931_FULL_MAX_LLM_CALLS" \
      --max-estimated-cost-usd "$GH931_FULL_MAX_ESTIMATED_COST_USD" \
      --json-out /tmp/gh931-primary.json
    jq -e '.self_verified == true and .valid_primary_runs == 144' \
      /tmp/gh931-primary.json
    gh931_remem bench report \
      --root eval/coding-bench/evidence/flagship-e2e-v1 \
      --json-out /tmp/gh931-recomputed-report.json \
      --markdown-out /tmp/gh931-recomputed-report.md
    jq -e '.self_verified == true' /tmp/gh931-recomputed-report.json
    ```

- [ ] `SP931-T11` Owner: report-claim lane + maintainer; Done when: paired report、cost、stop-loss、wording、review/merge/issue/release 决策完整； Verify: claim/public checks、fresh exact-head full preflight 与人工 final review 通过； Covers: B-023-B-030。
  - Owner: report-claim lane + maintainer
  - Dependencies: SP931-T10
  - Covers: B-023, B-024, B-025, B-026, B-027, B-028, B-029, B-030
  - File ownership（T10 evidence seal 后顺序移交）:
    - `eval/coding-bench/reports/{flagship-e2e-v1.json,flagship-e2e-v1.md}`
    - `eval/claims/registry.json`（仅 `result_bindings`）
    - `README.md`, `README.zh-CN.md`, `CHANGELOG.md`（仅 PASS + exact wording approval）
  - Done when:
    - immutable JSON/Markdown report、receipt-free run bundle、detached source manifest/receipts 与 registry 一致，默认 verification 完全离线；
    - paired CI、maintenance cost、memory-harm stop-loss 可复算；
    - registration projection 保持 byte-identical；只更新 result bindings；
    - PASS/FAIL/INSUFFICIENT 都保留 report；只有 PASS + exact-report 未过期
      signed freshness receipt 才可批准 public wording/closure/release，receipt
      生成不得重写 report；
    - independent final review、CI、fresh exact-head full preflight 与 closure
      audit 完成；
    - maintainer 单独决定 merge、是否关闭 #931 与 release。
  - Verify:
    ```bash
    python3 eval/claims/claim_gate.py check
    python3 scripts/ci/check_public_claims.py
    git fetch origin main
    GH931_FINAL_PREFLIGHT_HEAD="$(git rev-parse HEAD)"
    test "$(gh pr view "$PR_NUMBER" --json headRefOid --jq .headRefOid)" = \
      "$GH931_FINAL_PREFLIGHT_HEAD"
    gh pr view "$PR_NUMBER" --json body --jq .body > /tmp/gh931-final-pr-body.md
    python3 scripts/ci/check_pr_preflight.py --base origin/main \
      --pr-body-file /tmp/gh931-final-pr-body.md
    test "$(git rev-parse HEAD)" = "$GH931_FINAL_PREFLIGHT_HEAD"
    ```

## 并行拆分

- SP931-T1 的 current-contract exact-diff maintainer/security 人工批准与 merge
  是本计划的 project-specific 安全前置条件；root historical packet 自身、
  label、环境变量或机器 output 都不能代替。T1 完成前不得启动 T2-T11。
- SP931-T2 在 T1 后先完成 closed-ID migration，并把 `artifact.rs` 移交 T3、
  `condition.rs` 移交 T4、runner/tests 移交 T6；T3 随后提交可编译的 parent
  declarations，T3A 再顺序接收 parent 并原子声明/创建 checkpoint module。
- SP931-T5 必须等待 T2/T3 完成，再独占 curator contract 文件；T4 等待 T5
  后独占 condition/failure 与完整 production-clock paths，测试只写其模块内。
- T6 等待 T3A/T4/T5 后独占 runner、`tests.rs`、report/claim integration，只用
  synthetic registration 测试，不写 final `registry.json`。
- T7 串行处理共享 CLI/docs/version/public-claim 高风险文件，完成 exact
  final binary build 后接收 `registry.json` ownership 并冻结 registration；
  T8/T9 均不得早于该 freeze；T7 不写 T1 已批准的 current-contract files，
  必要合同变更必须重新执行 T1 后才能继续。
- T2 对 `live-run-approvals.json` 的 closed scaffold ownership 在 T8 后顺序
  移交 T9；T9 是 live entry 的唯一 writer，不修改 T2 已批准的 schema/
  validator。schema 或 registry shape 变化必须回到原 owner、重新 review，
  不能与 live entry PR 并行。
- live T10 不得与任何改变 code/fixture/registration/executable/profile hash
  的 lane 并行；T10 seal evidence 后才把 result-path write ownership 移交 T11。
- 每个 agent 只能写任务列明的文件；新增路径先更新 tech manifest 并使 T1
  exact-diff approval 失效、重新走人工审批。

## Handoff Notes

- `origin/main@441d7da1325ab3b9eea39c10753d37131c062dbc`：
  PR #936 scaffold 已合并；PR #965 已退休旧 issue/PR workflow，root
  `specs/GH*` packet 只保留 historical planning evidence。
- 当前 Rust dry-run 仍输出旧
  `no_memory/remem/curated_file` 144 plan；这只是 legacy runner truth，不是
  primary flagship evidence。
- 当前没有 readiness label 或 packet state 机械授权/阻止 GH-931 工作；current
  `docs/specs/GH931/` contract 已在本 spec PR 更新，但仍缺 T1 exact-diff
  maintainer/security 人工批准，且 security/live/final review/merge/release
  gates 相互独立。
- 本 packet 没有执行 live benchmark，没有使用 provider/auth，没有生成
  flagship report，也没有授权 public claim、merge、close 或 release。
- Product invariants `B-001`…`B-030` 全部由 T2-T11（含 T3A）覆盖。
