# Tech Spec

Status: Draft — human `spec_approval` blocked on the evidence prerequisite in `B-001`

## Linked Issue

GH-850（Refs #850；Epic #849）

## Product Spec

[`product.md`](product.md)

## Evidence Gate

本 spec 基于 `origin/main@778a336999876817a268c546aaa2bc6f3e3524ae` 的代码事实和
GitHub issue #850/#849 的可见内容。两者引用的
`docs/research/agent-memory-optimization-research-2026-07.md` 不存在于该 commit，本文件不
引用或转述报告内容，也不据此设定百分比收益。缺失报告（或 maintainer 明确认可并绑定
immutable revision 的等价证据）是 `spec_approval` blocker；当前 `write_spec` route 只允许
起草 product/tech，不授权 task planning、implementation、approval 或 merge。

## Codebase Context

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| Canonical memory contract | `docs/specs/current-memory-contracts/TECH.md:23`, `docs/specs/current-memory-contracts/TECH.md:30`, `src/memory/types.rs:142` | `memories` 是唯一 canonical store；`content` 是正文，`search_context` 是可重建 metadata；公开 `Memory` 不含 `search_context` | 新能力必须扩展现有 index-only 字段，不能创建第二真相或泄漏到 DTO |
| Deterministic search context | `src/memory/search_context.rs:7`, `src/memory/search_context.rs:40`, `src/memory/search_context.rs:250` | 从 type/topic/files/正文标签生成最多 4000 字符的确定性 hints，并以 500 行 batch 重建 | 是 `retrieval_text or equivalent` 的现成等价承载面和失败 fallback |
| FTS schema | `src/migrations/v012_memory_search_context.sql:7`, `src/migrations/v020_memory_fts_all_status.sql:17`, `src/migrations/v020_memory_fts_all_status.sql:24` | FTS5 已索引 title/content/search_context；insert/update/delete trigger 覆盖所有 status | 更新单一 `search_context` 即可让 FTS 消费，无需第二 FTS 表 |
| FTS query | `src/retrieval/memory_search/fts.rs:70`, `src/retrieval/memory_search/fts.rs:102`, `src/retrieval/memory_search/tests.rs:42` | 查询通过 `memories_fts MATCH`，search_context 权重为 3.0；已有测试证明它能命中但不改 content | focused regression 可直接证明 enrichment 的 FTS 消费与原文不变 |
| Write paths | `src/memory/store/write.rs:108`, `src/memory/store/write.rs:220`, `src/memory/store/write.rs:372`, `src/cli/actions/markdown_archive.rs:487` | 主要写入计算 deterministic search_context；代码库仍有多个直接 `INSERT/UPDATE memories` 路径 | schema 级失效触发器必须兜住所有 canonical writer，避免漏改某条路径 |
| Embedding input | `src/retrieval/embedding.rs:207`, `src/retrieval/embedding.rs:246`, `src/retrieval/embedding.rs:446` | passage 与 content hash 只含 type/topic/title/content，不含 search_context | vector 当前不消费 enrichment，且旧 hash 无法证明新索引文本一致 |
| Embedding backfill | `src/retrieval/vector/reindex.rs:6`, `src/retrieval/vector.rs:385`, `src/retrieval/vector.rs:445`, `src/cli/actions/query/backfill.rs:70` | 候选按 missing/`updated_at` stale 选择；向量在事务外准备、batch savepoint 写入；CLI backfill 已有 #715 的有界/幂等模式 | GH-850 应复用准备/提交分离和 coverage 语义，不引入第二 job lifecycle |
| Worker idle lane | `src/worker.rs:47`, `src/worker.rs:247`, `src/worker.rs:310` | extraction/job 都为空后，以 128 行 batch 补 embedding；无工作时 sleep | enrichment 可放在 idle lane，避免 hook 与 foreground write 等待 LLM |
| Doctor | `src/doctor/embedding.rs:41`, `src/doctor/embedding.rs:95`, `src/doctor/report.rs:81` | provider/active-model coverage 已进入统一 doctor check 列表；coverage query 失败当前只 Warn | 新 enrichment 需要独立、fail-visible 的版本与覆盖检查，并校验向量 snapshot |
| Golden eval | `src/eval/golden/types.rs:21`, `src/eval/golden/run.rs:392`, `eval/gates/baseline.json:85`, `eval/gates/thresholds.json:188` | fixture 不含 index-only 文本；seed 只调用普通 memory insert；paraphrase 六项基线为 0，threshold 只有 no-drop | 必须加入 deterministic 预计算 fixture 与正向 minimum gate，不能在 CI 调 live AI |
| Injection boundary | `src/memory/types.rs:143`, `src/context/poisoning.rs:43`, `src/context/poisoning.rs:71`, `src/context/injection_gate/data_version_hint.rs:592` | injection 加载/渲染 canonical `Memory`；poison scan 只看 title+text；search_context 只参与 substrate fingerprint | enrichment 可改变候选/数据版本，但不得进入渲染 bytes 或绕过当前 injection poison contract |
| Poison/redaction primitives | `src/memory/poisoning.rs:4`, `src/memory/poisoning.rs:50`, `src/observation_extract/prompt.rs:92`, `src/observation_extract/prompt.rs:154` | 当前有 versioned instruction-pattern/opaque scan，以及 prompt secret redaction + byte truncation | 新 generator 必须复用同类输入/输出净化，并对生成文本重新扫描 |
| Migration registry | `src/migrate/types.rs:343`, `src/migrate/types.rs:348`, `src/migrate/run.rs:190` | 最新 schema 是 v069；post-migration hook 支持需要 Rust 重建的迁移，但启动迁移在 schema transaction 中运行 | GH-850 使用下一序号 v070；迁移本身只做 O(1) additive DDL/trigger，不同步跑 AI/full backfill |

