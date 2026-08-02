# Tech Spec：CurrentTruth 统一读取契约

## Linked Issue

GH-933

PR #939 已于 2026-07-26 合并（merge commit `0ed42e3d`），并随 GitHub
Release 与 crates.io `remem-ai` 0.6.26 公开发布；0.6.27 仍包含同一 public
v1 API。它是使用 `Refs #933` 的 Phase A baseline，不是 GH-933 的完整实现。
PR #965 已退役旧的仓库内执行工作流；本文件只保留 issue-level 规划证据，
规范性当前契约位于 `docs/specs/GH933/`。后续实现按普通 issue/PR、代码审查、
CI 与显式人工 merge 授权推进。

## Product Spec

[`product.md`](product.md)，行为契约 `CT-001`–`CT-015`。

## Codebase Context

| Area | Current truth | Hardening consequence |
| --- | --- | --- |
| Public API | Cargo package 名为 `remem-ai`，但 `[lib] name = "remem"`；真实 Rust path 是 `remem::truth` | README、docs、changelog 与 compile-able integration test 必须使用真实 path |
| v1 DTO | `TRUTH_PROJECTION_VERSION = 1`，裸 `subject_key`，effective “now” 不进入输出 | v2 必须提供 typed identity、可审计的 `reference_epoch` 与 replayability |
| Adapter | 多段读取 memories/evidence/edges/user claims，无统一 snapshot；不读 Observation/policy | 补齐 Observation/suppression，并把全 projection 固定在一个 read snapshot |
| Trust | 任意 tool event 会被标为 Verified，resolver 对 evidence 取 max | external tool output 可被提权；v2 必须复用 canonical source classification 与 cap |
| Observation status | writer 可写 `poisoning_quarantined`，lifecycle mapper 尚未列出 | quarantined prompt-injection 内容必须被显式 suppress，不能进入 catalog/truth |
| Temporal history | route discovery 只有 creation/current 候选；capture replay 覆盖时间；部分 mutation 缺 history | indexed route ledger/backfill；replay 时间 immutable；不可重建历史显式失败 |
| Cutover | ordinary open 自动 migration | prep journal；pre-start full rebuild；Windows v1 fallback；started exact resume |
| Relations | loader 全扫 edge table 后在 Rust 过滤；canonical heterogeneous pairwise conflict 使用 fallback operation metadata | relation lookup 必须 bounded；合法 heterogeneous operation 不得误报损坏 |
| Suppression | suppression row 有时间，`memory_entities` link 没有 | owner-safe intervals；entity current-only，历史 link 不可证明即失败 |
| Tests | `src/truth/tests.rs` 已有 679 行 | 加测试前先拆为 `src/truth/tests/**`，任何单文件保持少于 800 行 |
| Distribution | v1 已公开；v2 会改变合法输入的 public DTO/selector/output | v2 使用 0.7.0 或实现时下一个 breaking SemVer boundary，不能作为 0.6.x patch |

## Public v2 Contract Snapshot

以下 shape 是本历史 packet 在本次修订时对规范性
`docs/specs/GH933/TECH.md` 的镜像快照，不是第二份 normative source；若两者
不一致，以 current contract 为准。所有 enum 使用
`#[serde(rename_all = "snake_case")]`；tagged enum 使用字段
`scope_kind`。所有 `Option` 在 JSON 中显式序列化为 `null`，不得
`skip_serializing_if`，以保持 golden shape 稳定。

```rust
pub const TRUTH_PROJECTION_VERSION: u32 = 2;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimSource {
    Memory,
    UserContextClaim,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
pub struct SubjectIdentity {
    pub source: ClaimSource,
    pub owner_scope: String,
    pub owner_key: String,
    pub memory_scope: Option<String>,
    pub kind: String,
    pub key: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "scope_kind", rename_all = "snake_case")]
pub enum TruthScope {
    Project {
        project: String,
        branch: Option<String>,
    },
    Owner {
        owner_scope: String,
        owner_key: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TruthQuery {
    pub scope: TruthScope,
    pub as_of_epoch: Option<i64>,
    pub subject: Option<SubjectIdentity>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionReplayability { Exact, CurrentSnapshotOnly }

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    CapturedEvent,
    SourceRef,
    SourceTrustClass,
    Observation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceIntegrity {
    Validated,
    Opaque,
    Quarantined,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EvidenceView {
    pub evidence_ref: String,
    pub kind: EvidenceKind,
    pub source_ref: String,
    pub scope: TruthScope,
    pub lifecycle: Option<Lifecycle>,
    pub source_time_epoch: Option<i64>,
    pub knowledge_time_epoch: i64,
    pub trust: EvidenceTrust,
    pub integrity: EvidenceIntegrity,
    pub supporting_evidence_refs: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ClaimView {
    pub canonical_ref: String,
    pub subject: SubjectIdentity,
    pub statement: String,
    pub branch: Option<String>,
    pub lifecycle: Lifecycle,
    pub valid_from_epoch: Option<i64>,
    pub valid_to_epoch: Option<i64>,
    pub source_time_epoch: Option<i64>,
    pub knowledge_time_epoch: i64,
    pub evidence: Vec<EvidenceView>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CurrentTruthView {
    pub subject: SubjectIdentity,
    pub claim: Option<ClaimView>,
    pub validity: ValidityState,
    pub evidence: Vec<EvidenceView>,
    pub supporting_relations: Vec<RelationView>,
    pub contradicting_relations: Vec<RelationView>,
    pub rejected: Vec<String>,
    pub conflicting_claims: Vec<ClaimView>,
    pub selected_reason: TruthSelectionReason,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CurrentTruthProjection {
    pub projection_version: u32,
    pub scope: TruthScope,
    pub requested_as_of_epoch: Option<i64>,
    pub reference_epoch: i64,
    pub replayability: ProjectionReplayability,
    pub truths: Vec<CurrentTruthView>,
    pub evidence_catalog: Vec<EvidenceView>,
}

pub fn project_current_truth(
    conn: &rusqlite::Connection,
    query: &TruthQuery,
) -> anyhow::Result<CurrentTruthProjection>;

pub fn project_user_claim_truth(
    conn: &rusqlite::Connection,
    owner_scope: &str,
    owner_key: &str,
    as_of_epoch: Option<i64>,
) -> anyhow::Result<CurrentTruthProjection>;
```

`RelationView`、`Lifecycle`、`EvidenceTrust`、`TruthSelectionReason` 的 v1 字段和
snake_case serde names 保持不变。`TruthScope` 必须实现 `Serialize`，让输出能
完整回放输入 scope。Owner selector 的 owner 必须 exact match；Project selector
必须是 non-global Memory identity，但 membership 由 row-level Project routing
+ branch predicate 决定，不要求 `owner_key=project`，因此 owner Q、
`target_project=P` 的 row 可匹配 Project(P)。结构不兼容返回 contextual error；
兼容 selector 没有 scoped row 时固定 `truths=[]`，只有已 load identity 没有
eligible survivor 才输出 `Unknown`。Project scope 查询 memories 与 scoped
observation catalog；Owner scope 查询 canonical owner exact-match 的 memories
与 user-context claims，`evidence_catalog` 为空。selector 只过滤 `truths`，
不改变 Project observation catalog。

