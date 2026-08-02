# Historical Task Plan Snapshot：CurrentTruth 统一读取契约

## Linked Issue

GH-933

PR #939 已于 2026-07-26 合并，并把 public Truth v1 发布到 0.6.26/0.6.27。
它是 Phase A baseline，不是可继续写入的 lane，也没有完成 issue 中的
Observation adapter、worktree/task selector、Context Bundle 或 writer decision。
PR #963 只更新历史 planning packet、规范性 `docs/specs/GH933/`、索引与已发布
Truth v1 的 changelog 事实；所有 implementation checkbox 保持未完成。本文件
不是 active workflow、execution prerequisite 或 implementation authorization；
未来执行必须从当时的 live issue、`origin/main` 和规范性 current contract
重新建立计划。

## Current Evidence

- Public crate path 是 `remem::truth`；package 名 `remem-ai` 不能用作 Rust path。
- v1 已公开，因此 source-breaking v2 必须使用 0.7.0 或实现时下一个 breaking
  SemVer boundary，并提供 README migration。
- 当前 adapter 存在外部 tool evidence 提权、Observation quarantine 状态缺失、
  relation 全表读取、suppression owner 未应用和 effective epoch 不可审计等 gap。
- user claim edit 有可重建版本链；非 governance/versioned 的原地 mutation
  不可重建，而 memory governance 有 timestamped audit/ledger，不能混为一类。
- `src/truth/tests.rs` 已有 679 行；新增测试前必须拆分。
- Phase B/C 仍是后续工作，GH-933 在最终 closure audit 前保持开放。

## Phase A v2 Tasks

- [ ] `GH933-A1` — Baseline inventory and branch setup.
  - Owner: coordinator/read-only reviewer.
  - Start from live `origin/main`;确认 #939 只作 merged baseline，避免在旧 branch
    上继续提交。
  - 记录 public v1 serde bytes/hash、真实 release/version metadata、migration/
    index/truth source fingerprints。
  - 全量核对 `CT-001`–`CT-012`、`CT-015`，不只挑一个样本。
  - Verify: `cargo test truth -- --nocapture`, exact base/head SHA, inventory notes.

- [ ] `GH933-A2` — Public v2 DTO and test split.
  - Owner files: `src/truth.rs`, `src/truth/types.rs`, `src/truth/tests.rs`,
    `src/truth/tests/**`, `tests/truth_public_api.rs`.
  - 先拆分 679-line test file，再实现 `TruthScope`、`SubjectIdentity`、
      `EvidenceIntegrity`、Observation evidence fields、effective
    `reference_epoch`、`ProjectionReplayability`、exact selector 与 stable
    serde/order；manual scalar ref 规范为 synthetic singleton path `0`。
  - `projection_version=2`；external integration test 必须编译
    `use remem::truth::{...}`。
  - Verify: DTO golden、public API test、line-count check、`cargo fmt --check`,
    `cargo check`.