## 设计方案

### 1. 复用唯一 index-only truth

不新增公共 `retrieval_text` 列。扩展既有 `memories.search_context`，将其定义为该行唯一的
authoritative index-only text：

```text
deterministic fallback hints
context: <one bounded generated sentence>
keywords: <bounded normalized synonym list>
```

`title`、`content`、evidence 与公开 `Memory` 保持原样。FTS 继续把 title/content 和
search_context 作为三个字段索引；embedding passage 改为对同一 snapshot 的
`memory_type + topic_key + title + content + search_context` 编码。`search_context` 不加入公开
row mapper、API/MCP schema、pack/Markdown export 或 context renderer。

固定同一批 memory IDs 的 render regression 比较 enrichment 前后 byte-for-byte 输出。候选集合
因召回改善而变化属于预期，但新文本本身永不被注入。现有 data-version/substrate fingerprint
可因 FTS index 更新而变化，这是重新运行选择门禁所需，不等于渲染 enrichment。

### 2. v070 schema、identity 与全写入失效

`v070_memory_retrieval_enrichment.sql` 以 additive migration 为 `memories` 增加：

- `search_context_enrichment_version INTEGER NOT NULL DEFAULT 0`；
- `search_context_source_hash TEXT`：generator 输入的 length-delimited SHA-256；
- `search_context_index_hash TEXT`：最终 embedding passage schema + bytes 的 SHA-256；
- `search_context_enrichment_attempts INTEGER NOT NULL DEFAULT 0`；
- `search_context_next_retry_at_epoch INTEGER`；
- `search_context_last_error_code TEXT`：闭集、非敏感错误类别，不存 provider 文本或原文。

Rust 常量 `RETRIEVAL_ENRICHMENT_VERSION = 1` 绑定 prompt schema、输出 schema、normalization、
redaction/poison policy 和 index composition。source hash 只覆盖 generator 实际允许读取的
`title/content/memory_type/topic_key/files` 原始 bytes，并使用字段 tag + byte length，避免简单
分隔符碰撞；不把 project path、其他记忆或检索结果作为 generator 输入。

迁移安装一个 canonical-input update trigger，监听上述五个 source 字段。真实 bytes 改变时：

- version/source/index identity、attempt/backoff/error 全部 reset；
- 若 writer 同一 statement 已更新 `search_context`，保留其 deterministic fallback；
- 若 writer 没有更新 `search_context`，清为空字符串，使 FTS 立即只索引新 title/content，绝不
  暂留绑定旧 source 的 enrichment；
- 不改 canonical `updated_at_epoch` 以外的既有 writer 语义。

