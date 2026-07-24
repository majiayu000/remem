# Task Plan

## Linked Issue

GH-825（Refs #825；关联 #821；依赖 #822、#823；installed E2E 关联 #824）

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`

## Current Gate

本计划只固定依赖满足后的顺序，不批准实现。#822/PR #914 evidence 人工采用、
GH-823/GH-825 exact-head
人工规格批准和 `ready_to_implement` gate 未满足前，`SP825-T3` 以后全部 blocked。GH-825 禁止
新增 migration、表、列或索引；tool payload、identity、status/loop parser 只能采用 GH-823/#822
批准合同。

## Implementation Tasks

- [ ] `SP825-T1` — Real-host evidence — Owner: maintainer with real Cursor access；Dependencies: #822；Done when: PR #874 exact head的Cursor 3.6.31 `role/message`与PR #914 exact head `c0802c42c3fc22770aecb0b7b2eec88f117f795c` 的Cursor 3.12.17 `role/message`、assistant `tool_use`、独立`turn_ended`、completed/aborted数字`loop_count:0` fixture已获人工采用；记录无逐消息ID/timestamp/独立tool_result及物理record顺序。#822继续补齐可信根、platform path、transcript max/append、非零/缺失/null loop、status:error和跨版本grammar。单次fixture不宣称完整grammar；Verify: 两个exact-head bundle、fixture provenance、secret scan、maintainer approval。未完成时所有runtime任务blocked。
- [ ] `SP825-T2` — Human contract freeze — Owner: human maintainer；Dependencies: `SP825-T1`、GH-823 Draft spec；Done when: PR #914的 `loop_count:0`/status/record grammar实测结论已进入GH-823 packet amendment，人工在 exact heads 批准 GH-823 的 canonical identity/event、`CURSOR_TOOL_FIELD_MAX_BYTES`、malformed/over-limit zero-write、accepted status、loop字段类型/缺失/null/规范化/canonical key，以及 GH-825 的 transcript max/path/versioned record enum、所有JSONL物理record ordinal、raw ordinal空洞、`turn_ended`内部隔离、无合成逐消息identity/独立tool_result要求、Stop/snapshot→既有transcript identity关联、Stop/task原子性、completed/aborted grounded、若批准error则no-LLM、no-new-schema合同，并分别记录 readiness；Verify: GH-823 amendment diff、GitHub approval/readiness evidence和两份packet head SHA。loop amendment或完整grammar未批准时runtime任务blocked，但不得把PR #874/#914正例仅因缺少未观察字段判invalid。
- [ ] `SP825-T3` — GH-823 foundation handoff — Owner: GH-823 implementation lane；Dependencies: `SP825-T2`；Done when: 见下；Verify: 见下。
  GH-823 SP823-T3/T4 的 canonical
  host/session/workspace、outer bounded parser、PII removal、observe generic capture、
  B-016人工选择的唯一 MCP ownership branch和所有 persistence allowlists 已实现并合入：
  若选择generic ownership，则 beforeMCPExecution/afterMCPExecution均不注册，generic
  postToolUse 是唯一 capture owner；若选择specific ownership，则只注册
  afterMCPExecution、beforeMCPExecution不注册，且generic MCP postToolUse为成功zero-write。
  GH-825只消费该已批准分支产出的单一 canonical MCP capture，不能把
  afterMCPExecution-only 写成无条件前置。Stop transcript wiring仍保持disabled；
  Verify: 与GH-823 B-016 approval一致的条件性focused tests、final-head review/CI、capability
  evidence及每次MCP调用exactly-one capture。该任务不等待
  GH-823 SP823-T5，因此不形成依赖环。
- [ ] `SP825-T4` — Atomic Stop/task and production snapshot bundle — Owner: GH-825 reader lane；Dependencies: `SP825-T2` `SP825-T3`；Done when: 见下；Verify: 见下。
  normalized Cursor Stop 的 stable key 绑定
  canonical host/project/session identity、source Stop stable event key 和 canonical Stop key，
  并与 durable pending SessionRollup task 原子提交；PR #874/#914 fixtures 在无逐消息
  ID/timestamp/独立tool_result 下仍验证成功，IR physical ordinal 覆盖所有 JSONL records，
  `turn_ended` 不进入 raw/prompt；`commit_cursor_snapshot_bundle` 是唯一 raw writer，在单一
  immediate transaction 内 full-first 仲裁、复用 transcript identity、按 ordinal 写 exact-content
  occurrence、blob/event/capture-drop 并推进 task watermark；安全reader固定trusted source
  root/normalized path，helper解析identity后才构造Full，且candidate/bundle/companion不从Stop
  payload重推identity。同identity多snapshot必须验证单调prefix chain，reconcile选择最长approved
  boundary并拒绝fork/truncation/mutated prefix。Cursor path/parser degradation 与
  Full raw insertion error 都不写 `raw_ingest_failures`，后者使整包 rollback/retry。重放、append、
  whitespace/identity/evidence conflict 与 raw reconcile 路由继续遵守 tech spec；
  Verify: PR #874/#914 fixtures、ordinal/internal-boundary、append/replay/conflict、
  cross-project same-session/Stop distinct-ID、helper/reconcile、bundle fault injection、
  Cursor raw-ingest-failure zero-write spy、path/max/TOCTOU、schema snapshot，以及
  `cargo test -q db::capture --lib`、`cargo test -q memory::raw_archive --lib`、
  `cargo test -q memory::raw_reconcile --lib`、`cargo test -q session_rollup --lib`。
- [ ] `SP825-T5` — SessionRollup recovery/full-first/degraded/blank — Owner: GH-825 rollup lane；Dependencies: `SP825-T4`；Done when: 见下；Verify: 见下。
  preparation grace/Waiting 与 serialized helper 重查保证
  full 已存在时零 degraded/drop，其他情况原子提交 degraded bundle；full-first loader 按 Stop
  key/status 拆分 work item，payload-only 只引用 approved range，blank 固定 AI=0、
  `reason:no_usable_evidence`、空 content 且无 summary ID/range；publish 前 immediate authority
  recheck，变化则整项 zero-write retry，publish raw-write spy 始终为零，degraded/blank 禁止
  candidate/workstream/follow-up；terminal selector固定 `full > degraded > blank`，blank后迟到
  approved payload可提升为degraded，反向到达和replay不降级；Verify: 两条精确 barrier、
  blank→degraded→full及反向/replay矩阵、bundle 各写点/commit/Waiting fault
  injection、Cursor raw-ingest-failure zero-write spy、mixed-status LLM spy、payload/blank/internal
  sentinel、`cargo test -q session_rollup --lib`、`cargo test -q memory::raw_archive --lib`。
- [ ] `SP825-T6` — Full-first outcomes and doctor — Owner: GH-825 observability lane；Dependencies: `SP825-T5`；Done when: 见下；Verify: 见下。
  stable outcome ID 绑定 canonical host/project/session 与
  source Stop stable event key，跨项目同 session/Stop 不碰撞；publish 复用 authoritative
  selector，full 后到成为 doctor、CLI raw sessions/messages、context 与 API 唯一当前态，旧
  blank/degraded summary/drop 仅审计且 drop 可链接 recovery；每个 Stop 内固定使用
  `full > degraded > blank`，session-level读取面先按immutable source Stop
  `(created_at_epoch,id)`选当前Stop、再取其fidelity，outcome replay不推进Stop顺序，早期full不能
  遮蔽后续degraded/blank；CLI/API 暴露
  `capture_health` full/degraded/blank/null，blank 内容/summary 字段严格为空，损坏 outcome/DB
  读取报错；`raw sessions` union无raw row的outcome-only Cursor session并用
  `source_root:"cursor-outcome"`零计数行呈现，`raw messages`对该locator返回空页+health；
  `cursor-outcome`必须由`ScanRoot::parse`及ingest preflight在持久化前拒绝，default roots不受影响，
  持久化同名raw/identity collision读取non-zero；
  Verify: cross-project outcome-ID uniqueness、每Stop full/degraded/blank到达顺序、
  earlier-full/later-blank、earlier-blank/later-full、same-epoch source-Stop id tie、replay不推进、
  blank schema、outcome-only union/filter/order/dedup与reserved-locator parse/preflight/
  persisted-collision/empty-page、
  publish interleave、CLI/context/API current-only、public contract、recovered/corrupt fixtures、
  no-new-schema、`cargo test -q doctor --lib` 与 CLI/API focused tests。
- [ ] `SP825-T7` — Joint GH-823 Stop wiring — Owner: GH-823 Stop lane + GH-825 verifier；Dependencies: `SP825-T4` `SP825-T5` `SP825-T6` and GH-825 reader capability merged；Done when: #822后GH-823 loop amendment已在exact head批准，GH-823 SP823-T5把approved normalized Stop接到GH-825 reader；completed/aborted × new/replay/conflict canonical loop key矩阵始终通过，二者有evidence AI=1、无evidence AI=0；若GH-823批准error，则增加error × new/replay/conflict、completed+error、aborted+error和三状态mixed-range矩阵，按Stop拆分、保留证据、error始终AI=0并写error-level log；若未批准，则exact status:error由outer parser non-zero拒绝、zero writes、AI=0且不得创建task；Cursor不得进入Claude/Codex parser；Verify: amendment approval、reader spy、approved-status/loop matrix、条件性mixed-range LLM-call matrix、unapproved-error zero-write invalid input、idempotent replay、capture-preservation、full Claude/Codex regression、final-head independent review/CI。该联合阶段完成 GH-823 的 Stop handoff，不要求 GH-823 在 GH-825 reader之前“全部实现”。
- [ ] `SP825-T8` — Runtime fixture verification — Owner: verification agent；Dependencies: `SP825-T7`；Done when: 在隔离data dir、无需安装hook，直接用approved normalized fixtures运行PR #874/PR #914无逐消息ID/timestamp/独立tool_result正例、record enum/物理ordinal空洞/turn_ended隔离、同文重复record、同路径snapshot append replay、多个growing Full companion的每一approved prefix boundary与最长approved选择、fork/truncation/mutated-prefix拒绝、既有ordinal changed-field raw conflict、same-snapshot replay、same-hash/different-path-identity conflict、changed-snapshot companion conflict、full/null/missing/corrupt/oversized/status-loop/retry、early-claim/grace、每Stop `full > degraded > blank`所有到达/replay顺序、跨Stop earlier-full/later-blank、earlier-blank/later-full、same-epoch source-Stop id tie/replay，以及reserved `cursor-outcome` user-root/preflight/persisted-collision和bundle逐写点crash；断言Full构造前已固定trusted source root/normalized path/identity ID且后续不从Stop重推，identified rows均有identity+ordinal且未走identity=NULL content dedup，raw只有bundle写入且publish zero-raw，raw reconcile按companion prefix chain选择Cursor parser并对missing/conflicting/hash-mismatch显式失败，API capability type序列化正确；所有Stop均有durable task，commit前无partial bundle，commit后diagnostics/task watermark不丢，最终每Stop收敛到semantic-priority最高的full/degraded/blank且session读取面显示immutable当前Stop，或留下可见pending/failed；schema snapshot不变，完整suite通过；Verify: deterministic parser/occurrence/reconcile/replay/conflict/barrier/crash-injection matrix、`cargo fmt --check`、`cargo check`、focused tests、`cargo test`、`python3 checks/check_workflow.py --repo . --spec-dir specs/GH825`、final-head review/CI。
- [ ] `SP825-T9` — Installed-hook E2E — Owner: human release verifier；Dependencies: `SP825-T8`、GH-824 merged with Cursor Stop capability probe；Done when: 在隔离HOME和合成workspace通过managed `hooks.json`运行真实Cursor Stop full/degraded/status-loop smoke，doctor与session输出正确；不读取真实用户会话；Verify: #824 final-head evidence、installed config receipt、sanitized smoke log、人工verification record。GH-824未完成时只阻塞发布/广告，不反向阻塞T4-T8 runtime实现。

## Parallelization

- `SP825-T1` 是 real-host evidence lane，完成后 `SP825-T2` 由人工串行收口。
- `SP825-T3` 属于 GH-823 基础 lane；其合入后，`SP825-T4`→`T5`→`T6` 默认串行，因为共享
  ledger IR、range过滤和outcome合同。若要并行，必须先固定接口并声明不重叠文件所有权。
- `SP825-T7` 是 GH-823 Stop wiring 与 GH-825 reader 的联合集成，不与 reader核心并行。
- `SP825-T8` 是无安装依赖的runtime验证；`SP825-T9` 单独等待GH-824 installed surface。

## Verification

- Spec stage: `git diff --check`
- Spec stage: `PYTHONPATH=checks python3 -c 'from pathlib import Path; from sensitive_enforcement import parse_planned_changes_manifest; m=parse_planned_changes_manifest(Path("specs/GH825/tech.md").read_bytes()); assert m["version"] == 1 and m["issue"] == 825 and m["complete"] is True'`
- Spec stage: `python3 checks/github_issue_evidence.py --github-repo majiayu000/remem --issue 825 --json > /tmp/gh825-issue-evidence.json`
- Spec stage: `python3 checks/github_duplicate_evidence.py --github-repo majiayu000/remem --issue 825 --json > /tmp/gh825-duplicate-evidence.json`
- Spec stage: `python3 checks/route_gate.py --repo . --route write_spec --issue 825 --evidence /tmp/gh825-issue-evidence.json --duplicate-evidence /tmp/gh825-duplicate-evidence.json --artifact product_spec=specs/GH825/product.md --artifact tech_spec=specs/GH825/tech.md --artifact task_plan=specs/GH825/tasks.md --json`
- Spec stage: `python3 checks/check_workflow.py --repo .`
- Spec stage: `python3 checks/check_workflow.py --repo . --spec-dir specs/GH825`
- Implement route: fresh #822 evidence、GH-823/GH-825 exact-head approval 与 `ready_to_implement`
- Focused: `cargo test -q db::capture --lib`
- Focused: `cargo test -q memory::raw_archive --lib`
- Focused: `cargo test -q session_rollup --lib`
- Focused: `cargo test -q doctor --lib`
- Completion: `cargo fmt --check`, `cargo check`, `cargo test`
- Schema: migration/schema snapshot unchanged；无新migration/table/column/index
- Merge readiness: exact-head independent review、CI、unresolved-thread和PR gate evidence

## Handoff Notes

- `tasks.md` 不授权runtime实现，也不把GH-825自动提升为`ready_to_implement`。
- 依赖未满足的spec PR只使用 `Refs #825`，不得使用 `Closes #825`。
- GH-823是outer protocol唯一owner；GH-825不得硬编码tool-field限额或放宽malformed/over-limit
  zero-write。