- [ ] `GH933-A2R` — Durable route/lifecycle ledgers and route-writer cutover.
  - Owner files: next `src/migrations/vNNN_current_truth_history_ledgers.sql`,
    `src/migrate/run.rs` + focused backfill helper/tests,
    `src/memory/{store/write,operation,lifecycle}.rs`,
    `src/memory/service/{types,save}.rs` and API/MCP save request adapters,
    `src/memory_candidate/apply.rs`, `src/cli/actions/{import,markdown_archive}.rs`,
    `src/pack_import/active_import.rs`, `src/memory/scope_cleanup/mutate.rs`,
    affected eval/test fixtures, request-map/route guard/schema tests.
  - Add append-only route and lifecycle versions with contiguous memory-local
    version/predecessor chains, complete state snapshots（route 必含
    `memory_type` 与 raw nullable `topic_key`，NULL/empty 不合并）, coverage metadata,
    memory/self `ON DELETE RESTRICT`, and no FK/cascade to deletable `events`.
    Audit IDs are copied diagnostics only. 两 ledger 保存 `source_fingerprint
    TEXT NOT NULL CHECK(length(source_fingerprint)=64 AND source_fingerprint
    NOT GLOB '*[^0-9a-f]*')`，并有
    `(memory_id,source_kind,source_fingerprint)` unique，无 NULL bypass。
    同 migration 建 append-only request intent/result bindings/final commit seal；
    request ID 必须 executable DDL nonblank，request/result fingerprint 都用
    TEXT NOT NULL + exact lowercase-64hex CHECK，deferred seal FK 禁止 pending commit。
    Add the TECH-named route indexes plus
    lifecycle `(memory_id,effective_at_epoch,id)` and coverage/operation indexes.
  - In one foreground migration transaction, copy only exhaustive durable proof.
    Legacy save/Markdown changes and pruned events make unproved rows
    migration-time `forward_only`; never infer absence or earlier eligibility.
    Before old-table absence, snapshot/drop every external dependent trigger,
    then recreate it byte-exact；all preexisting memory-owned UPDATE effects stay absent
    until terminal C byte equality. Materialize A→B→C only when every transition is
    proved: seal one deterministic baseline request, then use and seal a distinct
    deterministic request for each successor before the next step. Validate counts,
    terminal snapshots/chains and reported incomplete counts before apply.
    Every internal/API request identity requires TEXT/no-NUL；route successor 必须
    NULL-safe real change，referenced API mutation row immutable；every integer-
    domain ID/version/ordinal/epoch/floor requires INTEGER storage。
  - 六类 INSERT 在 mutation 前取得/derive 稳定 request/operation ID，canonical
    entrypoint 先查 sealed writer+ID/request hash，miss append immutable intent；
    memory INSERT 带 writer/request/result-ordinal origin tuple，Route trigger 用
    strict request hash+ordinal+exact typed NEW 立即建 v1，不等待 final response；
    全部 downstream IDs/response 完成后 append mappings，再 append final seal。
    committed response-loss/concurrent duplicate 从 cross-memory mapping 返回
    exact winner，禁止使用 post-insert memory/operation row ID 作 retry identity。
    Existing
    save upsert, Markdown restore/update and scope cleanup use the canonical
    route-transition service to stage the exact next route/identity snapshot
    only when NULL-safe OLD/NEW tuple（含 type/raw key）changes; same-value assignments pass.
    normal-save selector 必须同 type，只实现 reachable raw-key transition；
    type transition 仅 stable-`source_id` Markdown path。Validated Markdown
    project→global uses `source_kind=markdown_import`.
    Scope cleanup also writes its same-status
    lifecycle version and event mirror in the transaction. Reject wrong/missing stages and direct bypasses; rollback
    all pieces together. Lifecycle INSERT trigger writes its baseline.
  - Implement TECH's canonical binary framing and every governed writer
    discriminator。string 默认 exact raw UTF-8，不做 generic CRLF/outer-trim；
    只有 writer 实际 persisted canonical form 才规范化。save request hash 枚举
    `SaveMemoryRequest` 全部 text/title/project/session/host/topic/type/files/scope/
    reference+created epoch/branch/local path+toggle/claim source/ack pattern 字段、
    Option presence 与 derived values；files 保留 order/duplicate。result hash 必含
    exact final response、memory/operation/route/lifecycle/claim/ack/local-copy/
    next-step outputs。same identity 新 content 是不同请求。Markdown 用 stable
    source binding 或 `export-version+canonical archive-relative path+synthesized
    topic` no-source identity 与 post-render semantic form，只忽略 importer-owned
    metadata rewrite，byte-preserved fields 仍 exact。
    Exact retry returns mapped response/result IDs；same ID/different payload
    conflicts；generated memory/ledger/operation/audit IDs are outputs。
  - Verify indexed `EXPLAIN` plans, rollback/idempotence, partial/forward-only
    backfill, pre-floor global failure, A→B→C intermediate discovery, all six
    inserts/three updates, no-op/change/same-second guards, terminal drift,
    normal-save same-type raw-key before/equal/after，stable-source-ID Markdown
    project→global+type/key before/equal/after plus atomic rollback and missing-
    predecessor/legacy-gap errors；strict fingerprint DDL/NULL/uppercase/nonhex/
    blank-request-ID negative、unsealed deferred-FK failure、six-insert intent→
    immediate-v1→complete-result-seal mapping、full-response/full-content save、
    local-copy/claim/ack/files-order/raw-CRLF collision negatives、metadata-rewrite Markdown，以及
    every governed writer's pre-commit crash、committed-response-loss exact retry、
    different-payload conflict、concurrent winner and event-cleanup independence.

