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
| Adapter | 读取 memories、captured events、memory/graph edges、user claims；不读取 observation evidence catalog 或 policy suppression | 补齐 issue 明确要求的 observation adapter 与 suppression policy |
| Trust | 任意 tool event 会被标为 Verified，resolver 对 evidence 取 max | external tool output 可被提权；v2 必须复用 canonical source classification 与 cap |
| Observation status | writer 可写 `poisoning_quarantined`，lifecycle mapper 尚未列出 | quarantined prompt-injection 内容必须被显式 suppress，不能进入 catalog/truth |
| Temporal history | user claim edit 生成版本链；suppress/unsuppress/delete 原地 mutation | edit 可按 transition 重建；原地 mutation 只能保守排除/Unknown |
| Relations | loader 全扫 edge table 后在 Rust 过滤；canonical heterogeneous pairwise conflict 使用 fallback operation metadata | relation lookup 必须 bounded；合法 heterogeneous operation 不得误报损坏 |
| Suppression | `memory_suppressions` 有 owner pair 和 active/revoked 时间字段；production filter 忽略 owner | truth adapter 必须定义 owner-safe、可历史重放的 visibility 语义 |
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

`as_of_epoch=Some(t)` 时 `requested_as_of_epoch=Some(t)` 且
`reference_epoch=t`、`replayability=exact`。`None` 时入口只取一次 wall-clock
epoch；若输出依赖无 durable proof 的 current binding，则
`replayability=current_snapshot_only`，该 epoch 仅可审计、不能作为 replay
key；否则为 `exact`。整个 adapter/resolver 使用同一值。

输出排序固定为：

1. `truths` 按 `SubjectIdentity` 字段的派生 lexicographic order；
2. 每个 claim 的 evidence 按 `(source_time_epoch, knowledge_time_epoch,
   evidence_ref)` 升序，其中 `None` 早于任意 `Some`；
3. `evidence_catalog` 以同一 evidence key 排序，并按 `evidence_ref` 去重；
4. relations 按 `(created_at_epoch, relation_ref)`，canonical refs 按字节序。

各 Evidence kind 的 v2 field semantics：

| Kind | scope | lifecycle | source/knowledge time | integrity | supporting refs |
| --- | --- | --- | --- | --- | --- |
| CapturedEvent | canonical event project，branch=`None` | `None` | event reference/created；`inserted_at_epoch` | Validated | empty |
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
  `learned_at_epoch <= t`、`valid_from IS NULL OR <=t`、
  `invalidated_at IS NULL OR >t` 与 half-open `valid_to >t`；只有既有 helper
  的“invalidation/replacement 尚未被 t 时点获知”条件可以保留旧 link。
  known fact status/predicate、replacement linkage 和 endpoints 都要验证，并
  覆盖 learned/invalidation/replacement before/equal/after。

### Evidence trust and provenance

- captured-event trust 必须复用
  `src/memory/poisoning.rs::SourceTrustClass` 的 canonical 分类，不得使用
  “存在 tool_name 即 Verified”的简化规则。truth 先重建 full content：
  `raw_keep` exact 为 `full_content_byte_length<=16384`、无 blob、完整 `content_text` 与
  canonical SHA-256 event hash；`raw_compact` 为 `full_content_byte_length>16384`、plain UTF-8 blob、
  两项 byte counts 等于 blob length、canonical preview/event SHA-256，以及
  matching SHA-256 或 exact 16-hex legacy blob hash。dangling/crossed storage、
  encoding/length/preview/hash mismatch fail closed。只可暴露 capture constant/
  pure preview helper 与 poisoning pure classifier，不改变 writer。测试锁定
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
  event ref 不得静默成功。非空 `source_candidate_id` 是 claimed binding：
  candidate 必须是 `auto_promoted|approved|edited`，candidate/memory
  evidence、content/type/topic/confidence exact，candidate scope 是 validated
  input。completion memory scope 必须等于 validated route 的
  `CandidateRoute::memory_scope()`：user owner 为 global，其他为 project；
  candidate scope 与 derived title 都不是 copied-equality fields。必须有
  `memory_operation_log(source='memory_candidate',source_candidate_id=candidate.id,
  result_memory_id=memory.id)` completion；workspace/pack positive fixtures
  锁定 scope mapping/title exclusion。owner/project route 只可通过
  contiguous/unambiguous `scope_cleanup` events 的
  `previous_owner→new_owner` chain 改变并结束于 current route；其他 immutable/
  routing drift 返回 `unverifiable_post_candidate_mutation`。refs 绑定 candidate
  creation，completion/cleanup knowledge 独立要求 reference-eligible。无 candidate
  时 compatible result operation 绑定 refs；两者都没有时 `as_of=None` 以
  `reference_epoch` 绑定 current refs，explicit historical 排除/Unknown。
  malformed claimed link 报错；event source/insert 不得晚于 binding/reference。