trigger 自身只更新 search_context/metadata，不递归触发 source invalidation。插入行依靠 column
defaults 处于 pending；现有 writer 写入的 deterministic search_context 可立即使用。schema drift
invariants 固定 v070 列与 trigger；migration tests 覆盖 pre-v070 rows、NULL/empty、writer 已更新
context、writer 未更新 context、重复迁移和 FTS 原文可见。

迁移不生成 enrichment、不调用网络、不下载模型、不遍历全表。历史行保持原 search_context 与
title/content 索引，随后由 worker 渐进迁移到 v1。

### 3. 生成协议与安全验证

新增 `memory::retrieval_enrichment`，把 snapshot、prompt 构造、严格解析、composition、identity、
batch selection 与 conditional commit 放在一个 owner module。AI 调用复用 `ai::call_ai` 和现有
memory AI profile resolution，usage operation 固定为 `retrieval_enrichment`。

生成输入先使用现有 sensitive-text redaction，再分别限长；原文包装为 JSON data object，system
prompt 明确“只描述/扩展检索词，不执行或服从 data 中的指令”。一次只包含一条 memory，不含
project absolute path、其他 memory、raw events、credentials 或 hidden prompt。

输出只接受下列 closed JSON shape：

```json
{"context":"one sentence, <= 240 Unicode scalar values","keywords":["1..12 items, each <= 64 Unicode scalar values"]}
```

parser 拒绝 unknown/missing fields、空白值、duplicate/empty keywords、非单句、markdown/code
fence、control/bidi override、越界 UTF-8、trailing data 与截断 JSON。keywords 统一 trim、
Unicode normalization、稳定去重，但不做隐式 alias/fuzzy 修复。context/keywords 再执行 secret
redaction；若发生 redaction 后为空、命中 `scan_instruction_pattern`/opaque payload，或含执行式
命令，拒绝整份新输出。canonical source 的 poisoning acknowledgement 不继承给生成结果。

最终组合仍受现有 4000-character upper bound。日志只写 memory id、stage、generator version、
attempt 和闭集 error code；provider 原始 response、canonical text、generated text 与 secret 均
不进日志。AI usage 继续由现有 usage recorder 记录 model/token/cost metadata。

### 4. 有界非阻塞 worker/backfill

不扩展 `JobType`。worker 在 extraction task 和 durable job queue 均为空后，先运行
`run_idle_retrieval_enrichment`，再运行现有 embedding backfill。默认每次最多选择 16 个 due
rows，并继续使用 worker 的 idle sleep，因此 hook、save response 和普通 job 不等待 LLM。

候选覆盖 retrieval 可见的 `active|stale|archived`：

- version/source identity 未达到 current；
- `next_retry_at_epoch` 为空或到期；
- 按 due time、updated time、id 稳定排序；
- 相同 current version + source hash + index hash 已 ready 的行不进入候选。

每行处理顺序：

1. 短读事务加载 snapshot，构造 source hash 后释放数据库锁。
2. 在事务外调用 AI、解析/净化，并构造 proposed search_context。
3. 若 embedding provider enabled，在事务外为 proposed authoritative passage 准备 embedding 与
   `search_context_index_hash`；provider=off 时记录显式 disabled branch，不伪造 vector。
4. 开启 `BEGIN IMMEDIATE`，重读行并验证 id/status/source hash。行删除、source 改变或另一 worker
   已提交同 version/hash 时 no-op。
5. source 仍一致时，在同一 transaction 更新 search_context/version/hashes、清 failure metadata；
   FTS update trigger 同步刷新 index；enabled embedding 以相同 index hash upsert；随后 commit。

AI/parse/security/embedding 失败时，以独立短事务只增加 attempts、写 error code 和指数退避的
`next_retry_at_epoch`，不改 search_context/version/index hash/vector；同时 error-level log。退避
上限为 15 分钟，无永久“成功”终态，后续可重试；失败行不会占满同一 batch，selector 继续处理
其他 due rows。若 failure-state transaction 自身失败，向上传递 error 并让 worker sleep/退出该
sweep，不静默丢失诊断。

`once` worker 模式仍只做有界 sweep；若本 sweep 只有失败/no-op，返回 no-work，避免 tight loop。
crash 在 conditional commit 前保持旧一致 snapshot；commit 后重复选择看到 current identity 并
no-op。