- [ ] `GH933-A3` — Adapter and resolver hardening.
  - Owner files: `src/truth/adapter.rs`, `src/truth/lifecycle.rs`,
    `src/truth/projection.rs` plus A2-owned tests after A2/A2R stop writing；
    `src/memory/governance.rs`、`src/memory/scope_cleanup/{mutate,plan}.rs` and
    focused API/event-cleanup lifecycle tests after A2R handoff；
    `src/memory/poisoning.rs` 仅暴露 pure classifier；`src/db/capture.rs` 只允许
    duplicate `(host_id, session_id, event_id)` row timestamp no-op 与
    content-boundary/preview helper。
  - Implement owner/scope/type identity、NULL/exact-empty singleton、nonempty
    topic byte identity、canonical repo
    owner/target Project inclusion、stale non-repo placement exclusion、Owner
    memory+user-claim union、global/legacy fallback、Project/Owner branch
    semantics，以及 user-claim-only compatibility wrapper；wrapper 仅 bounded
    读取 applicable `user_claim`/`pattern` suppressions 与显式 memory ref。
    explicit history 从 A2R 的 indexed route-state candidates 加完整 chain
    重建 owner/target/scope/type/raw nullable topic key；candidate/result 先对
    initial route state 校验一次；后续 operation/Project/Owner membership 与
    emitted SubjectIdentity 使用 operation/cutoff state，但不得拿 immutable
    candidate identity 再与 cutoff/current state exact-match。equality 用 new state。normalized memory scope 是 per-version
    route state；gap/fork/coverage/terminal drift 返回 routing integrity error，
    但完整 validated scope transition 不报错。
  - Implement total user source-kind/ref grammar；candidate-derived claim 必须
    exact-match authoritative candidate/result copied fields、nested provenance
    与 preserved edit fields；验证 owner/host/project/session、provenance-root
    binding、各 candidate 自己的 result/edit chain、single-wrapper recursion、
    explicit-user first-party events、duplicate 与 cycle。所有 structured summary
    ref/status 在 Phase A 返回 `unverifiable_session_summary_provenance`，不读取
    content/trust、不猜 `topic_segments`。
  - Implement scoped-ID bounded relation reads、decision/provenance split and
    canonical preference conflict post-pass；uniform graph 必须是 matching，
    A-B + A-C overlap contextual error。对 touching scoped ID 的每条
    `memory_edges` 在 endpoint 过滤前 total-parse closed six-kind domain：
    Supersedes/Supports/Refutes/DerivedFrom 的 exact direction 固定；unknown kind
    返回 table/edge/raw context，NULL-source provenance 不伪造 endpoint。
  - Accept structurally valid heterogeneous canonical pairwise conflict as
    decision-neutral；unbacked 要求 candidate/operation IDs 都为 NULL，
    candidate-only error、operation-only valid；malformed provenance仍报错。
  - Map active observations into ordered/deduped `evidence_catalog`；只允许
    scoped/bitemporally effective
    `memory_facts(source_memory_id, source_observation_id)` attachment；覆盖
    NULL refs/NULL creation epoch、read-time poison scan、external supporting
    trust、empty-ref ModelGenerated default、fact learned/actual-insert `created_at_epoch`/
    valid/invalidation/replacement boundaries 与 late-insert/backdated-learned
    rejection；fact creation NULL/missing 没有 legacy fallback。
    `poisoning_quarantined` 映射 Candidate/Unknown/Live/Suppressed 并在 usable
    output/trust 前排除；explicit history 遇到 cutoff 前存在但无完整 transition
    history 的 stale/compressed row 时返回
    `unreconstructable_observation_lifecycle`。
  - Reuse canonical source classification；effective cap 取 stored class 与所有
    referenced events 重新分类后的最弱值。external/pack 不能被 WebFetch、MCP、
    network Bash 或 stale v060 default 提权；SourceTrustClass 不参加 evidence
    max，unknown class fail closed。classifier 必须读取并校验完整 plain
    content，不能只按 16 KiB preview：覆盖 `raw_keep`、current-hash
    `raw_compact`、legacy-hash `raw_compact`，并校验 retention/blob 组合、
    UTF-8/byte counts/preview/event+blob hash。锁定 16384/16385 bytes、multibyte
    boundary 和只在 compacted middle 出现 network marker；classifier 本身不得
    写入，writer 变化仅限上述 duplicate timestamp no-op。
  - Apply memory/user claim policy suppression with global/exact-owner/partial-owner
    rules and active/revoked historical intervals；识别并验证 canonical
    user-candidate/summary targets，但保持 Phase-A non-applicable。entity target
    current query 标 `CurrentSnapshotOnly`；explicit history 无 durable link
    history 时返回 `unreconstructable_entity_link_history`。
  - Reconstruct valid user-claim edit chains，区分 ClaimView state time、
    immutable provenance-root SourceRef binding 与 predecessor transition；
    preserved refs 不得在 successor creation 重新绑定。按
    authoritative candidate/result/timestamp pattern 重建 replacement/no-op
    multi-active co-predecessors，并拒绝 unexplained unlinked Superseded row；
    非 governance/versioned 的 post-cutoff in-place mutation 保守排除/Unknown。
    从 A2R durable lifecycle ledger 按 memory/time index 重建；general/Web
    governance、scope archive、cleanup-plan、save/Markdown、candidate apply、TTL、
    soft supersede、preference removal 与 stale archive 全部经 canonical service
    将 status+next changed version 原子提交，对称 status update guard 要求 open
    exact staged successor，same-status writer row 禁止；v2 startup 不得启用 bypass。
    Web row 复制 operation binding，并 exact bind durable API mutation
    resource/action/schema/response/status/time；audit ID 非 proof。
    unsupported/unrecorded/gap/fork/terminal drift/ledger mismatch 返回
    `unreconstructable_memory_lifecycle`。
  - Fail closed on unknown status、malformed/dangling/foreign/future-binding
    evidence and inconsistent routing/ownership；所有 SQL parameterized。
    memory candidate evidence 还要求 completed status、origin trust cap、
    exact confidence/persisted copied fields（不含未持久化 derived title）与
    result operation；candidate input scope 按 validated completion route 映射为
    user=global/other=project，workspace positive 必须通过。后续 owner/project/
    scope/type/raw-key transition 只接受 indexed ledger 的完整 anchor/adjacency/
    terminal chain，其他 mutation fail closed。effective memory
    knowledge 选择 earliest operation-epoch-compatible ingestion proof，再取
    canonical noop/route/ack 的 max；noop 验证自身 epoch 的 result identity 而不要求 input topic 等于
    result topic。无 proof 的 procedure memory 仅 current-snapshot
    可用，explicit historical 排除/Unknown。epoch 比较为秒级 `<=`，同秒绝对
    顺序留给 Phase C attachment sequence。projection data statements 全程
    SELECT-only。
  - Projection autocommit 使用 deferred read transaction 覆盖 first scoped SELECT
    到 final resolve；caller transaction 被复用且不由 projection commit。capture
    replay 只追加 keyed Git evidence/extraction work，原 payload 与
    creation/insertion/reference epochs 全部不变；pre-v2 stored insertion 作为
    conservative floor。
  - Verify: focused routed/global/legacy identity、Project/Owner branch、
    canonical repo-global 与 repo-rerouted-global Project-negative、
    compatibility wrapper result/error isolation（含 malformed exact-owner memory）、
    recursive candidate own-result chain、Claim/provenance-root SourceRef time、
    candidate replacement/no-op before/equal/after、workspace route、trust、Observation、七种
    suppression target、relation/overlap、>16 KiB network-Bash、summary
    fail-closed、wrapper suppression isolation、candidate route/mutation、
    procedure current-only、memory proof/ack/provenance、
    same/cross-topic noop/ack trust transition、relation source-ID shapes、
    replayability serde、
    duplicate capture timestamp/history stability、post-cutoff Observation
    stale/compressed integrity error、post-cutoff entity add/replace negative、
    Project/Owner/candidate initial-vs-cutoff route before/equal/after、
    normal-save same-type raw-key transition、stable-source-ID Markdown
    project→global/type/key 与 `markdown_import`/rollback/gap、
    A→B→C 与 coverage error、general/Web/
    scope archive/cleanup durable lifecycle before/equal/after、30-day event
    cleanup invariance、all-governed-writer strict-fingerprint/mapping/crash/retry
    与 broken-ledger error、late-insert fact、六种
    edge mapping 与 unknown-kind error、
    WAL concurrent-writer single-snapshot、authorizer transaction-control/
    `total_changes` tests；`cargo test truth -- --nocapture`.