current contract 中的 `project_current_truth` 按 `TruthQuery.scope` dispatch，
是 v2 public entrypoint。`project_user_claim_truth` 作为 user-claim-only
compatibility convenience 保留：在 row load/validation 前先选择
`ClaimSource::UserContextClaim`，再复用 shared user-claim Owner
adapter/resolver。它不枚举/验证无关 memory claims、memory-only relations、
Observation attachment 或 memory-only suppression；只 bounded 读取对 selected
claims 可应用的 `user_claim`/`pattern` suppression。只有 explicit
`preference_backfill` ref 可定点读取 memory，因此 unreferenced malformed
exact-owner memory 不改变 wrapper result/error domain。exact-owner memories
只由 normative Owner query 返回。v1 的
`load_memory_claim_groups`/`load_user_claim_groups` 不再从
`remem::truth` re-export，adapter loaders 改为 crate-private，避免调用方绕过
shared epoch、scope、suppression 与 catalog。

`as_of_epoch=Some(t)` 使用 `t/exact`；`None` 只取一次 wall-clock，no-proof
binding 或 unversioned entity link 使其 `current_snapshot_only`。所有 stage 共享
该 epoch 与一个 SQLite snapshot：autocommit 入口拥有 deferred BEGIN 和 terminal
COMMIT/ROLLBACK；caller transaction 被复用且不由 projection commit。

输出排序固定为：

1. `truths` 按 `SubjectIdentity` 字段的派生 lexicographic order；
2. 每个 claim 的 evidence 按 `(source_time_epoch, knowledge_time_epoch,
   evidence_ref)` 升序，其中 `None` 早于任意 `Some`；
3. `evidence_catalog` 以同一 evidence key 排序，并按 `evidence_ref` 去重；
4. relations 按 `(created_at_epoch, relation_ref)`，canonical refs 按字节序。

各 Evidence kind 的 v2 field semantics：

| Kind | scope | lifecycle | source/knowledge time | integrity | supporting refs |
| --- | --- | --- | --- | --- | --- |
| CapturedEvent | canonical event project，branch=`None` | `None` | immutable first-capture source；insertion | Validated | empty |
| SourceRef | containing user-claim Owner scope | `None` | claim-version valid-from-or-created；immutable provenance-binding epoch | resolved structured ref 为 Validated；仅 manual/free-form 为 Opaque | resolved nested canonical refs 或 empty |
| SourceTrustClass | containing memory query scope/claim branch | `None` | memory reference/created；下文 effective memory knowledge | Validated | empty |
| Observation | canonical observation Project/branch | `Some(observation lifecycle)` | observation reference/created；observation `created_at_epoch` | Validated or Quarantined | validated captured-event refs |

`SourceTrustClass` view 是 diagnostic/cap provenance，不参加
`strongest_evidence` 的 max；cap 单独按下文计算。CapturedEvent 的 branch 为
`None`，因为 canonical event schema 不持久化 branch，adapter 不得从当前
workspace branch 猜值。SourceRef 的 Opaque 表示已通过 JSON shape validation，
但其自由格式指针没有 ledger integrity proof。

CapturedEvent `source_ref` 沿用 v1 `event_type/role`（无 role 时为
`event_type`）；SourceTrustClass `source_ref` 是 validated stored class；
structured SourceRef 使用 canonical compact JSON。

`EvidenceView.evidence_ref` 固定为：CapturedEvent
`captured_event:<event-id>`、SourceTrustClass
`memory_trust_class:<memory-id>`、Observation `observation:<observation-id>`、
empty-manual SourceRef `user_claim_source:<claim-id>:manual`，其余 SourceRef
为 `user_claim_source:<claim-id>:<path>`。legacy top-level manual JSON string
规范成 synthetic singleton array，path 固定 `0`。每层 ref array 递归 canonicalize、
exact-dedup、按 bytes 排序，再用 zero-based canonical index 组成冒号分隔 path；
candidate nested ref 例如 `0:0`，绝不使用原 JSON index。wrapper 与每个 nested
ref 都有独立 SourceRef EvidenceView，wrapper 的 supporting refs 指向直属 child
paths；empty/manual/nested candidate 都纳入 v2 golden。

User-claim `source_kind` 是 closed mapping：

| source_kind | cap / required provenance |
| --- | --- |
| `manual` | Verified；zero refs 或仅 nonblank strings/`manual_cli`，均为 Opaque |
| `explicit_user_statement` | Verified cap；1+ captured events 且全部 first-party（`event_type=user_prompt_submit` 或 `role=user`），可另有最多一个 session summary |
| `preference_backfill` | 等于 referenced memory 的 effective trust；必须恰有一个有效 memory ref |
| `inferred_from_behavior` | ModelGenerated；1+ behavior captured events，可另有最多一个 session summary |
| `session_summary` | ModelGenerated；1+ captured events，可另有最多一个 session summary |
| `speculative_inference` | ModelGenerated；1+ captured events，可另有最多一个 session summary |
| `third_party_statement` | Untrusted；1+ captured events，可另有最多一个 session summary |
| `user_context_candidate` | 恰一个 authoritative candidate wrapper；cap 递归得出 |

`source_refs_json` 必须是 array；唯一例外是 legacy `manual` nonblank scalar
string。只有 `manual` 可用 empty array 并生成 Opaque `source_ref=manual`；
其他 kind 至少一个 resolved ref并满足上表；其他 scalar/blank 是 error。
上表是 exact allowlist，额外 kind/count 均报错。五种 terminal extraction kind
使用 canonical writer 的 captured-events + optional-one-summary shape；即使
`session_summary` 也拒绝 direct summary-only。candidate 只能按下文
single-wrapper rule 递归。structured object 使用 exact schema：

| ref kind | exact fields/types | resolution、scope、time |
| --- | --- | --- |
| `captured_event` | `{"kind":"captured_event","id":<positive integer>}` | row 存在且 time eligible；enclosing candidate 要求 exact host/project/session，direct ref 以 containing `repo` owner key 为 project anchor |
| `session_summary` | `{"kind":"session_summary","id":<positive integer>}` | recognized，但因下文 read-only limitation 在 Phase A 不可用 |
| `memory` | `{"kind":"memory","id":<positive integer>}` | row 存在、time eligible、canonical owner 与 containing claim exact match；trust 使用该 memory effective trust |
| `manual_cli` | `{"kind":"manual_cli","command":<nonblank string>}` | 不解析 row，在 containing Owner scope 保持 Opaque |
| `user_context_candidate` | `{"kind":"user_context_candidate","candidate_id":<positive integer>,"source_kind":<string>,"source_refs":<array>}` | 对 authoritative candidate 校验并递归解析 refs |