### 5. FTS 与 embedding 的一致消费

FTS schema/query 不新增表或分支：existing `memories_au` trigger 在事务内同步把最终
search_context 写入 FTS。focused test 用只存在于 generated keywords 的 term 证明 FTS 命中，
同时断言 content 与公开 Memory JSON 不含该 term。

`retrieval::embedding` 增加明确的 `memory_index_text(..., search_context)` 与 versioned hash
schema（例如 `memory-index-v2` prefix）。curated semantic-dedup 的 canonical comparison 保持原
接口，不误把尚未生成的 index text 当业务内容。`vector` candidate/load/prepare paths 显式读取
search_context；`memory_embeddings.content_hash` 等于该行 `search_context_index_hash`。

enrichment commit 在 provider enabled 时必须同时 upsert active model vector，否则整行回滚，旧
FTS/vector snapshot 保留。provider=off 时允许只 commit FTS，doctor 显示 vector disabled；以后
启用 provider，现有 missing-model backfill 读取 authoritative search_context 并生成匹配 hash。
active-model coverage 只有在 model/dimensions 匹配且 embedding content_hash 等于当前
search_context_index_hash 时才算一致，不能把旧原文 vector 计为 enriched-ready。

新写入在 enrichment pending 时仍用 deterministic search_context 走既有同步 embedding；生成
成功后原子替换。旧行在 enrichment 失败时保留已有原文/deterministic vector；缺 vector 的行仍由
现有 missing-vector backfill 提供 fallback。任何路径都不删除其他 model profile，prune 仍遵守
#715 的显式授权。

### 6. Doctor 与 evidence integrity

新增独立 `Retrieval enrichment coverage` check，读取 current generator version 并对 eligible rows
重新计算 source identity，报告：

- eligible total；
- current/ready；
- pending；
- failed/backoff；
- source identity drift；
- provider enabled 时 vector-consistent；provider off 时 explicit disabled。

eligible=0 返回 OK 0/0。所有 eligible rows 都 current 且 enabled vector 一致才为 OK；部分覆盖
为 Warn，并给出启动 worker/等待 backfill/检查 error log 的恢复动作；identity drift、非法 version
或查询/哈希计算失败为 Fail。错误不得折算成 0。现有 embedding coverage 同步改为只统计匹配
current index hash 的 vectors，避免两个 doctor check 给出矛盾结论。

diagnostic 与 ready evidence 绑定 memory id、version、source hash/index hash（doctor 输出仅显示
计数和 version，不输出 hash 全值）。测试通过篡改 metadata、old vector hash、provider off、DB
缺列/错误与 0 rows 验证 fail-visible 行为。

### 7. Deterministic golden paraphrase gate

`GoldenMemory` 增加 optional、仅 eval fixture 使用的 `search_context`。`seed_fixture_corpus` 对该
字段执行与 production 相同的 bounds/poison validation，然后直接安装已审查、versioned 的
precomputed context 并同步构造 test provider vector；不得调用 `call_ai`、网络或本机用户配置。
未提供字段的旧 fixture 行行为不变。

在 `eval/golden.json` 为 paraphrase cases 提交人工审查的 context/keywords，并添加 EN/CJK 负例
防止把 expected query 原样复制成作弊关键词。`src/eval.rs` 的 threshold schema 增加
`min_value`，为下列三项设置严格正值（`> 0` 的 machine threshold），同时保留所有现有
`max_drop=0`：

- `golden.slice.paraphrase.hit_at_k`；
- `golden.slice.paraphrase.evidence_recall_at_k`；
- `golden.slice.paraphrase.mrr_at_10`。

implementation PR 在 `eval/retrieval-enrichment/report.json` 记录 exact base SHA、head SHA、
generator version、fixture hash、base/head 三项值和所有 gate 结果。exact-main base 必须复现三项
0；head 三项必须严格大于 0。更新 `eval/gates/baseline.json` 只发生在该 comparison 通过后，不能
先把零 baseline 改掉再宣称改善。

此外，FTS focused test 与 feature-hash/local vector focused test 分别使用仅存在于 enrichment 的
同义 term，证明两个 channel 独立消费；golden fused improvement 不能替代 channel wiring 证据。
现有 abstention、project scope leak、knowledge update、temporal、capacity 与 current-memory
contracts 继续走原 thresholds。