- [ ] `GH933-A4` — Bounded lookup and performance evidence.
  - Owner: read-only verifier after A3 stops writing.
  - Run seed-933 corpus: 901 target memories、1,802 relations、901 evidence refs、
    900-link high fanout，以及 4,505 unrelated memories、9,010 relations、
    4,505 evidence refs。
  - Assert bind chunks `<=900`、route owner/target/legacy/coverage 与其他 indexed
    scoped plans、zero unrelated returned rows、
    documented data-statement formula、autocommit transaction-control count=2 /
    caller-transaction count=0，以及 unrelated corpus 前后 target output 相同；
    facts、raw edges、route/lifecycle ledgers 也必须 scoped/index-bounded，且
    history plans 不得扫描 `events`。
  - Run the structural verifier and, when useful, generate a preliminary
    implementation-head record:

    ```bash
    GH933_PERF_JSON_OUT=/tmp/gh933-truth-perf-v2-preliminary.json \
      cargo test truth_performance_contract --release -- --ignored --nocapture
    ```

  - p50/p95 are recorded, not treated as a cross-machine hard threshold。
  - This preliminary record is feedback only；A5 docs/version/base-sync work
    necessarily invalidates it。

- [ ] `GH933-A5` — Current docs, breaking release and final verification.
  - Owner: coordinator after A2/A3/A4 stop writing.
  - Update `README.md`, `docs/ARCHITECTURE.md`,
    `docs/specs/GH933/{PRODUCT,TECH}.md`, index, changelog and every distribution
    version file. Migration uses the real `remem::truth` path。
  - Use 0.7.0 or the then-current breaking boundary; never publish v2 as a
    0.6.x patch。
  - 普通 open/read-only/CLI/hook/worker/MCP/API 与通用 migration runner 在 pending
    v2 时 fail closed；仅 dedicated operator `plan`/`apply` 可执行 cutover。plan
    mode 0600 且 durable，绑定 DB identity/hash、schema/target、binary、backup、
    nonce/expiry；plan creates sole backup；apply 写 `approved`，preflight 可 exact
    retire/replan，started 只 same-attempt resume。
  - PR uses `Refs #933`; it does not claim Phase B/C completion or close issue。
  - Commit all source/docs/version changes and complete the final base sync
    first。Then, on that exact candidate SHA, regenerate the final record and
    run the remaining verification:

    ```bash
    GH933_PERF_JSON_OUT=/tmp/gh933-truth-perf-v2.json \
      cargo test truth_performance_contract --release -- --ignored --nocapture
    cargo fmt --check
    cargo check
    cargo test truth -- --nocapture
    cargo test --test truth_public_api
    cargo test
    cargo clippy --all-targets -- -D warnings
    python3 scripts/ci/check_plugin_version_sync.py
    python3 scripts/ci/check_version_bump.py origin/main HEAD
    python3 scripts/ci/check_pr_preflight.py --base origin/main \
      --pr-body-file /tmp/pr-body.md
    ```

  - Any later commit/base sync invalidates both the final record and preflight
    evidence and requires both to rerun。
  - Require fresh exact-head CI、independent review and explicit human merge
    authorization。Agent 不自动 merge。