- PR #914 提供 completed/aborted 数字 `loop_count:0` 真实证据；GH-823 packet amendment与
  exact-head人工批准独占完整字段类型集合、缺失/null、规范化和canonical Stop key。GH-825只
  消费批准结果，未批准时不得实现。
- PR #874/PR #914 已观察 `role/message`、assistant `tool_use` 与独立 `turn_ended`；它们不是
  完整grammar批准，但缺少逐消息ID/timestamp/独立tool_result本身不得触发`format_invalid`。
  IR顺序只来自稳定JSONL物理record ordinal，所有record占位、raw ordinal可有空洞，
  `turn_ended`内部隔离，禁止合成message identity。companion幂等用canonical Stop key、status与
  snapshot hash；raw occurrence幂等另用既有transcript identity+ordinal，两层不得混用。
- GH-825 reader可先合入disabled capability，随后GH-823 SP823-T5接线；这是消除依赖环的唯一
  批准顺序。
- Stop与pending SessionRollup task必须复用现有capture savepoint原子提交；companion bundle必须
  通过新增production helper `commit_cursor_snapshot_bundle` 在单一immediate transaction内完成
  semantic-key重查、full-first仲裁、companion、相关diagnostics与task watermark。不得把现有API
  虚构为已支持整包事务，不得增加preparing字段或恢复事务外check-then-write。
- Waiting只允许在helper成功commit后事务外执行；失败由lease recovery重试完整bundle。helper失败
  则进入现有defer/backoff，不得Waiting或吞掉diagnostics。任何loader/outcome都不得让degraded覆盖full。
- no-new-schema是验收合同。若实现证明现有ledger/drop/raw/session表无法满足，停止并回到issue/
  spec人工改scope，不得静默新增migration。
- v071 occurrence合同是实现基线：identified Cursor rows必须携带`transcript_identity_id`与
  `transcript_record_ordinal`；同文不同ordinal分别保留，existing ordinal稳定字段冲突整包回滚，
  不得退回content-hash raw dedup或增加source message ID。
- GH-824只阻塞installed-hook E2E和发布宣称，不阻塞隔离runtime fixture验证。