object 不允许 extra fields。non-repo Owner 的 project-bound direct ref 没有持久化
project anchor，必须报错。missing、foreign-scope、quarantined 或 malformed
referent 都是 contextual error。referent 晚于 immutable binding epoch 也是
integrity error；合法 bound referent 只有在晚于 query cutoff 时才作为
time-ineligible。同一 sibling array 的 exact duplicate 先按 canonical compact
JSON/string bytes 折叠；相同 kind/ID 不同 payload 报错。最终 refs byte-sort。

terminal ref 绑定 enclosing candidate creation；direct ref 绑定 edit chain 中
首次引入 exact kind/refs 的 provenance root，后续 edit 必须保留且不得重绑。
source/knowledge 必须 `<=` binding/reference；event knowledge 是 insert，memory
使用 `effective_memory_knowledge_epoch`。wrapper knowledge 是 validated
application，必须 `<=` containing binding（top-level root-result creation；
nested enclosing-candidate creation）。比较为秒级，equality eligible；同秒绝对
顺序需 Phase C durable sequence。
explicit-user 至少一个 captured ref，且全部必须 first-party。

top-level `source_kind=user_context_candidate` 必须且只能有一个 wrapper。
每个 wrapper 的 authoritative candidate 必须有 nonblank host/session/project；
event 与 optional summary 必须 join 这三者表示的 exact
`(host_id,project_id,session_row_id)`，missing/foreign/disagreement 报错。
nested `source_kind` 是五种 terminal kind 之一（refs 遵循对应 event/optional
summary rule 且不得含 candidate），或 `user_context_candidate`（refs 恰一个
wrapper，不得混 terminal refs）。candidate 必须存在、owner exact 且 application
不晚于 containing binding/reference；wrapper kind/refs structural-equal stored
row。其 `result_claim_id` 指自己的 initial result；top-level result 是 current
row/ancestor，nested wrapper 则验证自己的 root/edit chain。replacement/new
initial row exact copy candidate
user/owner/type/key/text/confidence/sensitivity，使用 canonical wrapper、NULL
validity，并让 creation/initial-update/last-confirmed 等于 application。后续
descendant 保留 user/owner/confidence/source kind/refs，各自 creation 等于 edit
transition，只允许 type/key/text/sensitivity/validity 改变。no-op 指向 ordered
pre-existing exact match；kept row/SourceRefs 不变，不能冒充 wrapper Claim。
nested wrapper 重复全部校验；ancestry 再遇同 ID 是 cycle，sibling exact
duplicate 已折叠，同 ID/different payload 报错；trust 取最弱 leaf。

`session_summary` 在 Phase A 必须 fail closed。writer event range 由
`(host_id,project_id,session_row_id,first_event_id,last_event_id)` 标识，但 schema
没有 summary 到完整 generated surfaces 的 immutable binding：
`topic_segments` 没有 `session_summary_id`，quarantined segment 也可能不持久化。
因此任何 status/ack 都不能证明原 writer scan input；每个 structured summary
ref 在 content/trust 使用前返回
`unverifiable_session_summary_provenance`。projection 不按 range 猜 segment、
不写状态；Phase C 加 immutable surface snapshot/FK 后才可启用。SELECT-only
fixtures 覆盖 missing/safe/quarantined/acknowledged。

## Phase A Hardening Design

### Subject identity and scope

- Memory identity 是
  `(Memory, canonical owner_scope, canonical owner_key,
  Some(normalized memory scope), memory_type,
  nonempty topic_key or memory:<id>)`。
- normalized memory scope 是
  `COALESCE(NULLIF(TRIM(scope), ''), 'project')`。只有 NULL 或 exact-empty
  `topic_key` 使用 `memory:<id>` singleton；所有 nonempty key（含纯空白）按
  bytes 原样成 slot。
- User claim identity 是
  `(UserContextClaim, exact owner_scope, exact owner_key, None,
  claim_type, claim_key)`。
- memory owner pair 必须完整。两者都缺失时才按既有 v019/default writer
  原子规则 fallback：global => `user/user:default`，其余 =>
  `repo/memory.project`。完整且 nonblank 的 pair 是 authoritative，即使
  scope-cleanup reroute 不改 `memories.project`；partial/blank pair contextual
  error。stored `owner_scope` 必须 exact 属于
  `user|workspace|repo|tool|domain|workstream|session`，owner key 必须
  trim-stable/nonblank；normalized memory scope 只能是 `project|global`。
  unknown domain 均 contextual error。memory scope trim 是 v2 intentional
  hardening：raw `" global "` canonicalize 为 global，绝不泄入 Project；不声称
  与 legacy untrimmed context SQL byte-equivalent。
- `branch=Some(B)` 读取 neutral + exact-B；`branch=None` 保持公开 v1 的
  branch-agnostic 全分支视图。Project/branch scope 外 claim 和 relation 不能
  影响结果。worktree/task selector 属 Phase B。
- Project(P,B) memory inclusion 先统一要求 normalized memory scope!=global，
  再满足 `(owner_scope='repo' AND (owner_key=P OR target_project=P))`，或
  `(owner_scope IS NULL AND owner_key IS NULL AND memories.project=P)`，最后
  应用 branch predicate。
  `source_project` 仅用于 evidence provenance。完整 repo owner 的 owner、
  target、placement 可以合法不同；owner 或 target 任一匹配即可。完整的
  repo-rerouted global 仍只由 Owner 读取。
  tool/domain/workspace/workstream/session/user owner 即使 stale placement、
  source 或 target 为 P 也不进入 Project，而由 exact Owner query 读取。
  canonical/legacy global 同样不进 Project，由
  `Owner(user,user:default)` 读取。legacy non-global 则同时可由 Project(P) 与
  `Owner(repo,P)` 读取。
- scoped validation probe 包含 placement、repo owner 或 target 引用 P 的 row；
  命中的 partial owner pair 必须 contextual error。full repo pair 的 blank target
  当作 absent，owner/target difference 合法。legacy 只有 owner pair 双 NULL 才
  原子 fallback；`target_project` 不扩大 legacy/non-repo Project inclusion。
- Owner(S,K) 纳入 canonical owner（含 atomic legacy fallback）exact 等于
  `(S,K)` 的全部 memories，以及 exact owner 的 user-context claims；不通过
  `target_project` 扩张。Owner 没有 branch selector，因此 memory 明确使用
  branch-agnostic all-branch semantics。Owner 不读取 Observation catalog，
  Phase A 也不把 Observation attachment 暗中带入 Owner memory。
- Project observation 优先用 `observations.project_id` join
  `projects.project_path` exact match；只有 project_id NULL 时才可 fallback 到
  非空 legacy `observations.project`，两者都有时必须一致，并应用同一 branch
  predicate。subject selector 与 scope 不一致 contextual error。