### 8. Compatibility、rollout 与 rollback

v070 只有 additive columns/triggers，旧客户端 payload 与现有数据库备份兼容。实现文档同步
current-memory contract（canonical content + rebuildable search_context）、local-semantic contract
（passage input/hash）和 architecture/index；用户说明 enrichment 在 worker 后台执行且不会注入。

rollout 顺序：migration → foreground deterministic fallback → idle batches → doctor coverage →
golden gate。不得等全量回填才完成启动，也不得在 migration hook 中 AI backfill。

未合并时直接关闭 implementation PR。合并后若需要 rollback：

1. 发布 forward-only recovery（不 down-migrate v070），停止新 generator sweep；
2. 复用 `search_context::rebuild_all` 的 bounded deterministic builder 重建 search_context，清除
   current enrichment identity/failure metadata，并删除受影响 active-model vectors 使现有 backfill
   重建；
3. 每批保持 title/content FTS 可见，验证 doctor/golden 后再移除 generator 调度；
4. 保留 additive columns 和非敏感失败审计，旧 binary 可忽略它们。

rollback 不改 canonical bytes、不 prune 其他 model profiles、不使用 destructive down migration。
若 forward recovery 中断，重复执行从 identity/pending 状态继续。

## Planned Changes Manifest

```specrail-planned-changes
{
  "issue": 850,
  "complete": true,
  "paths": [
    "specs/GH850/product.md",
    "specs/GH850/tech.md",
    "README.md",
    "docs/ARCHITECTURE.md",
    "docs/specs/README.md",
    "docs/specs/current-memory-contracts/TECH.md",
    "docs/specs/local-semantic-embedding/PRODUCT.md",
    "docs/specs/local-semantic-embedding/TECH.md",
    "src/migrations/v070_memory_retrieval_enrichment.sql",
    "src/migrate.rs",
    "src/migrate/types.rs",
    "src/migrate/schema_drift/invariants.rs",
    "src/migrate/tests_retrieval_enrichment.rs",
    "src/migrate/tests_schema_drift.rs",
    "src/memory.rs",
    "src/memory/types.rs",
    "src/memory/search_context.rs",
    "src/memory/retrieval_enrichment.rs",
    "src/retrieval/embedding.rs",
    "src/retrieval/vector.rs",
    "src/retrieval/vector/reindex.rs",
    "src/retrieval/vector/coverage.rs",
    "src/retrieval/vector/tests.rs",
    "src/retrieval/memory_search/tests.rs",
    "src/worker.rs",
    "src/worker/tests.rs",
    "src/doctor.rs",
    "src/doctor/report.rs",
    "src/doctor/embedding.rs",
    "src/doctor/retrieval_enrichment.rs",
    "src/context/tests/render_stability.rs",
    "src/api/tests.rs",
    "src/eval.rs",
    "src/eval/golden/types.rs",
    "src/eval/golden/run.rs",
    "src/eval/golden/tests.rs",
    "eval/golden.json",
    "eval/gates/baseline.json",
    "eval/gates/thresholds.json",
    "eval/retrieval-enrichment/report.json",
    "Cargo.toml",
    "Cargo.lock",
    "plugins/remem/.codex-plugin/plugin.json",
    "plugins/remem/runtimes/remem-releases.json",
    "npm/remem/package.json"
  ],
  "spec_refs": [
    "B-001",
    "B-002",
    "B-003",
    "B-004",
    "B-005",
    "B-006",
    "B-007",
    "B-008",
    "B-009",
    "B-010",
    "B-011",
    "B-012",
    "B-013",
    "B-014",
    "B-015",
    "B-016",
    "B-017",
    "B-018"
  ]
}
```