### Temporal reconstruction

- 所有 Phase A 读取共享 `CurrentTruthProjection.reference_epoch`。
- `effective_memory_knowledge_epoch` 统一用于 memory ClaimView、SourceRef
  referent 与 SourceTrustClass。proof 是 validated candidate completion（含 route
  chain），或 `memory-operation-planner-v1` 的 `add|update|conflict` result row；
  后者须 result ID/current canonical owner/type/topic exact，historical mismatch/
  其他 planner 是 non-proof。已有 ingestion proof 后，canonical `noop` 必须
  planner/result ID/current owner/type、empty transition sets、
  `noop_reason='already represented by active memory'`
  及 source tuple `direct/save_memory/NULL` 或
  `memory_candidate/memory_candidate` + matching noop candidate；input topic
  可与 result topic 不同。
  它只作 transition proof，不能独立证明 ingestion。epoch 取 earliest proof、
  eligible noops、memory update、candidate completion/ack、route-chain events 与
  validated
  complete/current memory ack 的 max，partial/stale ack 报错。无 proof 时
  historical 排除/Unknown，current 用
  `reference_epoch`。source time 仍是
  `COALESCE(reference_time_epoch,created_at_epoch)`，future raw time 报错。
  direct-save noop 以 operation timestamp 绑定同 transaction trust/ack rewrite；
  governance ack 用 memory update，candidate ack 用 candidate update。
- UserContextClaim source epoch 是
  `COALESCE(valid_from_epoch,created_at_epoch)`；edited descendant 的 SourceRef
  仍用上文 provenance-root binding，transition 只改变 ClaimView state
  knowledge，不能重新附着 inherited refs。
- Captured event 必须同时满足
  `COALESCE(reference_time_epoch, created_at_epoch) <= reference_epoch` 与
  `inserted_at_epoch <= reference_epoch`；equality 可用。
- User claim `edit_claim` 是版本化例外：旧 row 在 transition epoch 标为
  superseded，新 row 同时插入并以 `supersedes_claim_id` 指向旧 row。
  `reference_epoch < transition` 恢复旧 row；等于或晚于 transition 使用新 row。
  historical predecessor 的 ClaimView 使用 pre-transition lifecycle 和
  `knowledge_time_epoch=created_at_epoch`；transition equality 后 successor 使用
  自己的 source/creation time。若 predecessor 作为 rejected provenance 保留，
  它使用 transition knowledge epoch 与 Superseded lifecycle。predecessor 的
  mutated `updated_at_epoch` 是 edit boundary，不是 immutable SourceRef knowledge
  time。missing、forked、cross-owner 或 timestamp-inconsistent chain contextual
  error。
- Candidate apply 是另一种 canonical multi-row transition。applied candidate
  status 只能是 `auto_promoted`、`approved` 或 `edited`，必须有 exact
  owner/type/key 的 `result_claim_id`，且 candidate update 与每个 changed
  predecessor update 使用同一 transition epoch。
  - Replacement：active rows 按 `(updated_at_epoch DESC,id DESC)`；first 必须是
    result 的 `supersedes_claim_id`，同 epoch Superseded 的其他 same-identity
    rows 也是 co-predecessors。transition 前恢复，equality 时 result 替代。
  - No-op：`result_claim_id` 指向 pre-existing、text/sensitivity exact match 且
    在同一 ordering 中最先的 row；它保持不变，其他 active rows 被 Superseded。
    transition 前恢复它们，equality 时 kept result current；candidate 不替换其
    SourceRefs。
  - unlinked Superseded row 只有在 authoritative candidate/result/timestamp
    pattern 全部验证通过时才合法；否则 historical state 不可重建，返回 contextual
    integrity error，不能 silent drop 或猜 predecessor。