- explicit `as_of=t` 从持久 `memory_route_ledger` 恢复 Project/Owner membership 与 `SubjectIdentity`。逻辑列为 `id,memory_id,route_version,previous_route_id,effective_at_epoch,source_kind,audit_event_id(no FK),source_ref=pre-write request ID(or migration identity),source_fingerprint TEXT NOT NULL lowercase-64hex,coverage_kind/start_epoch`，完整 route snapshot 的文本均 TEXT/no-NUL、confidence numeric/null，raw nullable `topic_key` 保留 NULL≠empty；memory/self FK RESTRICT，version/predecessor/fingerprint unique。
- memory/time、owner、target、legacy placement 与 coverage indexes 支持先 UNION scope candidate、再按 ID chunk 读完整链；A→B→C 的 B 即使不在 creation/current route 仍可发现。
- foreground migration 先 snapshot/drop/recreate 外部 triggers；所有 preexisting memory-owned UPDATE effects（含 FTS/enrichment/version/archive/status）延迟到 A→B→C exact-match stored C/dependents 后安装。完整 history 用独立 baseline/step request 逐步 seal；不可证明 row 仅 `forward_only`。
- canonical insert entrypoint 在 mutation 前创建稳定 request intent；每个新 memory 带 `insert_writer_kind TEXT NOT NULL/insert_request_id TEXT NOT NULL/insert_result_ordinal INTEGER NOT NULL CHECK >=0`，三列 UNIQUE、composite request FK RESTRICT 且 update-abort trigger 保证 immutable（legacy backfill 用 deterministic migration identity）。cutover `AFTER INSERT` trigger 要求该 tuple 匹配 open intent，以 strict request hash、ordinal 与完整 typed `NEW` 立即建 v1，不依赖尚未产生的 final response/downstream IDs，并覆盖 store、lifecycle、candidate、CLI、Markdown、pack 六类。save upsert、Markdown restore/update、scope cleanup 共用 route-transition service，NULL-safe 比较实际 OLD/NEW placement/branch/scope/source/target/owner/type/raw-key/topic-domain/routing/context；真实变化在同 savepoint/transaction/epoch append+update，同值不写。normal-save selector 必须同 type，只允许 reachable raw-key transition；type change 仅 Markdown validated stable-`source_id` path。Markdown project→global 用 `markdown_import`，scope cleanup 同时写 mirror。
- guard 拒绝 changed tuple 的 missing/wrong-head/NEW-mismatch stage，任一步失败全 rollback，ledger update/delete 禁止。source kind closed 为 `insert|legacy_backfill|save_upsert|markdown_import|scope_cleanup`；diagnostic event 不参与 proof。fold `(effective_at_epoch,id)` 中 `epoch<=t` 的完整 route/identity；invalid scope、missing predecessor/source、gap/fork/time/terminal/coverage gap fail closed，合法 scope/identity transition 不报错。

### Observation evidence

- scoped Observation 的 `created_at_epoch=NULL` 是 contextual integrity error，
  因为 public knowledge-time 非 NULL；display `created_at` 不能作 canonical
  epoch fallback。它与 `reference_time_epoch=NULL` 分开处理，后者合法 fallback
  到 required creation epoch。
- `observations` 必须被 adapter 映射，不能只覆盖 lifecycle unit test。
  canonical ref 与 `EvidenceView.evidence_ref` 均为 `observation:<id>`，
  `kind=Observation`，`source_ref=observation:<id>`，
  `scope=TruthScope::Project { project, branch }`，
  `source_time_epoch=COALESCE(reference_time_epoch, created_at_epoch)`，
  `knowledge_time_epoch=created_at_epoch`。
- 正常 writer 可保留 `evidence_event_ids=NULL`，它表示 empty refs；非 NULL
  才必须严格解析为 JSON 正整数 array、排序、去重，并逐个验证存在、project
  相同和时间 eligible。Observation 任一 `host_id/project_id/session_row_id`
  非 NULL 时三者必须齐全、joined session exact，并要求每个 event exact 同三元；
  三者全 NULL 才是 legacy path；partial/dangling/cross-host/session 报错。结果作为
  `supporting_evidence_refs=["captured_event:<id>", ...]`。malformed、
  dangling、foreign-project ref 返回 contextual error。每个 event 的 source 与
  `inserted_at_epoch` 还必须不晚于 Observation `created_at_epoch` binding epoch
  和 query reference epoch；later-inserted future ID 是 integrity error，不能
  retroactive support。
- Observation trust 是
  `min(ModelGenerated, weakest canonical supporting-event trust)`，无 refs 时
  为 ModelGenerated；external-backed row 必须是 Untrusted，不能被抬级。
- historical active row 没有 scan-version，不能直接标 Validated。adapter
  必须用当前 generated-surface scanner 重扫 title/subtitle/narrative/facts/
  concepts；clean 才是 Validated，命中则返回 contextual poisoning error 且不
  产生成功 projection。stored `poisoning_quarantined` 是 Quarantined policy
  state，在 public output/trust 前过滤且不重新暴露内容。
- `active` observation 的 lifecycle 是
  `(Active, Current, Live, Visible)`；`stale` 不进入 usable current catalog；
  `compressed` 是 Archived，也不进入 current catalog。
  explicit history 若看到 cutoff 前已存在的 scoped stale/compressed row，必须
  验证完整 transition history，否则返回
  `unreconstructable_observation_lifecycle`。现有 compression snapshot 不含 prior
  status，stale mutation 也无 timestamp，不能充当该证明。
  `poisoning_quarantined` 必须映射为
  `(Candidate, Unknown, Live, Suppressed)`，integrity 为 Quarantined，并在
  public `evidence_catalog`、claim attachment、trust aggregation 和 current
  truth 之前过滤。其他 unknown observation status 返回含 table/ref/raw value
  的 contextual error。
- Observation 从不进入 `ClaimSource`。唯一允许的 claim attachment 是
  `memory_facts` row 同时带
  `source_memory_id=<memory claim id>` 与
  `source_observation_id=<observation id>`；两端必须已在 scoped/time-eligible
  set，fact project、memory placement 与 canonical observation project exact
  相同。
  方向固定为 observation evidence supports memory claim。共享 captured event、
  相同 topic/text 或模型相似度都不是 link。User claim 在 Phase A 没有
  observation attachment。
  Link at `t` 复用 `memory::facts::as_of_validity_filter_sql`：
  `learned_at_epoch <= t`、实际插入 `created_at_epoch <= t`、
  `valid_from IS NULL OR <=t`、
  `invalidated_at IS NULL OR >t` 与 half-open `valid_to >t`；只有既有 helper
  的“invalidation/replacement 尚未被 t 时点获知”条件可以保留旧 link。
  real schema 中 `created_at_epoch` NOT NULL；NULL/missing legacy 字段不能用
  learned/updated/display time fallback。replacement 也必须已 learned/inserted。
  验证 status/predicate/link/endpoints，并覆盖 learned/created/invalidation/
  replacement boundaries 与 late-insert/backdated-learned regression。