manifest 是 implementation 的完整预期文件边界；若实现探索证明需要新增路径，必须先更新
tech spec 并重新走 human spec review，不能静默扩大范围。version surfaces 因仓库的
`src/**` version gate 被显式列入；`AGENTS.md`、hooks/config 高上下文文件不在计划内。

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| `B-001` evidence prerequisite | spec metadata + Git tree evidence | `git cat-file -e 778a336999876817a268c546aaa2bc6f3e3524ae:docs/research/agent-memory-optimization-research-2026-07.md` 当前非零；未绑定替代证据时 route/handoff 必须保持 blocked |
| `B-002` one index-only truth | v070 schema + `memory::retrieval_enrichment` | migration/schema test 断言仅扩展 `search_context` 且无 `retrieval_text` column/table；`rg -n "retrieval_text" src/migrations src/memory` 只允许文档/测试负断言 |
| `B-003` canonical/output bytes | memory mapper + context/API regression | `cargo test context::tests::render_stability`; `cargo test api::tests`; focused test 比较同 IDs 的 render bytes 与 DB `hex(content)` 前后相等 |
| `B-004` closed output | strict parser/normalizer | `cargo test memory::retrieval_enrichment::tests::output_contract` 覆盖 missing/empty/extra/limits/control/bidi/trailing/truncated JSON |
| `B-005` same FTS/vector snapshot | FTS trigger + versioned embedding input/hash | `cargo test retrieval::memory_search::tests::fts_search_uses_retrieval_enrichment`; `cargo test retrieval::vector::tests::embedding_uses_retrieval_enrichment`; 断言两者 index hash 相同 |
| `B-006` foreground fallback/non-blocking | write fallback + worker scheduling | focused write/hook test 使用 blocking fake generator，断言 save/hook 已返回且原文 FTS/initial vector 可查；`cargo test worker::tests::worker_once_backfills_pending_retrieval_enrichment` |
| `B-007` bounded/idempotent retry | idle selector + retry metadata | `cargo test memory::retrieval_enrichment::tests::backfill_` 覆盖 batch 16、all statuses、重复 current row 零 AI calls、失败退避与其他行继续 |
| `B-008` fail-visible fallback | per-stage failure handling | `cargo test memory::retrieval_enrichment::tests::failure_` 注入 AI/parse/redaction/poison/embed/DB errors，断言 error log、failure code、原文 FTS 与旧 vector 保留、ready count 不增加 |
| `B-009` canonical invalidation | v070 update trigger | migration tests 分别更新 title/content/type/topic/files，断言 old identity reset、旧 enrichment 不可用且新 title/content 立即 FTS 可见 |
| `B-010` concurrent CAS | conditional commit | 双 connection barrier tests 覆盖 update/delete/status change/winner commit；loser 影响行数为 0 且不覆盖新 source/version |
| `B-011` atomicity/cancellation | prepare outside transaction + single commit | fault injection 在每个 commit boundary crash/rollback；重开 DB 后只允许 old-consistent 或 new-consistent，provider=off 负例明确无 vector |
| `B-012` compatibility/migration | v070 + unchanged DTO | pre-v070 fixture migration、NULL/empty rows、old JSON snapshot tests；迁移期间 fake AI call count=0，title/content FTS 可命中 |
| `B-013` doctor coverage | new doctor check + embedding coverage | `cargo test doctor::retrieval_enrichment::tests`; 覆盖 0/0、partial、failed、drift、wrong vector hash、provider off 和 DB error→Fail |
| `B-014` permission/privacy | prompt builder + existing memory AI resolver | prompt snapshot tests 断言单 row、redaction、byte budget、无 project path/other row；fake executor 证明只通过既有 profile，未配置/不可用走 `B-008` |
| `B-015` poisoning | output validator + current scanner | `cargo test memory::retrieval_enrichment::tests::rejects_`; 中英 instruction、opaque payload、canonical acknowledged + generated poison 仍拒绝，且 injection regression 继续通过 |
| `B-016` audit identity | metadata/log/doctor/eval report | structured log capture 断言 id/stage/version/error code 存在而 payload/secret 不存在；篡改 old hash/version 后 doctor/eval 不接受 ready |
| `B-017` golden paraphrase gate | deterministic fixture + `min_value` + report | `cargo test eval::golden`; `cargo run -- eval-gates --json-out /tmp/gh850-gates.json`; report 断言 base SHA 三项=0、head 三项>0、无 live AI、其他 gates pass |
| `B-018` rollback | bounded deterministic rebuild + forward recovery rehearsal | 临时 DB 先全量 enrich，再分两批 deterministic rebuild，中断重启后完成；断言 content hex 不变、FTS 始终命中、vectors 被重建为 fallback hash |

## 数据流

正常写入与后台升级：