- `suppress`、`unsuppress`、`delete` 是原地 mutation。若
  `updated_at_epoch > reference_epoch` 且没有独立版本 row，保守排除/Unknown，
  不把 current status/content 回灌历史。查询在 mutation 时点或之后可以让当前
  ClaimView 使用 `knowledge_time_epoch=updated_at_epoch`；SourceRef knowledge
  仍是 provenance-root binding，因为 refs 未改写。
- hard delete 或一般 content rewrite 没有历史表时无法完整恢复；规格明确少返回，
  不根据 current bytes 猜过去。
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

### Relations and resolution

- Relation 两端必须都在本次 scoped claim set。`Supersedes` 与普通 `Refutes`
  只有两端 `SubjectIdentity` 完全相同才进入 survivor/conflict 计算。
- `Supports`/`DerivedFrom` 可以在 scoped set 内跨 typed subject；当一端是 winner
  时作为 provenance 输出，但不参与 survivor、trust 或 recency。
- 唯一 cross-identity decision exception 是 canonical operation-backed
  preference conflict。两端必须同 owner、同 normalized memory scope、同
  normalized branch（`COALESCE(branch,'')` byte equality）、都是
  `memory_type=preference`，并分别是自身 slot survivor；post-pass 将两个
  output 标为 Contradicted，但不合并 identity。
- 带 `source_operation_id` 的 conflict 必须验证 operation kind、
  `conflicting_ids` 的 integer-ID array、replacement/pairwise membership、
  source/candidate/edge linkage。missing/wrong/malformed/inconsistent data 返回
  包含 edge/operation/endpoints/field 的 contextual error。没有 operation ID
  的 unbacked conflict edge decision-neutral。
- Canonical graph/dream pairwise conflict 可以连接 heterogeneous owner/type；
  writer 合法使用 `owner_scope=repo`、`owner_key=source_project`、
  `memory_type=memory` fallback metadata。只要 operation/link/membership 有效，
  这种 row 不报错，但因不满足 uniform preference predicate而
  decision-neutral。
- approved uniform-conflict graph 在 same-pair parallel edges 折叠后必须是
  matching；任一 survivor 有两个不同 partner（如 A-B 与 A-C）即 contextual
  integrity error。
- approved cross-topic pair 的每个 output 固定为：subject 保持各自 identity；
  `claim=None`；`validity=Contradicted`；evidence 是双方 survivor evidence
  按 evidence_ref 去重并按标准排序的 union；`supporting_relations=[]`；
  `contradicting_relations` 是连接双方的 validated canonical conflict relations
  标准排序集合；`rejected` 保留各 slot 之前 rejected refs 并 byte-sort/dedup；
  `conflicting_claims` 固定含双方并按 canonical_ref bytes 排序；
  `selected_reason=UnresolvedConflict`。两个 outputs 使用同一 pair/relation set。
- unbacked 明确要求 candidate/operation 两个 source ID 都为 NULL；knowledge 是
  edge creation，不查 lookup，unbacked conflict decision-neutral。candidate-only
  contextual error；operation-only 合法。operation-backed provenance 不
  late-resolve：operation creation 必须 `<=edge.created_at_epoch`，两者
  reference-eligible。claimed candidate creation 必须 `<=` operation、匹配
  source discriminator/ID，并证明 canonical completion：
  memory status/result/operation/endpoints，或 graph status/promoted-edge/
  source-operation。writer 在 edge 后更新 candidate，所以 relation knowledge 是
  edge/operation creation 与 validated application update 的 max；它须
  reference-eligible，但不要求 candidate update<=edge。dangling/future-created/
  mismatch 报错；application before/equal/after fixtures 防 retroactive visibility。