### Evidence trust and provenance

- captured-event trust 必须复用
  `src/memory/poisoning.rs::SourceTrustClass` 的 canonical 分类，不得使用
  “存在 tool_name 即 Verified”的简化规则。truth 先重建 full content：
  `raw_keep` exact 为 `full_content_byte_length<=16384`、无 blob、完整 `content_text` 与
  canonical SHA-256 event hash；`raw_compact` 为 `full_content_byte_length>16384`、plain UTF-8 blob、
  两项 byte counts 等于 blob length、canonical preview/event SHA-256，以及
  matching SHA-256 或 exact 16-hex legacy blob hash。dangling/crossed storage、
  encoding/length/preview/hash mismatch fail closed。只可暴露 capture constant/
  pure preview helper 与 poisoning pure classifier；除 duplicate timestamp
  guard 外不改变 writer。测试锁定
  16384/16385、multibyte boundary 及只出现在 compacted middle 的 network marker。
- `user_prompt`、`repo_file`、`local_tool_output` 的 cap 是 Verified；
  `pack`、`external_content` 的 cap 是 Untrusted。WebFetch、WebSearch、
  `mcp__*`、抓取外部 URL 的 Bash 与 session-stop 均按 canonical classifier
  归入 `external_content`。unknown stored class 返回 contextual error。
- 每条 memory 先解析 stored class，再用当前 canonical classifier 重新分类
  每一个 referenced event。`effective_source_cap` 是 stored cap 与所有 event
  caps 中的最弱者；无 event 时只用 stored cap。`strongest_evidence` 只对
  eligible、validated、非 SourceTrustClass evidence 取 max，无 evidence 时是
  ModelGenerated。最终
  `effective_claim_trust=min(strongest_evidence,effective_source_cap)`。
  SourceTrustClass view 只作 diagnostic/cap provenance，不参加 max；source cap
  只限制、不能凭自身把无 verified evidence 的 claim 抬级。
- candidate-backed memory 的 `effective_source_cap` 还必须包含 validated
  candidate stored cap；后续 memory trust rewrite 不能提权。
- `external_content`/`pack` memory 即使同时引用一个被旧 adapter 视为 tool
  output 的 event，也保持 Untrusted。即使 v060 legacy/default stored class 是
  `local_tool_output`，WebFetch/MCP/network-Bash ref 的 recomputed weakest cap
  仍使 mixed evidence 为 Untrusted。
- captured event 必须 join `projects.project_path`。expected source project
  优先使用非空 `memory.source_project`；只有没有 routing assertion、owner
  legacy pair 完整可推导时才 fallback `memory.project`。missing、foreign、
  ambiguous 或 routed-but-source-missing 均 contextual error。
- malformed `memories.evidence_event_ids`、user `source_refs_json` 或 dangling
  event ref 不得静默成功。非空 `source_candidate_id` 必须解析
  `auto_promoted|approved|edited` candidate，evidence/content/confidence exact。
  route-ledger initial state（不是 today row）必须 exact candidate/result owner/
  project/scope/type/raw key，scope 等于 validated `CandidateRoute::memory_scope()`：
  user owner 为 global，其他为 project；candidate scope/derived title 非 copied field。
  `memory_operation_log(source='memory_candidate',source_candidate_id=candidate.id,
  result_memory_id=memory.id)` 绑定 initial state；workspace/pack fixture 锁定规则。
  candidate/result 只对 completion 的 initial v1 identity 校验一次；cutoff fold 完整
  chain 后用该 version 计算 membership 与 emitted `SubjectIdentity`，不得把 immutable
  candidate identity 再与 cutoff/today state 比较。合法 owner/project/scope/type/
  raw-key transition 可通过；current 还要求 terminal full tuple=current。
  chain/coverage/terminal error 用 `unreconstructable_routing_history`，其他 content/
  provenance drift 用 `unverifiable_post_candidate_mutation`。refs 绑定 candidate
  creation，completion/cleanup knowledge 独立要求 reference-eligible。无 candidate
  时 compatible result operation 绑定 refs；两者都没有时 `as_of=None` 以
  `reference_epoch` 绑定 current refs，explicit historical 排除/Unknown。
  malformed claimed link 报错；event source/insert 不得晚于 binding/reference。

### Temporal reconstruction

- 所有 Phase A 读取共享 projection 的 `reference_epoch` 与 SQLite snapshot。
- `effective_memory_knowledge_epoch` 统一用于 memory ClaimView、SourceRef 与
  SourceTrustClass。proof 是 validated candidate completion+route，或
  `memory-operation-planner-v1 add|update|conflict` exact result 与 operation
  epoch 的 route/identity；later legal route 不抹除 proof。canonical noop 要求
  planner/result 与 noop epoch identity、
  empty transitions、`noop_reason='already represented by active memory'`，以及
  `direct/save_memory/NULL` 或 exact candidate source + matching noop candidate；
  input/result topic 可不同。noop 只证明 transition。epoch 是 earliest
  ingestion proof 与 eligible noop/memory update/candidate completion-or-ack/
  route-ledger transition/complete current ack 的 max；partial/stale ack 报错。无 proof 的
  history 排除/Unknown，current 用 reference epoch。source time 仍为
  `COALESCE(reference_time_epoch,created_at_epoch)` 且不能 future；direct noop、
  governance ack、candidate ack 分别用 operation/memory/candidate update。