```text
save/import/promote
  -> canonical title/content + deterministic search_context transaction
  -> FTS title/content/fallback immediately visible
  -> existing foreground embedding of current fallback (when enabled)
  -> response/hook returns

worker idle sweep
  -> load one bounded/redacted memory snapshot + source hash
  -> existing memory AI profile (outside DB transaction)
  -> strict JSON / bounds / secret / poison validation
  -> compose one authoritative search_context
  -> prepare active-model embedding + index hash (outside transaction)
  -> BEGIN IMMEDIATE; re-read + compare source/version
  -> UPDATE memories (FTS trigger) + UPSERT matching vector
  -> COMMIT; doctor/eval can count ready
```

失败与竞态：

```text
any pre-commit failure
  -> short failure-metadata transaction + redacted error log + backoff
  -> old consistent search_context/vector remain; title/content FTS remain

source changes while AI runs
  -> v070 trigger resets identity and stale enrichment
  -> conditional commit observes hash mismatch
  -> no-op; new source remains original/fallback indexed
```

持久化只增加 v070 metadata；生成全文只存在于 `search_context`，失败响应不持久化。外部调用只有
已授权的 existing memory AI executor，以及既有 configured embedding provider；SQLite transaction
内不执行网络/模型调用。

## 备选方案

- **新增 `retrieval_text` 与保留 `search_context`**：拒绝。两份 index-only text 会在 writer、FTS、
  embedding、backfill 和 rollback 间漂移，违反 `B-002`。
- **覆盖 canonical `content`**：拒绝。破坏证据、导出、注入 bytes 与用户信任，违反 `B-003`。
- **只让 FTS 使用 enrichment**：拒绝。issue 明确要求 embedding 同步消费，且 hybrid channels 会
  对同一 memory 形成不一致 identity。
- **FTS 与 vector 分别调用 AI**：拒绝。成本翻倍且不可证明 snapshot 相同。
- **在 save/hook 同步生成**：拒绝。LLM/network/model latency 会阻塞高频 hook，失败也会扩大到
  canonical write。
- **用新的 durable `JobType`**：暂不采用。现有 idle embedding lane 已证明有界 backfill pattern；
  新 job 类型会扩大 lease/retry/failure lifecycle 与 migration 范围。v070 retry metadata足以隔离坏行。
- **迁移时一次性回填全表**：拒绝。会延长 schema startup lock，且 migration 中不能安全调用 AI。
- **失败时写空 search_context 或删除旧 vector**：拒绝。会造成静默召回损失；原文/旧一致
  snapshot 必须保留。
- **用 live LLM 运行 golden eval**：拒绝。CI 不可重复且会把 provider availability 误当质量变化。
- **只更新 baseline、不加正向 threshold**：拒绝。当前 paraphrase=0 且 no-drop threshold 允许永远
  保持 0，不能证明改善。

## 风险

- Security: canonical/generated text 均可携带 prompt injection；必须以 data boundary 发送，输入
  输出双向 redaction，生成结果重新 poison scan。remote memory AI 只能沿既有配置授权，日志不含
  payload。该区域按 SEC-11 要求人类 security review。
- Privacy: enrichment 可能扩展敏感概念的可检索性。generator 不读取跨 row/project 数据，不索引
  新 secret，输出再 redaction；公共 DTO/render/export 永不暴露 search_context。绝对路径只用于
  local filtering，不进入 prompt。
- Integrity: hallucinated synonym 会制造 false-positive recall。closed schema、单句/关键词 bounds、
  poison rejection、golden positive/negative fixtures 和不注入生成文本限制 blast radius；最终 spec
  approval 仍等待研究依据。
- Compatibility: v070 additive columns 对旧 readers 安全；旧 binary 会继续用已存 search_context
  做 FTS，因此 rollback 必须先 forward rebuild，不能仅降 binary。
- Concurrency: SQLite worker、foreground writers 和多 worker 可能交错。source hash CAS 与单事务
  FTS/vector commit 是 correctness boundary；不得在 AI await 时持锁。
- Performance/Cost: 每条 pending memory 最多一次 current-version AI call，batch=16、idle-only、
  exponential backoff 限制峰值；doctor 暴露 backlog。不能以成本为由删除 LLM enrichment或静默
  降级，但可以通过 worker pacing 保护 latency。