- Resolver 顺序固定：scope/time/lifecycle eligibility → exact-identity
  supersedes → exact-identity refutes → evidence trust → recency →
  cross-topic preference post-pass。stored confidence 不进入决策。

### Bounded read behavior

- `memory_edges`、trusted memory-to-memory `graph_edges`、captured events、
  observation links和 policy suppressions 都必须由 scoped IDs/owner predicates
  约束；不得扫描无关 project 全表后用 Rust `Vec::contains` 过滤。
- SQLite bind-ID chunk 固定最大 900。任何 scoped set 都通过 stable ascending ID
  chunks 查询，避免依赖构建机的 `SQLITE_MAX_VARIABLE_NUMBER`。
- 加入 deterministic fixture `truth_bounded_lookup_contract`：
  seed `933`，target project 901 memories、1,802 relevant relations、901 evidence
  refs、900-link high-fanout subject；unrelated project 4,505 memories、9,010
  relations、4,505 evidence refs。它必须证明 chunk boundary+1、high fanout 和
  大量 unrelated rows 都不会泄漏或改变 target output。
- Structural pass conditions：
  - 每个 edge/evidence/suppression plan 使用对应 existing index search，不出现
    无 project/ID predicate 的 table scan；
  - bind count 每 statement `<=900`；
  - materialized relation/evidence rows不超过 target fixture 的 relevant row
    count，unrelated returned rows 为 0；
  - SQL statement count `<= 12 + 5*ceil(scoped_claims/900)
    + 2*ceil(scoped_evidence_refs/900)`；
  - 在加入 unrelated corpus 前后 statement count、target row count 和
    serialized target output完全相同。
- `src/truth/tests/performance.rs` 提供 ignored release-mode recorder。命令：

```bash
GH933_PERF_JSON_OUT=/tmp/gh933-truth-perf-v2.json \
  cargo test truth_performance_contract --release -- --ignored --nocapture
```

  固定 5 次 warm-up、50 次 measured runs。JSON 必须含
  `schema_version=1`、exact head SHA、seed/corpus counts、chunk size、SQLite/
  Rust versions、query plans、statement/bind/row counts、serialized bytes、
  p50/p95、migration/index/truth/dependency fingerprints 和每项 structural
  check boolean。Rust test 自身验证必填字段与 structural checks；p50/p95 仅记录，
  本 Phase A 未建立可跨机器比较的 latency hard budget，不能把任意数字称为 pass。
  final candidate SHA 的 artifact 只能在最后一次 commit/base sync 后生成。

## Product-to-Test Mapping