- Explicit history 只读 `memory_lifecycle_ledger(id,memory_id,lifecycle_version,previous_lifecycle_id,effective_at_epoch,previous_status,new_status,source_kind/action,source_operation_id,audit_event_id(no FK),source_fingerprint TEXT NOT NULL CHECK(typeof(source_fingerprint)='text' AND length(source_fingerprint)=64 AND source_fingerprint NOT GLOB '*[^0-9a-f]*'),coverage_kind/start_epoch)`；memory/self FK RESTRICT，version/predecessor 与 `(memory_id,source_kind,source_fingerprint)` unique（无 NULL bypass），indexes 为 memory/time、coverage/memory 与 partial unique operation。
- source kind closed 为 `insert|legacy_backfill|memory_governance|web_governance|scope_cleanup|writer_transition`；writer_transition 必须 new≠previous，actions 覆盖 save/Markdown/candidate/TTL/soft-supersede/preference-removal/stale-archive exact transition。baseline predecessor/status NULL；migration 只复制 exhaustive proof。
- 所有 production `memories.status` writer 均经 canonical lifecycle service 原子更新 status+append，对称 `BEFORE UPDATE OF status` guard 要求 open exact staged successor；任何 v2 startup 不得启用 uninstrumented writer。链 previous=prior new、terminal=current，按 `(epoch,id)` fold且 equality 用 new；gap/fork/drift 均 fail closed。
- 两 ledger indefinite retention、无 events FK/cascade、排除于 event cleanup；memory/self RESTRICT。未来 purge 需 reviewed tombstone migration；cleanup regression 锁定两 ledger/Web proof/output、零 delete 与 `foreign_key_check`。
- 两 ledger 的 `source_fingerprint` 是 ordered binary frame 的 lowercase SHA-256：field-name length+bytes、type tag、value length+bytes；integer signed big-endian、real IEEE-754 big-endian、string 默认 exact raw UTF-8、NULL≠empty。输入为 schema/ledger version、memory/source/action、predecessor ID/version、stable request ID、strict request fingerprint、result ordinal 与完整 typed OLD/NEW；不含 INSERT trigger 执行时尚不存在的 request-wide result fingerprint、response 或 downstream IDs，后者只进 final commit seal。不存在通用 CRLF/trim：仅 production writer 真正 canonicalize 的字段使用其 documented canonical bytes；raw caller bytes 与 derived bytes 都影响行为时分别入 frame。ordered array 保留 order/duplicate，只有声明为 set 的字段 bytewise sort/dedupe。

| Writer | Canonical request discriminator |
| --- | --- |
| insert / legacy backfill | pre-write request/operation ID+完整 canonical insert request / migration version+memory ID+baseline/step ordinal；每个 proved successor 使用独立 request 并在下一步前 seal |
| save / Markdown | pre-write operation ID+`src/memory/service/types.rs::SaveMemoryRequest` 全部 raw values：`text,title,project,session_id,host,topic_key,memory_type,files,scope,created_at_epoch,branch,local_path,local_copy_enabled,claim_enabled,claim_source,acknowledge_pattern`+另传 raw `reference_time_epoch`（含 Option presence、files order/duplicate 与 raw CR/LF/outer whitespace）+adapter raw envelope/另行 framed validated/defaulted values；result 是 exact final `SaveMemoryResult`/serialized response、memory/operation/route/lifecycle/claim IDs/rows、poisoning ack fields、local-copy status/path/reason/content digest、claim status/id/error、next-step fields / Markdown 用 stable source binding(`source_id`+creation/reference，prior source hash 仅 lookup precondition)或 no-source identity(export version+archive-root-relative POSIX path，移除 lexical dots/拒绝 parent escape+synthesized persisted topic)+post-render semantic frame；排除 importer-owned metadata，但 byte-preserved parsed fields 必须 exact，只有 parser 实际 canonicalize 的字段用其 documented bytes |
| general / Web governance | action+actor+normalized reason+acknowledgment pattern+sorted target set / durable operation idempotency identity+canonical request hash |
| scope reroute / archive / cleanup | action+object ref+normalized owner/target/topic/routing/context/reason / action+object ref+normalized reason / planner version+canonical plan/group snapshot hash |
- executable DDL 以 CUTOVER 为准：step2 exact pure rebuild 并在 write lock 下重验；route typed；seal exact-match API full snapshot 且 `response_aux=response_json`；route successor 必须真实变化。
- 所有 writer 在 mutation 前取得 opaque request ID/hash；intent→typed results→exact response seal。Direct save 先 request lock；启用 local copy 再按 canonical target digest 取 target lock并 fsync owner，固定 L→LT 顺序且持有至 postcommit cleanup/owner removal；same-target different-R 先 reconcile owner，distinct targets 可并行。Windows v2 plan/apply 零副作用 typed 拒绝并继续 v1。
- UserContextClaim source 是 `COALESCE(valid_from_epoch,created_at_epoch)`；
  descendants 保留 provenance-root SourceRefs，transition 只改 state knowledge。
- Captured event 的 source 与 original insertion 都须 reference-eligible。
  duplicate `(host_id,session_id,event_id)` 保留 payload/所有 clocks，只追加 keyed
  Git evidence/work；pre-v2 insertion 是 conservative floor。
- `edit_claim` 保留旧 row 并插入 successor。transition 前恢复旧 row，equality
  使用新 row；predecessor 用 creation knowledge，retained rejected predecessor
  用 transition knowledge。mutated update 只是 boundary，不重绑 SourceRef。
  missing/forked/cross-owner/timestamp-inconsistent chain 报错。
- Candidate apply 的 `auto_promoted|approved|edited` row 要求 exact owner/type/key
  result 与所有 predecessor 共用 transition epoch。Replacement 在 equality 用
  result 替代 ordered explicit predecessor 及同 epoch co-predecessors；no-op 保留
  ordered exact-match result/SourceRefs 并恢复 transition 前其他 rows。unlinked
  Superseded 仅在完整 authoritative candidate/result/timestamp pattern 下合法。
- 非 governance/versioned 的原地 suppress/unsuppress/delete 若 post-cutoff，
  保守排除/Unknown；current ClaimView 可用 update knowledge，SourceRefs 不重绑。
  一般 hard delete/content rewrite 无 history 时也不根据 current bytes猜过去。
- Claim、Relation、Fact 的 event-validity window 全部 half-open：
  `valid_from <= t`、`valid_to > t`；valid_to equality 已失效。source/knowledge
  equality eligible，user-edit successor 在 transition equality 生效；
  suppression revocation equality 按独立 policy 恢复可见。

### Policy suppression

- Canonical stored matrix 固定：`memory`/`user_claim`/`user_candidate` 只能用
  positive ID；`topic_key`/`entity`/`pattern` 只能用 trim-stable nonblank
  value；`summary` 必须在 positive ID 与 trim-stable nonblank value 中恰选一个。
  extra/both/missing 字段均报错。ID 分别解析到 `memories`、
  `user_context_claims`、`user_context_candidates` 与
  `user_context_summaries`，不能把后两者错绑为 memory candidate 或
  `session_summaries`。
- topic 是 `memories.topic_key` byte-exact；entity 是 linked
  `entities.canonical_name` 的 SQLite `lower()` equality；pattern 是 SQLite
  `instr(lower(field),lower(value))>0`，field 为 memory title/content 或
  user-claim text/key。summary value 按 production 对
  `user_context_summaries` 的 decimal ID equality 或 summary_text
  case-insensitive substring。
- memory/topic/entity/pattern/user-claim 命中只改 Visibility，不改 validity。
  `user_candidate` 与 `summary` 在 Phase A recognized-but-non-applicable；验证
  后不产生 effect，也不 transitively suppress candidate-derived claim、
  `session_summaries` SourceRef 或 evidence。unknown kind 报错。
- owner pair `(NULL,NULL)` 是 global；两个字段都非空时仅对 exact
  `SubjectIdentity.owner_scope/owner_key` 生效；partial pair contextual error。
  exact-owner direct ID 必须校验 target row 属于该 canonical owner，
  value 也只在该 owner 内匹配；global `(NULL,NULL)` direct ID 可命中任意 owner
  的 exact row，value 不加 owner 限制，但 query scope 仍限制可隐藏 claims。
  missing direct target 或 owner mismatch 报错。