- Retrieval quality: 新 terms 可能提高 recall 同时降低 precision。现有所有 non-paraphrase gates
  no-drop，paraphrase minimum + channel-focused tests 同时约束。
- Maintenance: prompt/output schema、version、hash schema、doctor 与 golden fixture 必须一起更新；
  任一语义改变都 bump generator version 并触发渐进回填。
- Test integrity: 不准把 paraphrase query 原样复制进 fixture context、不准删除 abstention/scope
  negatives、不准在 tests 中绕过 production validator。

## 测试计划

- [ ] Evidence check：确认 prerequisite report 的 immutable path/revision；缺失时明确阻塞 human
      `spec_approval`，不继续 implementation gate。
- [ ] Route：
      `python3 checks/route_gate.py --repo . --route write_spec --issue 850 --state ready_to_spec --json`。
- [ ] Migration/schema：v070 pre-upgrade、additive columns、triggers、NULL/empty、FTS fallback、schema
      drift、idempotent convergence tests。
- [ ] Parser/security：closed JSON、bounds、Unicode/control/bidi、redaction、中英 poison、opaque
      payload、acknowledgement non-inheritance。
- [ ] Worker/backfill：batch bound、all statuses、idempotency、retry/backoff、bad-row fairness、once/sleep、
      no foreground/hook blocking。
- [ ] Concurrency/atomicity：双 worker、source update/delete/status race、embedding failure、SQLite failure、
      cancel/crash before/after commit。
- [ ] FTS/vector：两条 focused channel tests；provider enabled/off、wrong model/dims/hash、later enable
      backfill。
- [ ] Output isolation：DB `hex(title/content)`、Memory JSON、API/MCP、pack/Markdown、context render
      byte snapshots。
- [ ] Doctor：0/0、partial、failed/backoff、identity drift、wrong vector、provider off、database failure。
- [ ] Golden：precomputed contexts，无 live AI；exact-main 三项=0、head 三项>0、所有既有 gates pass；
      comparison report 含 SHA/version/fixture hash。
- [ ] `cargo test memory::retrieval_enrichment`。
- [ ] `cargo test retrieval::memory_search`。
- [ ] `cargo test retrieval::vector`。
- [ ] `cargo test worker::tests`。
- [ ] `cargo test doctor`。
- [ ] `cargo test eval::golden`。
- [ ] `cargo run -- eval-extraction --json --check-baseline`。
- [ ] `cargo run -- eval-gates --json-out /tmp/gh850-eval-gates.json`。
- [ ] `python3 scripts/ci/check_plugin_version_sync.py`。
- [ ] `cargo fmt --check`。
- [ ] `cargo check`。
- [ ] `cargo clippy -- -D warnings`。
- [ ] `cargo test`。
- [ ] full PR preflight 使用 implementation PR 的实际 body：
      `python3 scripts/ci/check_pr_preflight.py --base origin/main --pr-body-file /tmp/pr-body.md`。
- [ ] `git diff --check origin/main...HEAD`，并人工核对 manifest paths、无 public enrichment field、
      无 secrets、无测试弱化。

## 回滚方案

spec-only 阶段删除/关闭草案即可，不产生 schema 或运行时状态。本草案不授权实现或 merge。

implementation 合并后的回滚使用“先 forward repair、后停用代码”，不执行 `ALTER TABLE DROP`
或恢复旧 schema version。forward repair 逐批调用 deterministic rebuild，清 current enrichment
identity/failure metadata，并只使受影响 active-model vectors 进入现有 reindex；title/content FTS 在
全过程保持。repair 可中断/重复，doctor 报告 pending 直到 fallback vector 一致。完成后再发布不
调度 generator 的 binary；additive v070 columns/triggers 保留给新旧 binary 忽略/使用。

若回滚原因是 security/privacy，先停止 worker 进程以阻止新外部调用，再发布 forward repair；
停止 worker 属于运维动作，必须由授权人执行。任何删除 AI usage/audit、prune 其他 model vectors、
改写 canonical memory 或 down migration 都不属于允许的 rollback。

本文件不构成 `spec_approval`。在 `B-001` 证据补齐、maintainer 批准 product+tech 且 GH-850 被
人工置为 `ready_to_implement` 前，不得生成 `tasks.md` 或开始 implementation。