## Later GH-933 Tasks

- [ ] `GH933-B1` — Write/update the Phase B current contract after Phase A v2 has
  real correctness and bounded-read evidence。Define shared render epoch、
  current truth/decision/conflict mapping、worktree/task selectors、budget/cache、
  error-visible behavior、historical explanation and old-path rollback。

- [ ] `GH933-B2` — Implement Context Bundle consumption without writer changes。
  One render invokes projection once；failure logs/returns error rather than an
  apparently successful empty context。Verify separately with:

  ```bash
  cargo test context
  cargo test truth
  ```

- [ ] `GH933-C1` — Benchmark and record an architecture decision for general
  Claim writers beyond the Phase A route/lifecycle history substrate:
  `converge_writers` or `retain_read_projection`。A convergence choice requires
  a separate migration/dual-write/backfill/cutover/rollback contract；a no-go
  decision records limitations and long-term owner。

- [ ] `GH933-C2` — Only if C1 chooses convergence, implement the reviewed writer
  contract and generated-enrichment Claim firewall。Otherwise mark this task
  not applicable with the architecture decision; do not pretend convergence
  happened。

- [ ] `GH933-Z1` — Closure audit。Every issue acceptance item must have current
  implementation/test evidence or an explicit reviewed no-go decision；
  docs/releases/rollback state agree；CI and review are fresh on exact head。
  Only then may a final implementation PR use closing linkage, and only a human
  decides merge/release/issue closure。

## File Ownership and Handoff

- One writable owner at a time for shared truth modules、tests、current specs、
  capture helper、version metadata and PR body。Read-only reviewers may run in parallel。
- A2 completes and hands off test paths before A3 writes them；A4 is read-only；
  A5 starts after implementation and verifier stop writing。
- Phase A migration/writer ownership is limited to A2R plus A3's reviewed
  lifecycle instrumentation. Do not add other schema/writers、`src/context/**`
  or public network surface without updating the normative contract/review。
- External GitHub comments、review-thread resolution、labels、merge、release and
  issue closure remain separate actions。This task plan does not authorize them。