- Current query 只应用 active row。Historical `as_of=t` 应用：

```text
created_at_epoch <= t
AND (
  status = 'active'
  OR (status = 'revoked' AND t < updated_at_epoch)
)
```

  因此 revocation equality 已恢复可见。unknown suppression status/target kind、
  同时缺少 target ID/value 或同时填写不合法组合均 contextual error。测试覆盖
  全部七种 canonical target，包括两种 recognized non-applicable target。
  `memory_entities` 无 link time：applicable current entity target 使 projection
  `CurrentSnapshotOnly`；effective historical entity target 返回
  `unreconstructable_entity_link_history`，除非 durable history 证明全部 scoped
  membership/non-membership。entity creation/backfill/current join 都不是证明。

### Relations and resolution

- `memory_edges` closed domain/mapping 是：
  `supersedes→Supersedes(stored old→new, DTO new→old)`；
  `duplicates→Supports(from→to)`；`conflicts→Refutes(from→to)`；
  `derived_from|merged_into|split_from→DerivedFrom(to→from)`。bounded query
  对每个 touching scoped ID 的 row 先 parse raw kind 再过滤 endpoints。
  unknown/newer/typo（含 graph-only `extracted_from`）返回
  table/edge-ID/raw-value context。known NULL-source candidate `derived_from`
  只验证 provenance，不能伪造 Claim endpoint。
- emitted relation 两端都在 scoped set。Supersedes/ordinary Refutes 仅在 exact
  identity 内裁决；cross-subject Supports/DerivedFrom 只作 winner provenance。
- 唯一 cross-identity decision exception 是 operation-backed preference
  conflict：两端同 owner/scope/normalized branch、type=preference 且分别 surviving；
  post-pass 标两个 Contradicted outputs，不合并 identity。
- operation-backed conflict 验证 operation kind、integer `conflicting_ids`、
  replacement/pairwise membership 与 source/candidate/edge linkage；错误含
  edge/operation/endpoints/field context。canonical graph/dream heterogeneous
  pair 可用 writer fallback owner/type，结构有效时 decision-neutral。
- uniform-conflict graph 折叠 parallel same-pair 后必须是 matching；A-B+A-C
  报错。approved pair outputs 保留各 subject、`claim=None`、Contradicted、
  sorted/dedup evidence、validated contradiction set、per-slot rejected refs、
  both conflicting refs 与 `UnresolvedConflict`；两个 outputs 共享 relation set。
- unbacked 要求两个 source IDs 都 NULL，以 edge creation 为 knowledge 且
  decision-neutral；candidate-only 报错，operation-only 合法。operation creation
  `<=edge.created_at_epoch` 且 eligible。claimed candidate 匹配 discriminator/ID，
  creation `<=operation`，并证明 memory status/result/operation/endpoints 或 graph
  status/promoted-edge/operation；relation knowledge 是 edge/operation creation 与
  validated application update 的 max，可晚于 edge。所有 clock eligible；
  dangling/future/mismatch 与 application boundaries 有 fixtures。
- Resolver 固定为 scope/time/lifecycle → exact supersedes → exact refutes →
  evidence trust → recency → preference post-pass；confidence 不参与。

### Bounded read behavior

- claims、route/lifecycle coverage/discovery/history、raw edges、events、facts、
  suppressions 全部以 scoped IDs/owner/project/named index
  查询，stable ascending bind chunk `<=900`，不允许无关 table scan 后
  `Vec::contains`。第一条 claim SELECT 到 resolve 共享一个 snapshot。
- seed-933：target 901 memories、1,802 relations、901 evidence refs、900-link
  high fanout；unrelated 4,505/9,010/4,505，加入后不得改变 target output/counts。
- Structural pass conditions：
  - authorizer 只允许 read/SELECT 与 owned transaction controls，`total_changes=0`；
  - indexed data plan、bind `<=900`、unrelated materialized/returned rows 为 0；
  - data statements `<=12 + 5*ceil(scoped_claims/900) +
    2*ceil(scoped_evidence_refs/900)`；
  - transaction-control count 在 autocommit 为 2（BEGIN + COMMIT/ROLLBACK），在
    caller transaction 为 0，并与 data statements 分开记录。
- `src/truth/tests/performance.rs` 提供 ignored release-mode recorder。命令：

```bash
GH933_PERF_JSON_OUT=/tmp/gh933-truth-perf-v2.json \
  cargo test truth_performance_contract --release -- --ignored --nocapture
```

  固定 5 次 warm-up、50 次 measured runs。JSON 必须含
  `schema_version=1`、exact head SHA、seed/corpus counts、chunk size、SQLite/
  Rust versions、query plans、data/transaction statement、bind/row counts、
  serialized bytes、
  p50/p95、migration/index/truth/dependency fingerprints 和每项 structural
  check boolean。Rust test 自身验证必填字段与 structural checks；p50/p95 仅记录，
  本 Phase A 未建立可跨机器比较的 latency hard budget，不能把任意数字称为 pass。
  final candidate SHA 的 artifact 只能在最后一次 commit/base sync 后生成。

## Product-to-Test Mapping

| Invariant | Verification |
| --- | --- |
| CT-001 | typed identity、exact selector、stable serde/order、v2 golden、effective reference epoch |
| CT-002 | 所有已知 lifecycle values；quarantined observation 显式 Suppressed |
| CT-003 | current + before/equal/after route Project/Owner inclusion、incomplete-route error、Owner union、branch、relation scope |
| CT-004 | memory/event time、capture immutability、candidate/route、durable governance/Web lifecycle、edit/in-place mutation |
| CT-005 | exact-identity supersedes beats recency |
| CT-006 | full-blob canonical classifier、provenance-root/binding checks、total recursive user source grammar、candidate own-result/edit invariants、summary provenance fail-closed、WebFetch/MCP/Bash-network、pack/external cap、no-uplift/unknown class |
| CT-007 | six-kind memory-edge mapping/unknown error、same-slot refutes、preference post-pass、overlap/operation errors |
| CT-008 | empty/stale-only abstention；malformed/dangling/unknown fail closed |
| CT-009 | Observation DTO/catalog/trust/attachment；fact insertion clock；stale/compressed history error |
| CT-010 | ClaimSource 仅 Memory/UserContextClaim；Observation 只作 evidence |
| CT-011 | raw status；七种 suppression/owner/time；entity current-only/history error |
| CT-012 | Archived 不进 current truth/catalog；后续 historical explanation 单独设计 |
| CT-013 | Phase B context load/render/error/rollback；本 PR 不声称完成 |
| CT-014 | Phase C benchmark/architecture decision；本 PR 不声称完成 |
| CT-015 | one read snapshot；authorizer/transaction controls/`total_changes=0`；public API |

