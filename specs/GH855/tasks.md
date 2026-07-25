# Task Plan

## Linked Issue

GH-855

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`

## 当前阻塞与前置门禁

截至 2026-07-19，GH-855 的可信 GitHub evidence 仍为 `ready_to_spec`，没有 maintainer
`spec_approval`；issue 引用的
`docs/research/agent-memory-optimization-research-2026-07.md` 也不在当前仓库中。PR #866 是
引用 GH-855 的现有 spec-only PR，不能把它或本 task plan 的存在解释为 implementation
readiness。

开始或勾选任何 implementation task 前，必须全部满足：

- maintainer 提供 issue 所引用研究报告的 immutable revision，或明确批准等价一手证据；
- maintainer 留下明确的 `spec_approval` 与 security-review scope；
- GH-855 的实际 readiness label/state 变为 `ready_to_implement`；
- 重新生成 fresh issue/duplicate evidence，并确认没有竞争 implementation PR；
- implement route gate 返回允许 implementation，而不是 `needs_human` 或 `blocked`。

```bash
python3 checks/github_issue_evidence.py --github-repo majiayu000/remem --issue 855 --json > /tmp/gh855-issue-evidence.json
python3 checks/github_duplicate_evidence.py --github-repo majiayu000/remem --issue 855 --json > /tmp/gh855-duplicate-evidence.json
python3 checks/route_gate.py --repo . --route implement --issue 855 \
  --evidence /tmp/gh855-issue-evidence.json \
  --duplicate-evidence /tmp/gh855-duplicate-evidence.json --json