| Invariant | Verification |
| --- | --- |
| CT-001 | typed identity、exact selector、stable serde/order、v2 golden、effective reference epoch |
| CT-002 | 所有已知 lifecycle values；quarantined observation 显式 Suppressed |
| CT-003 | repo owner/target Project inclusion、stale non-repo exclusion、Owner union、global/legacy fallback、wrapper suppression isolation、Project/Owner branch、relation scope |
| CT-004 | memory/event time、candidate completion/route chain、procedure current-only、provenance-root refs、edit/candidate before/equal/after、in-place mutation |
| CT-005 | exact-identity supersedes beats recency |
| CT-006 | full-blob canonical classifier、provenance-root/binding checks、total recursive user source grammar、candidate own-result/edit invariants、summary provenance fail-closed、WebFetch/MCP/Bash-network、pack/external cap、no-uplift/unknown class |
| CT-007 | same-slot refutes、preference post-pass、overlap error、heterogeneous canonical pair neutral、malformed operation errors |
| CT-008 | empty/stale-only abstention；malformed/dangling/unknown fail closed |
| CT-009 | Observation DTO/catalog/order/dedup/NULL-ref ModelGenerated/NULL-epoch/read-scan/trust；memory_facts temporal attachment；no implicit link |
| CT-010 | ClaimSource 仅 Memory/UserContextClaim；Observation 只作 evidence |
| CT-011 | raw status validation；七种 suppression target（含两种 non-applicable）/owner/time boundaries |
| CT-012 | Archived 不进 current truth/catalog；后续 historical explanation 单独设计 |
| CT-013 | Phase B context load/render/error/rollback；本 PR 不声称完成 |
| CT-014 | Phase C benchmark/architecture decision；本 PR 不声称完成 |
| CT-015 | SQLite authorizer + clean/poison-match `total_changes` SELECT-only；public API compile test；restricted v1→v2 diff |

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
  2. EvidenceView v2 fields/integrity、ClaimView temporal-field replacement、
     Observation catalog/read-scan/nullable-epoch/trust/explicit attachment；
  3. NULL/exact-empty topic singleton、owner/scope/type isolation、canonical owner/target
     Project inclusion、stale non-repo placement exclusion、Owner memory+claim
     union、global/legacy fallback 与 user-claim-only compatibility wrapper；
  4. versioned edit 与 candidate multi-row/no-op historical recovery、in-place
     mutation conservative exclusion；
  5. policy suppression owner/time visibility；
  6. canonical stored+recomputed source-trust cap、all-source binding-time
     checks、first-party explicit-user rules、candidate/result/edit invariants、
     summary provenance fail-closed 与 full-blob external/tool提权修复；
  7. valid heterogeneous conflict 由 error 改为 neutral；
  8. approved cross-topic preference outputs 与 overlapping-pair error；
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
- Projection 只读：不执行 migration、write、LLM、network 或 external process。
  读取失败记录/返回 diagnostic error，不能包装为空成功。

## Phase B / Phase C Boundaries

- Phase B 才让 Context Bundle 使用 projection。一次 render 只计算一个
  reference epoch；current truth、decisions、conflicts 分开渲染；旧 path 有明确
  rollback；projection failure 必须 error-visible。worktree/task selector、
  budget/cache/historical explanation 也在 Phase B 细化。
- Phase C 才评估 writer convergence。是否收敛由 benchmark 与架构审阅决定；
  migration、dual-write、backfill、cutover 和 generated-enrichment writer
  firewall 需要独立 contract。Phase A DTO 不自动授权 schema 改动。

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
- [ ] Fresh exact-head CI and independent review.

## Rollback

Phase A hardening 不新增 schema 或数据 mutation。若 v2 有问题，回滚 truth module、
public integration test、docs/changelog 与整组 0.7.0 distribution metadata；不得
只回退一份 manifest 造成版本漂移。已发布 0.6.x v1 artifacts 不可改写。

Phase B 保留旧 context path 并按独立 rollout/rollback design 切换；projection
失败不得静默输出缺失 context。Phase C 如果引入 migration/dual-write，必须在
独立 contract 中给出停止写入、向后兼容读取、数据校验与恢复步骤。

## Implementation Change Set

Phase A v2 implementation 预计修改以下现有/明确新增路径，且不得静默扩大到
writer、schema 或 context：

- `src/truth.rs`
- `src/db/capture.rs`（只暴露 content boundary/pure preview helper）
- `src/memory/poisoning.rs`（只暴露 pure classifier）
- `src/truth/adapter.rs`
- `src/truth/lifecycle.rs`
- `src/truth/projection.rs`
- `src/truth/types.rs`
- `src/truth/tests.rs`（先拆分）
- `src/truth/tests/**`
- `tests/truth_public_api.rs`
- `README.md`
- `docs/ARCHITECTURE.md`
- `docs/specs/GH933/PRODUCT.md`
- `docs/specs/GH933/TECH.md`
- `docs/specs/README.md`
- `CHANGELOG.md`
- `Cargo.toml`
- `Cargo.lock`
- `plugins/remem/.codex-plugin/plugin.json`
- `plugins/remem/runtimes/remem-releases.json`
- `npm/remem/package.json`
- `server.json`

如果实现证明需要新 index/migration、writer、Context Bundle 或新的 public network
surface，停止该 implementation PR，先更新当前 contract 并重新审阅范围。