## Compatibility and Golden Diff

- v1 是公开 API。v2 在 `0.7.0` 或实现时下一个 breaking SemVer boundary 发布；
  Cargo、lockfile、plugin manifest、runtime release manifest、npm wrapper、
  `server.json` 与 changelog 同步。
- README 和 `docs/ARCHITECTURE.md` 必须给出 `remem::truth` v1→v2 migration：
  `subject_key/scope/as_of_epoch` 到 typed subject/TruthScope/
  `requested_as_of_epoch+reference_epoch`，ClaimView
  `created_at_epoch/updated_at_epoch` 到 version-specific
  `source_time_epoch/knowledge_time_epoch`（含 edit transition、in-place mutation
  限制与 recency 使用 selected ClaimView effective knowledge epoch），以及
  observation catalog。
- `tests/truth_public_api.rs` 必须作为 external crate consumer 编译
  `use remem::truth::{...}`，构造 Project 与 Owner query，并锁定 public exports。
  文档与编译示例只使用这一真实 library path。
- Golden diff 逐字段 allowlist 仅包括：
  1. version 1→2、public entrypoint/export、TruthScope、typed subject、exact selector、
     effective epoch/replayability；
  2. EvidenceView/ClaimView temporal fields、Observation catalog/read-scan/trust/
     attachment 与 fact actual `created_at_epoch` eligibility gate；
  3. identity isolation、canonical Project inclusion、indexed historical
     route/backfill（含 intermediate route 与 forward-only fail-closed）、Owner
     union、global/legacy fallback 与 compatibility wrapper；
  4. versioned edit/candidate reconstruction、durable general/Web/scope-cleanup
     lifecycle recovery、event-cleanup invariance 与 unsupported mutation handling；
  5. policy suppression owner/time visibility；
  6. canonical stored+recomputed source-trust cap、all-source binding-time
     checks、first-party explicit-user rules、candidate/result/edit invariants、
     summary provenance fail-closed 与 full-blob external/tool提权修复；
  7. valid heterogeneous conflict 由 error 改为 neutral；
  8. six-kind memory-edge total mapping、approved preference outputs 与 overlap error；
  9. malformed/dangling/unknown provenance/status 的 fail-closed/error output。
  branch semantics、same-slot resolver order和其它 contract-valid output 不变。
  禁止整份重录 golden 掩盖 allowlist 外 drift。

## Security

- 所有 SQL 使用 bound parameters；project、branch、owner、pattern 和 user content
  不得拼接。
- external content trust cap、poisoning quarantine、suppression owner scope 和
  relation endpoint scope 都是 security boundary，必须有人审阅实现。
- Future HTTP/MCP surface 需要独立 auth/redaction/sensitivity review；Phase A
  不能直接把完整 statement/catalog 序列化到网络。
- Projection data-SELECT-only；允许 owned transaction control，不执行 canonical
  write/migration/LLM/network/process。失败返回 diagnostic，不能包装为空成功。

## Phase B / Phase C Boundaries

- Phase B 才让 Context Bundle 使用 projection。一次 render 只计算一个
  reference epoch；current truth、decisions、conflicts 分开渲染；旧 path 有明确
  rollback；projection failure 必须 error-visible。worktree/task selector、
  budget/cache/historical explanation 也在 Phase B 细化。
- Phase C 评估 narrow Phase A history substrate 之外的一般 Claim-writer
  convergence；是否收敛由 benchmark/架构审阅决定，其他 migration、cutover 与
  generated-enrichment firewall 需要独立 contract。

## Test Plan

- [ ] `cargo test truth -- --nocapture`
- [ ] `cargo test --test truth_public_api`
- [ ] `cargo test context`
- [ ] `cargo fmt --check`
- [ ] `cargo check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test`
- [ ] `python3 scripts/ci/check_plugin_version_sync.py`
- [ ] `python3 scripts/ci/check_version_bump.py origin/main HEAD`
- [ ] Full PR preflight with the exact intended PR body.
- [ ] Final-head bounded/performance artifact command above.
- [ ] 六类 route INSERT 的 pre-write ID/cross-memory winner、三类 UPDATE、
      normal-save same-type raw-key 与 stable-source-ID Markdown project→global/
      type/key before/equal/after、candidate initial binding+cutoff membership/subject、
      metadata-rewrite-stable retry/full-content save/strict fingerprint DDL、
      `markdown_import`、rollback/gap；all-writer fingerprint/crash/retry、
      backfill/forward-only、durable lifecycle、event-cleanup invariance、
      fact created-at、six edge mappings、unknown/unsupported fail-closed。
- [ ] Fresh exact-head CI and independent review.

## Rollback

Projection 仍 data-SELECT-only，但 Phase A 新增 reviewed route/lifecycle ledgers、
backfill/writer instrumentation 与 route guards。回滚可停用 v2 consumer，但保留 history；
不得 drop history。恢复 pre-migration DB 需停写并证明不丢 backup 后写入；0.6.x 不能打开 newer schema。

Phase B 保留旧 context path 并按独立 rollout/rollback design 切换；projection
失败不得静默输出缺失 context。Phase C 如果引入 migration/dual-write，必须在
独立 contract 中给出停止写入、向后兼容读取、数据校验与恢复步骤。

## Implementation Change Set

Phase A v2 implementation 预计修改以下路径；除 reviewed history substrate 与
duplicate timestamp guard 外，不得静默扩大到一般 writer 或 context：

- truth：`src/truth.rs`、`src/truth/{adapter,lifecycle,projection,types}.rs`、
  `src/truth/tests.rs`（先拆分）、`src/truth/tests/**`、`tests/truth_public_api.rs`
- history schema/backfill：`src/migrations/vNNN_current_truth_history_ledgers.sql`、
  `src/migrate/run.rs`、focused helper/schema/migration tests
- route writers：`src/memory/store/write.rs`、`src/cli/actions/markdown_archive.rs`、
  `src/memory/scope_cleanup/mutate.rs`、`src/memory/service/{types,save}.rs`、
  API/MCP save request adapters 与 route-mutating eval/test fixtures
- lifecycle/Web writers：`src/memory/governance.rs`、
  `src/memory/scope_cleanup/{mutate,plan}.rs`、focused API/cleanup regressions
- trust/capture：`src/db/capture.rs`（duplicate timestamp no-op + preview helper）、
  `src/memory/poisoning.rs`（只暴露 pure classifier）
- docs：`README.md`、`docs/ARCHITECTURE.md`、`docs/specs/{README.md,GH933/**}`、
  `CHANGELOG.md`
- release：`Cargo.{toml,lock}`、plugin/runtime/npm manifests、`server.json`

如果实现需要上述 route/lifecycle contract 以外的新 schema/writer、Context Bundle 或 public network surface，停止 implementation，先更新 contract/review。