```

当前授权仅限 spec planning。以下 checkbox 不得在上述门禁满足前开始或标记完成。

## 实现任务

- [ ] `SP855-T1` — 统一 redaction/verdict 与 candidate source-taint — Owner: poisoning core agent; Dependencies: human gates; Done when: 见下; Verify: 见下
  - Dependencies: 所有前置门禁通过。
  - Files: `src/memory/poisoning.rs`、`src/memory_candidate.rs`、
    `src/memory_candidate/tests/poisoning.rs`、`src/observation_extract.rs` 及其直接
    parse/response/tests 模块。
  - Covers: `B-001`–`B-005`, `B-014`, `B-019`。
  - Done when: source/generated surface 使用同一版本化、确定性 verdict；所有文本先 redaction；
    source-only laundering、缺失/跨项目 evidence、scan/persistence error 均 fail closed；规范
    capture 保留且 active memory/auto-promote 为零；adapter evidence contract 对当前
    Claude/Codex source 完整。
  - Verify:
    - `cargo test memory::poisoning -- --nocapture`
    - `cargo test memory_candidate::tests::poisoning -- --nocapture`
    - `cargo test observation_extract -- --nocapture`

- [ ] `SP855-T2` — v072 schema、summary quarantine 与 durable side-effect state — Owner: migration/rollup agent; Dependencies: `SP855-T1`; Done when: 见下; Verify: 见下
  - Dependencies: `SP855-T1`；implementation 开始时重新确认 next-free migration，若 v072 已占用则
    顺延并同步本 packet 与所有 version/schema surfaces。
  - Files: next-free `src/migrations/v0*_session_summary_poisoning.sql`、`src/migrate/**` 中对应
    manifest/invariant/tests、`src/session_rollup/**`、`src/db/summarize/session/**`、
    `src/summarize/summary_job/**`。
  - Covers: `B-006`, `B-007`, `B-012`, `B-015`, `B-016`, `B-021`, `B-024`, `B-025`。
  - Done when: 只有 migration-origin pre-v072 rows 可为 `legacy_unscanned`；runtime writer 必须
    source-bound 且显式写 safe/quarantined；pending segment bundle 脱敏、有界、默认不可见；
    quarantine 前无 topic/candidate/native side effect；worker retry 不重调 LLM，completion marker
    仅在全部 checkpoint 成功后 CAS 写入；旧 binary writer 显式失败而不静默降级。
  - Verify:
    - `cargo test session_rollup::tests::poisoning -- --nocapture`
    - `cargo test db::summarize::session -- --nocapture`
    - migration upgrade/schema drift/rollback tests from a v071 fixture
    - `cargo test --no-default-features validate_schema_invariants_is_clean_after_current_migrations -- --nocapture`

- [ ] `SP855-T3` — shared summary eligibility、retrieval 与所有 model-visible sinks — Owner: context/trace agent; Dependencies: `SP855-T2`; Done when: 见下; Verify: 见下
  - Dependencies: `SP855-T2`。
  - Files: `src/context/**`、`src/db/query/summaries.rs`、`src/git_trace.rs`、
    `src/git_trace/tests.rs`、`src/mcp/server/commit_tools.rs`、summary-consuming
    `src/observation_extract.rs`、`src/summarize/**`、`src/user_context/**` 调用点。
  - Covers: `B-008`, `B-015`, `B-022`, `B-023`, `B-026`。
  - Done when: raw row decode 后、任何正文 dedup/query/ranking 前完成 canonical generated fields
    与 exact source combined verdict；poisoned/error row 对 retrieval 的影响等同于 row 不存在但
    error 可见；native memory、context、observation/summarize/user-context、git trace 与 MCP commit
    tools 均只能消费 `EligibleSessionSummary`；跨 project/session/ID 与 scanner/schema/audit error
    不返回正文。
  - Verify:
    - `cargo test context::tests::render_poisoning -- --nocapture`
    - `cargo test context::tests::sessions -- --nocapture`
    - `cargo test context::tests::retrieval -- --nocapture`
    - `cargo test git_trace::tests -- --nocapture`
    - `cargo test mcp::server::tests -- --nocapture`
    - closure audit: `rg -n 'session_summaries' src`，逐个 model-visible read 证明经过 eligibility gate

- [ ] `SP855-T4` — governance acknowledgement 与 checkpointed release — Owner: governance agent; Dependencies: `SP855-T2`, `SP855-T3`; Done when: 见下; Verify: 见下
  - Dependencies: `SP855-T2`, `SP855-T3`。
  - Files: `src/memory/governance.rs` 及 tests、`src/cli/**governance**`、
    `src/mcp/types.rs`、`src/mcp/server/write_tools.rs`、summary release worker/callers。
  - Covers: `B-009`–`B-012`, `B-014`, `B-024`–`B-026`。
  - Done when: optional target kind 保持旧调用 memory-only；session summary 只允许 explicit-ID
    `acknowledge-pattern`；source-bound ack 重载 exact source + canonical generated surface，
    generated-only 只接受 immutable migration origin；错 ID/version/project/state/evidence、
    missing reason/actor/confirm 与 CAS race 全部原子拒绝；成功 ack 只 enqueue retry，side effects
    由 checkpoint 至多释放一次。
  - Verify:
    - `cargo test cli::tests_governance -- --nocapture`
    - `cargo test memory::governance -- --nocapture`
    - `cargo test mcp::server::tests -- --nocapture`
    - fault-injection retry tests for every checkpoint and final completion CAS

- [ ] `SP855-T5` — doctor/status/API poisoning observability — Owner: observability agent; Dependencies: `SP855-T2`, `SP855-T4`; Done when: 见下; Verify: 见下
  - Dependencies: `SP855-T2`, `SP855-T4`。
  - Files: `src/db/query/stats.rs` 及 tests、`src/doctor/memory_poisoning.rs`、
    `src/doctor/tests.rs`、CLI status types/render/tests、`src/api/handlers/status.rs` 及 API tests。
  - Covers: `B-013`, `B-014`。
  - Done when: candidate/summary quarantine、legacy、source/generated、context block 与 pattern
    version 使用同一 stats contract；fresh/stale/error 状态明确；query/schema error 不显示 0/健康；
    human/JSON 输出只含 allowlisted metadata，不含 payload/secret。
  - Verify:
    - `cargo test doctor -- --nocapture`
    - `cargo test db::query::stats -- --nocapture`
    - focused CLI/API status tests for fresh, stale, expired-stale, and query-error cases

- [ ] `SP855-T6` — deterministic capture E2E 与 adversarial-policy v2 — Owner: eval agent; Dependencies: `SP855-T1`–`SP855-T5`; Done when: 见下; Verify: 见下
  - Dependencies: `SP855-T1`–`SP855-T5`。
  - Files: `src/eval/capture_poisoning/**`、`src/eval/memory_bench/**`、
    `eval/public/memory/suites/adversarial-policy/suite.json`、对应 v2 manifests/reports/artifacts
    与 public baseline。
  - Covers: `B-001`–`B-004`, `B-006`, `B-014`, `B-017`, `B-018`。
  - Done when: fake extractor/rollup 从真实 capture/schema 边界离线驱动两条生产路径；英中
    override、authority、opaque、secret-mixed 与 laundering case 的 active/injected poison 为零；
    benign quote 可见地进入 expected review；artifact verifier 复现且提交 artifact 不含 secret/
    raw payload。
  - Verify:
    - `cargo run -- eval-capture-poisoning --json-out /tmp/remem-capture-poisoning.json`
    - `cargo run -- bench memory --suite adversarial-policy --condition remem_default --root eval/public --artifact-prefix memory/artifacts/adversarial-policy-v2 --json-out eval/public/memory/reports/adversarial-policy-v2.json`
    - `cargo run -- bench verify --root eval/public --json-out /tmp/remem-bench-verify.json`

- [ ] `SP855-T7` — current contracts、用户文档与 version surfaces — Owner: integration agent; Dependencies: `SP855-T1`–`SP855-T6`; Done when: 见下; Verify: 见下
  - Dependencies: `SP855-T1`–`SP855-T6`。
  - Files: `docs/specs/memory-poisoning-defense/PRODUCT.md`、
    `docs/specs/memory-poisoning-defense/TECH.md`、`docs/specs/README.md`、`README.md`、
    `docs/ARCHITECTURE.md`、`CHANGELOG.md` 与 version-sync contract 列出的 package/plugin/runtime
    surfaces。
  - Covers: `B-019`, `B-020`, `B-021` 及发布/兼容性说明。
  - Done when: current contract 与实现 truth 同步；README/architecture 只声明已验证行为；migration
    与 package/plugin/npm/server versions 一致；release note 明确 conservative quarantine 与
    forward-only/old-writer incompatibility，不虚构缺失研究证据。
  - Verify:
    - `python3 scripts/ci/check_plugin_version_sync.py`
    - `python3 scripts/ci/check_version_bump.py <base-sha> HEAD`
    - `node --test plugins/remem/scripts/remem-runtime.test.js plugins/remem/apps/remem/server.test.js npm/remem/scripts/install.test.js`

## 并行拆分

默认单 agent 串行执行 `SP855-T1 → T2 → T3 → T4 → T5 → T6 → T7`。这是 security、
migration、shared-reader 与 acknowledgement contract 的跨模块变更；多个任务共享
`src/observation_extract.rs`、summary worker、context/governance types 与 schema-generated surfaces，
未形成稳定前不启动并行 writable lane。

若后续 maintainer 明确批准并行，只允许在依赖任务 head 与 focused tests 已固定后，按任务列出的
files 建立 disjoint ownership；发现 shared file 时立即串行化并更新 ownership。full-suite、migration
fixture、eval artifact 与 version sync 由单一 integration owner 执行，禁止同一 worktree 并发 build/test。

## 验证

- Issue acceptance mapping:
  - “tool output/transcript 完整 capture 后不产生 active 指令性 memory”由
    `SP855-T1`–`SP855-T6` 的 source-taint、summary quarantine、reader gate 与 capture E2E
    evidence 共同关闭。
  - “adversarial-policy eval 扩展 capture 路径”由 `SP855-T6` 的 v2 suite、production-pipeline
    harness、report/artifact verifier 关闭。

- [ ] `SP855-T8` — full verification 与 security/merge handoff — Owner: integration agent; Dependencies: `SP855-T1`–`SP855-T7`; Done when: 见下; Verify: 见下
  - Dependencies: `SP855-T1`–`SP855-T7`；implementation route gate 仍为 allowed。
  - Covers: `B-001`–`B-026`。
  - Done when: Product-to-Test mapping 每个 invariant 都有 fresh evidence；focused tests、migration/
    schema drift、offline eval、format/build/lint/full tests、JS tests 与 PR preflight 全部通过；没有
    ignored failure、弱化 assertion、silent fallback 或未解释 changed file；security reviewer 明确
    审核 ack/CAS、redaction、migration、pre-retrieval gate、git/MCP identity 与 public artifacts。
  - Verify:
    - `git diff --check`
    - `python3 checks/check_workflow.py --repo .`
    - `python3 checks/check_workflow.py --repo . --spec-dir specs/GH855`
    - `cargo fmt --check`
    - `cargo check`
    - `cargo clippy --all-targets -- -D warnings`
    - `cargo test`
    - `cargo run -- eval-extraction --json --check-baseline`
    - `cargo run -- eval-gates --json-out /tmp/remem-eval-gates.json`
    - `python3 scripts/ci/check_pr_preflight.py --base origin/main --pr-body-file /tmp/pr-body.md`

## Handoff Notes

- This task plan is intentionally draft/gated. It satisfies packet completeness without granting
  `spec_approval`, `ready_to_implement`, security approval, final review, merge, or release.
- PR #866 remains spec-only and must use `Refs #855` and `Refs #849`; it must not close either issue.
- A future final implementation PR may use `Closes #855` only after every acceptance criterion and
  `SP855-T8` are complete. GH-849 remains open until its umbrella acceptance/child-issue semantics are met.
- Current main includes migrations v070 and v071, so the planned next-free migration is v072. Recheck at
  implementation start and update the packet if another migration lands first.
- The missing research report/equivalent primary evidence is a human decision gate. Do not replace it with
  issue prose, an agent-authored summary, or an unversioned local file.
- Locale is `zh-CN`; stable IDs, paths, commands, states, routes, and JSON keys remain English.
