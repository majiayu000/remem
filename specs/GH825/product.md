# Product Spec

Status: Draft，等待 #822/PR #914 evidence 人工采用、GH-823/GH-825 人工规格批准；
本文件不授权实现。

## Linked Issue

GH-825（Refs #825；关联 #821；依赖 #822、#823；安装态验证关联 #824）

## 用户问题

Cursor hook 的 base payload 提供可空的 `transcript_path`，但真实 transcript 格式、写入完成
时机、可信目录、输入上限以及 `loop_count` 语义必须由 #822 的真实 Cursor PoC 证明。PR #914
exact head `c0802c42c3fc22770aecb0b7b2eec88f117f795c` 已补充 Cursor 3.12.17 的
payload/transcript/status/loop 实测，但可信根、平台路径与输入上限仍未知。Cursor 的
host、会话、workspace、事件和 tool payload 边界由已合入的 GH-823 Draft 规格管理，仍等待
真实证据和人工批准。

若把 Cursor 文件交给 Claude/Codex JSONL reader，可能误判为空、留下部分 raw archive，或在
Stop 入账前因路径错误丢掉整次会话。若把 `null`、损坏或缺失文件当作成功，又会静默损害记忆
质量。本规格只在现有 capture ledger、raw archive、可恢复的 capture-drop diagnostics 和
SessionRollup 上增加 Cursor reader 与显式降级；既有 legacy raw-ingest failure history 保持
不变，不引入 Cursor 专用存储 schema。

## 目标

- 使用 #822 的真实脱敏 fixture 实现 Cursor transcript parser，并让同一份完整验证后的规范 IR
  同时驱动 raw archive 与 SessionRollup prompt evidence。
- 路径缺失、`null`、不可读、不可信、超限或格式损坏时，先保留有效 Stop 和此前已捕获的
  GH-823-approved `postToolUse`、获准的 `postToolUseFailure` 与 after-only
  `afterMCPExecution` ordinary capture evidence，再显式走 payload-only 或 blank 降级；
  `beforeMCPExecution` 不作为 capture owner，不得发明 `afterFileEdit` event。
- 让 `completed | aborted | error` 与经 #822 实测、随后由 GH-823 packet amendment 人工批准的
  `loop_count` 契约具有确定、幂等、可测试的捕获与 LLM 调用语义。
- 让 error log、doctor 和 session 读取面区分 full/degraded/blank，同时保留历史失败而不把旧失败
  永久冒充当前失败。
- 保持 Claude Code 与 Codex 的现有解析、raw archive、SessionRollup 和 doctor 行为不变。

## 非目标

- 不导入 Cursor 内置 Memories。
- 不在本 issue 安装 `hooks.json` / `mcp.json`；安装面属于 #824。
- 不在本 issue 定义 Cursor stdin/stdout、身份、tool payload 限额或失败事件映射；这些属于
  #823，并以 #822 证据和人工决定为唯一合同。
- 不新增表、列、索引或 migration；复用 `captured_events` / `event_blobs`、
  `capture_drop_events`、`raw_messages`、`session_summaries` 和 `extraction_tasks`；
  `raw_ingest_failures` schema/legacy writers 保持不变，Cursor 路径不新增该表记录。
- 不改变 v071 的 `raw_messages` occurrence identity：已识别 transcript record 使用
  `transcript_identity_id + transcript_record_ordinal` 保留每次出现；只有未绑定 transcript
  identity 的既有来源继续使用 content-hash 幂等。GH-825 不新增逐消息 ID/schema，也不恢复
  legacy `Summary` job 或 `src/summarize/input.rs` 生产路径。
- 不承诺从后来变化的原始路径把同一个 degraded Stop 升级为 full。

## 依赖和人工门

1. #822 必须记录真实 Cursor 版本、脱敏 transcript/payload fixture、格式与版本、Stop 时完整性、
   可信根、平台路径形态、文件尺寸/追加行为、真实 tool name、`status` 与 `loop_count` 字段类型、
   缺失规则、排序/重复语义和稳定 event identity。PR #874 的 Cursor 3.6.31 foreground
   Stop fixture 已证明两个 transcript record 只有顶层 `role` 与 `message`，没有逐消息 ID、
   timestamp、sequence 或 token metadata。PR #914 exact head 的 Cursor 3.12.17 fixture
   进一步观察到 `{role,message}` record、assistant `message.content[].tool_use` block，以及
   独立 `{type:"turn_ended",status:"success"}` / cancelled error record；没有观察到独立
   `tool_result` record。两组证据都不是跨版本完整 grammar 批准，但后续合同不得把未观察字段
   反向设为这些正例的必填项，也不得要求独立 `tool_result`。
2. GH-823 的人工批准先冻结 canonical host/session/workspace、事件 discriminator、
   `CURSOR_TOOL_FIELD_MAX_BYTES`、malformed/over-limit zero-write 语义和 Stop identity。#822 得出
   `loop_count` 结论后，必须由 GH-823 packet amendment 定义字段类型、缺失/null、规范化和
   canonical Stop key，并在 exact amendment head 获人工批准；GH-825 只消费该批准结果，不得
   自行定义或放宽这些合同。
3. 实施顺序固定为：GH-823 基础 identity/context/observe 接口（SP823-T3/T4）→ GH-825
   transcript reader、ledger IR、fallback 与 doctor → GH-823 Stop wiring（SP823-T5）联合集成。
   GH-825 reader 可先以禁用/未接线能力合入；不得反过来等待“GH-823 全部实现”形成循环。
4. #824 只负责安装面。GH-825 的 runtime fixture 验证不依赖 #824；真实 installed-hook E2E
   必须等待 #824，并且 #824 不得在 GH-825 reader 与 GH-823 Stop wiring 可用前安装 `stop`。
5. `tasks.md` 只记录依赖满足后的顺序。PR #914 尚未获人工采用，可信根、platform path、
   transcript max 和完整 versioned grammar 也尚未冻结；人工规格/readiness gate 未满足时，
   所有 runtime 任务保持 blocked。

## Behavior Invariants

1. P-001 未经 #822/PR #914 exact-head 真实证据获人工采用和 GH-823/GH-825 人工批准，
   Cursor transcript reader、路径信任、
   输入上限、`loop_count` 解释和 Stop wiring 都保持 fail-closed；不得从 Claude/Codex 或文档
   类比猜测。
2. P-002 GH-823 是 Cursor 外层协议的唯一 owner。malformed 或超过人工冻结 encoded/decoded
   上限的 `tool_input` / `tool_output` 必须 non-zero、zero writes：不得 capture、enqueue、spill、
   adapter call，也不得把原始 payload 写入诊断。未知但合法的 `tool_name` 仍按 GH-823 的通用
   capture 合同原名入账，不获得已知工具捷径。
3. P-003 只有通过 GH-823 身份、workspace、event、`status` 和 canonical loop key 校验后，Stop
   才可进入 GH-825；loop key 必须基于 #822 证据并经 GH-823 packet amendment exact-head 人工
   批准，GH-825 不得自行定义。身份或字段错误在外层 non-zero 且 zero writes；此前已入账的
   事件不得删除。
4. P-004 对一个协议有效的新 Stop，系统必须在现有 capture transaction 中原子写入
   `session_stop` 和 durable pending SessionRollup task，再处理 transcript IR。任何时刻都不允许
   存在“Stop 已提交但没有可恢复 task”的状态；事务失败则两者都不写并 non-zero。
   `transcript_path` 缺失、`null`、空白、文件不存在、权限失败或解析失败不能使该 Stop、task
   和此前 payload 证据消失。
5. P-005 transcript reader 只接受位于 #822 人工批准可信根内的绝对普通文件，并使用批准的
   no-follow/platform 规则和 `CURSOR_TRANSCRIPT_MAX_BYTES`。未证明的 Windows/UNC 形态、相对
   路径、穿越、symlink/reparse point、FIFO/socket/device、越界、超限、短读或复制期间变化
   均不得作为 transcript evidence。
6. P-006 Stop/task 原子提交后，同一 hook invocation 必须完整读取并全量验证 transcript；任何格式
   签名、版本、编码、记录或尾部错误使整份输入失败，不得提交有效前缀。成功后生成受限、脱敏、
   不可变的规范 Cursor transcript IR，并通过 GH-825 新增的 production transaction helper
   `commit_cursor_snapshot_bundle` 原子仲裁/提交带稳定 session snapshot semantic key 的
   identified raw occurrences 与 `captured_events`/`event_blobs` full companion。该 helper
   是 raw insertion 的唯一 owner；后续 SessionRollup publish 只读取这批 raw/IR evidence，
   不得再次 drain 或插入 raw。PR #874 与 PR #914 已观察的
   `{role,message}` record 缺少逐消息 ID 或 timestamp 本身不是格式错误；PR #914 的
   assistant `tool_use` content block 和独立 `turn_ended` record 也是当前 versioned grammar
   amendment 的正例，parser 不得要求未观察到的独立 `tool_result`，也不得合成未观察字段。
   规范 IR 必须保存每个物理 record 的原始顺序：message record 可含批准的 text/tool_use
   block，`turn_ended` 仅作为内部 turn boundary/status evidence，不伪装成 raw message。
   在构造任何 Full candidate 前，安全 reader 必须在所持 descriptor 与稳定 bytes boundary 下
   固定 approved local `source_root`、规范 `transcript_path` 并解析/复用
   `transcript_identity_id`；这三个可信 identity 字段随 candidate、bundle 和 companion
   传递，后续不得从原始 Stop payload 重新推导。完整 snapshot 由 canonical Stop key/status、
   稳定 bytes hash 和 byte length 标识；snapshot hash 不是 transcript identity，worker 永不
   重读原始路径。
7. P-007 只有规范 IR 含至少一条 usable user/assistant evidence 时该 Stop 才是 `full`。同一 IR
   必须同时供 raw archive 和 bounded prompt projection 使用。Cursor raw projection 必须复用
   v071 的 identified occurrence 路径：同一 approved local source root + 规范可信
   `transcript_path` 在多个 Stop/snapshot 间解析为同一 `transcript_identity_id`；每条已验证
   role+message record 的 `transcript_record_ordinal` 是它在完整稳定 JSONL snapshot 中从零开始的
   物理 record 位置；独立 `turn_ended` 和其他已批准非 message record 同样占用物理 ordinal，
   因而 raw message ordinal 可以有空洞。ordinal 在任何 record 类型、role/usability 过滤前确定。
   不得把 content hash 或 IR vector 的压缩
   位置当 occurrence key，也不得合成 source message ID。相同 identity+ordinal+稳定字段重放为
   no-op；同 identity 的后续 appended snapshot 必须形成单调 prefix chain：新稳定 bytes 在每个
   既有 approved byte-length boundary 的 prefix hash 都匹配原 companion，且 length 不缩短，
   然后只增加新 ordinal。相同长度不同 hash、缩短、分叉或既有 prefix/ordinal 的 role/text/
   event-time 等稳定字段变化都是 identity conflict，整个 raw/bundle 写入回滚并显式失败。相邻或
   非相邻的同 role 同文本只要 ordinal 不同，就必须作为不同 `raw_messages` occurrence 保留；
   leading/trailing whitespace 与完整 UTF-8 text bytes 也是稳定字段，full fidelity 路径不得调用
   会 `trim()` 内容的 legacy helper。content hash 对 identified rows 只用于稳定字段校验/有限
   legacy claim，不负责去重。
8. P-008 transcript 不可用但 ledger 中存在合法 payload evidence 时，SessionRollup 可以生成
   grounded payload-only 结果，fidelity 为 `degraded` 且带稳定 reason code。它不得声称包含
   未观察到的对话或完整 transcript。对同一 semantic key，full companion 永远优先：full 已存在
   时不得新写 degraded；degraded 先存在而同一次 Stop hook 的已验证 full IR 后到时，full 可原子
   supersede 并成为 loader 的唯一内容选择，degraded 只保留为诊断历史。
9. P-009 transcript 与 payload 都无 usable evidence 时，不调用 LLM，不创建伪造 summary/topic
   segment；用户内容为空，并写独立 `blank` terminal outcome：
   `reason: no_usable_evidence`、无 summary ID/range。相关 transcript failure 仍由现有
   ledger/capture-drop diagnostics 记录，但不得把 blank 伪装成 degraded summary。
10. P-010 PR #914 已观察 `completed` 与 `aborted`；`error` 尚未实测，仍是 GH-823
    amendment 的候选终态。只有经批准的终态才可进入系统；每种状态都保留其 Stop、transcript IR
    或 payload evidence。`completed` 和 `aborted` 有 usable evidence 时运行 grounded
    SessionRollup。若 GH-823 批准 `error`，它必须跳过 LLM并写 error-level log，即使有 evidence
    也只保留 capture/IR/outcome；若未批准，exact `status:error` 必须由 outer parser
    non-zero拒绝、zero writes、AI=0。每个已批准状态无 evidence 时都按 P-009 blank。状态不能
    改写或隐藏。
11. P-011 经 GH-823 packet amendment 批准的 canonical Stop key（基于 #822 实测并包含或规范化
    `loop_count`）是幂等键：相同 key、status 和 evidence 重放为 no-op；相同 key 却
    status/evidence 冲突时 non-zero、error log、zero new writes，保留先前记录。新的合法 loop key
    产生新 Stop；现有同 session extraction task 仍可作为 scheduling envelope coalesce，但 worker
    必须在任何 prompt/LLM 前按 canonical Stop key 和 status 拆成独立 work item。混合的已批准
    状态不得共用一次 SessionRollup 或统一 AI 决策；只有 GH-823 批准 `error` 时，它才可进入
    mixed-range planning。未批准的 `error` 不能产生 task。不得按内容猜测重复关系。逐消息 ID
    不参与幂等：full snapshot evidence 由 Stop 关联的 `transcript_identity_id`、完整稳定字节
    hash 与 byte length 共同标识；相同 Stop key/status/identity/hash/length 重放 no-op。相同
    Stop key 的 path 解析成不同 identity 时，即使 bytes hash 相同也属于 evidence 冲突；status、
    hash 或 length 不同同样冲突。
12. P-012 full/degraded/blank 完成尝试写入现有 `captured_events` 的内部 outcome event；
    stable event ID 必须绑定 canonical project/session identity 与 source Stop 的 stable event key，
    不得只用可能跨项目重复的 Stop key/evidence/result。blank 使用
    `reason: no_usable_evidence`、空 content 且无 summary ID/range。路径/解析失败只写现有
    `capture_drop_events`，使用稳定 reason、source Stop key、时间和脱敏 locator；不得写没有
    recovery link 的 `raw_ingest_failures`。Full raw insertion 失败必须使整个 bundle rollback/
    retry，不得留下 partial failure row。doctor/session/worker 对同一 semantic key 必须直接按
    固定 semantic priority `full > degraded > blank` 选择 terminal outcome，不按 row id、
    created time 或最后写入选择。blank 先完成后，若同一 approved range 的迟到 payload evidence
    产生 degraded outcome，则 degraded 成为唯一当前态；再有 full 到达则 full 成为唯一当前态。
    反向重放 blank/degraded 都不得降级现态或重复 summary/diagnostic。旧 blank/degraded/drop
    保留为历史，capture drop 可用现有
    `recovered_event_id` 链接 full，不能依赖轮转日志。doctor、CLI 现有
    `raw sessions --json`/`raw messages --json`、本地 `/api/v1/sessions/{id}` 与 context
    session reader 都必须经同一 authoritative outcome
    selector 读取其引用的 summary；不得直接枚举 `session_summaries` 而把旧 degraded 内容重新暴露。
    session-level selector 必须先以 source Stop 的 immutable
    `(captured_events.created_at_epoch, captured_events.id)` 选当前 Stop，再在该 Stop 内应用
    `full > degraded > blank`；不得跨 Stop 先取最高 fidelity。因此早期 Stop 的 full 不能遮蔽
    后续 Stop 的 degraded/blank；同 epoch 仅以 source Stop row ID 稳定打破平局，outcome replay
    始终引用原 Stop 且不能推进顺序。doctor 仍保留每个 Stop 的历史和计数。
    `raw sessions --json` 还必须 union authoritative outcome-only Cursor sessions：无 raw row 的
    degraded/blank 使用 output-only `source_root: "cursor-outcome"`、source Stop epoch、零计数和
    空 samples；`raw messages --json` 对该 locator 返回空 page + 非空 `capture_health`，不能让
    no-evidence session 从 CLI 消失或制造 raw message。`cursor-outcome` 是保留 label：
    用户 `--root cursor-outcome=...` 必须在持久化前被拒绝，绕过 parser 的 required ingest root
    也必须 preflight fail；读取到同名持久 raw/identity row 时 non-zero，不能与虚拟 locator 混合。
13. P-013 transcript 全量验证必须发生在 bundle 的任何 transcript-derived raw/companion write
    前。若 worker
    在 companion 尚未生成时领取 pending task，准备窗口内使用现有 Waiting；期限后必须调用与
    hook full commit 相同的 `commit_cursor_snapshot_bundle`。该 helper 在单一 SQLite
    immediate/serialized transaction 内按 canonical host/project/session、source Stop stable
    event key 与 canonical Stop key 重查 full/degraded companion、执行 full-first 仲裁、为 Full
    条件写 identified raw+companion、为失败条件写相关 capture-drop diagnostics，并
    advance/coalesce task high watermark。事务失败时整个 bundle 零写且task保持可重试；成功后才可
    在事务外 Waiting。worker 可在内存中构造 prompt/LLM 结果，但在任何 summary 或 side-effect
    DB write 前必须取得 immediate writer lock 并重跑 full-first selector；authority 变化则
    zero-write 并 retry full。Cursor 的 authoritative summary、允许的 side effects 与 terminal
    outcome 在该 publish transaction 中一起提交，且 publish 永不写 raw。degraded/blank 不执行
    candidate/workstream/follow-up side effects；full 后到时统一 selector 隐藏旧 degraded summary，
    只让 full 产生记忆侧 side effects。raw archive 使用 bundle transaction 的 savepoint
    rollback；任一
    hook/worker crash injection都必须最终收敛到full、显式
    degraded或可见failed/pending task，禁止孤儿Stop、partial diagnostic bundle和静默丢失。
14. P-014 内部 snapshot/outcome events 不进入普通 prompt、candidate 或 tool evidence，也不递归
    创建 extraction task。对相同合法输入，Claude Code 与 Codex 的 capture、raw archive、
    SessionRollup、doctor 和 session 输出保持不变。

## Stop Status × Loop Matrix

以下矩阵只使用 #822/GH-823 人工批准后的 canonical loop key；未批准前 P-001 阻止实现。

| `status` | 新的合法 loop key | 相同 key + 相同 evidence 重放 | 相同 key + 冲突 status/evidence |
| --- | --- | --- | --- |
| `completed` | 原子写 Stop+pending task；full 或 degraded；有证据调用 LLM，无证据 blank | no-op；不重复 raw/summary/LLM | non-zero、error log、zero new writes |
| `aborted` | 原子写 Stop+pending task并保留失败前证据；有证据调用 grounded LLM，无证据 blank | no-op | non-zero、error log、zero new writes |
| `error` | 原子写 Stop+pending task并保留 capture/IR；始终跳过 LLM并写 error log，内容为空 | no-op；不重复 capture/outcome/logical work | non-zero、error log、zero new writes |
| 缺失/错误/未批准的 status 或 loop 形态 | 由 GH-823/#822 外层 fail-closed；zero writes | 不适用 | 不适用 |

PR #914 在 completed/aborted Stop 上都观察到数字 `loop_count: 0`；completed 带 token
字段，aborted 不带。该证据不证明非零、缺失或 `null`。若后续 #822 证明 `loop_count`
可缺失或为 `null`，GH-823 packet amendment 与人工批准必须同时冻结
唯一规范化 sentinel 和幂等规则；实现不得自行把缺失值当 `0`。amendment 未批准即阻断 GH-825。

## Transcript Failure Matrix

| `transcript_path` / 输入状态 | 行为 | fidelity / 诊断 |
| --- | --- | --- |
| 字段缺失或 `null` | Stop 入账；有 payload 则 payload-only，否则 blank | `degraded/path_absent` 或 `blank/no_usable_evidence` |
| 字符串仅含空白 | Stop 入账；不得当 cwd/环境变量；有 payload 则 payload-only，否则 blank | `degraded/path_blank` 或 `blank/no_usable_evidence` |
| wrong type | GH-823 schema failure，non-zero、zero writes | error log；不得写 raw payload |
| 相对、越界、穿越、symlink/reparse、非普通文件 | 不读取；有 payload 则 payload-only，否则 blank | `degraded/path_untrusted`（或批准子类）/ `blank/no_usable_evidence` |
| 缺失、权限拒绝、短读、读取期间变化 | 不提交 transcript-derived rows；有 payload 则 payload-only，否则 blank | `degraded/read_failed` 或 `degraded/snapshot_changed` / `blank/no_usable_evidence` |
| 超过 #822 批准上限 | 不截断冒充完整；有 payload 则 payload-only，否则 blank | `degraded/snapshot_too_large` / `blank/no_usable_evidence` |
| 格式/版本/尾部/任一记录错误（不含 PR #874/#914 正例缺少未观察的逐消息 ID/timestamp/tool_result） | 全输入作废；零 transcript-derived writes；有 payload 则 payload-only，否则 blank | `degraded/format_invalid` 或 `degraded/format_unknown` / `blank/no_usable_evidence` |
| 全量验证成功但无 usable transcript evidence | 有 payload 则 payload-only，否则 blank | `degraded/transcript_empty` / `blank/no_usable_evidence` |
| 全量验证成功且有 usable evidence | 同一 IR 驱动 raw archive 与 prompt | `full` |
| worker absence check 后 hook 先提交 full | worker 在 serialized transaction 内重查并选择 full；零 degraded/diagnostic 写入 | `full` |
| worker 先提交 interrupted degraded，hook 后提交已验证 full | hook 原子提交 full、链接可恢复 drop并推进task；loader/outcome只选 full | 当前 `full`；旧 degraded仅诊断可见 |

## 验收标准

- [ ] #822 与 GH-823/GH-825 人工门有 exact-head 可追溯证据；未通过时实现保持 blocked。
- [ ] P-001..P-014、两张矩阵每一行都有确定性自动化验证。
- [ ] 使用 PR #874 与 PR #914 exact-head 的真实脱敏 fixture，而非手写猜测 Cursor
      格式、路径或 `loop_count`；未实测可信根/path/max 继续 blocked。
- [ ] PR #874 exact-head Cursor 3.6.31 两行 `{role,message}` fixture 是 parser 正例：无逐消息
  ID/timestamp 仍全量验证成功，IR/prompt 严格保持文件行序，且不合成未观察字段；该正例不等于
  完整 grammar 已获批准。
- [ ] PR #914 exact-head Cursor 3.12.17 `{role,message}`、assistant `tool_use` 和独立
  `turn_ended` fixture 是 versioned parser 正例：物理 ordinal 包含所有 record，raw message
  ordinal 可有空洞，`turn_ended` 不进入 raw/prompt，且 parser 不要求未观察的独立
  `tool_result`；该正例不等于跨版本完整 grammar 已获批准。
- [ ] missing/null/blank/nonexistent/unreadable/untrusted/oversized/corrupt/changed 路径均显式降级，
  Stop 与此前 payload 不丢失，且没有部分 raw archive 被标记 full。
- [ ] malformed/over-limit tool payload 按 GH-823 non-zero + zero writes；合法未知工具原名入 ledger。
- [ ] `commit_cursor_snapshot_bundle` 是 hook full-IR commit 与 worker interrupted recovery 共用的
  production helper；在单一 immediate transaction 内完成 semantic-key重查、full-first仲裁、
  identified raw occurrences、companion、相关capture_drop和task
  high-watermark bundle；SessionRollup publish 的 raw-write spy 必须为零。
- [ ] crash injection覆盖 bundle 的重查后、blob/event、capture_drop、task
  coalesce、commit 前后以及事务外 Waiting；commit前任一点全回滚，commit后即使Waiting失败也由
  lease recovery重试且bundle不丢。
- [ ] 精确并发测试覆盖“worker absence check → hook full commit → worker recovery”和“worker
  degraded commit → hook full commit”两种提交顺序；full始终是loader/outcome唯一内容选择。
- [ ] 每个 Stop 的 terminal selector固定为 `full > degraded > blank`，覆盖blank→degraded迟到
  evidence、degraded→blank反向到达、blank→degraded→full及三种outcome replay；
  session-level先选immutable当前source Stop再选fidelity，覆盖earlier-full/later-blank、
  earlier-blank/later-full、same-epoch tie与replay不推进；结果不依赖outcome row/time/latest。
- [ ] `cursor-outcome` 保留给output-only locator；CLI/user root与绕过parser的required ingest
  root均在写前拒绝，持久化同名collision读取失败，default roots不受影响。
- [ ] `completed|aborted` × new/replay/conflict loop key 矩阵始终通过：有evidence时AI=1，
  无evidence时AI=0且capture不丢失。若 GH-823 批准 `error`，再覆盖其
  new/replay/conflict、始终AI=0、error log及capture保留；若未批准，exact `status:error`
  non-zero、zero writes、AI=0。
- [ ] 相同 canonical Stop key/status/transcript identity/snapshot hash+length 重放为零新增；同 key
  但 path identity、status、稳定 snapshot bytes hash 或 length 改变时为冲突、non-zero、error
  log、zero new writes，既有记录不变。
- [ ] full raw/prompt 来自同一规范 IR；Full构造前已在安全descriptor/boundary下固定trusted
  source root、normalized path与identity ID，后续不从Stop重推。同路径 appended snapshot 的旧
  ordinal 重放 no-op、新 ordinal 新增，相同文本不同 ordinal 分别持久化，既有 ordinal 稳定字段
  变化使整包回滚；worker spy 证明不重读原路径且 SessionRollup publish 对 raw tables 零写入，
  内部 events 不进入 prompt。
- [ ] `raw reconcile` 对同identity的多个authoritative Full companions验证单调prefix chain并选择
  最长approved boundary后，复用同一 Cursor parser/physical-ordinal projection；不得把 Cursor
  record 交给 Claude/Codex classifier。fork、truncation、mutated prefix、companion metadata
  缺失/冲突或任一boundary hash/length不符时显式失败且保持 read-only/aggregate-only。
- [ ] doctor 区分当前 full/degraded/blank，显示稳定 reason/time/locator；blank 内容为空且无
  summary ID/range；历史 capture drop 保留且 recovery link 后不继续计作当前 actionable failure；
  Cursor 降级不新增 `raw_ingest_failures`。
- [ ] schema snapshot 完全不变，没有 migration、新表、新列或新索引。
- [ ] 完整 Claude/Codex 测试通过，且现有合法输入输出不变。

## 边界情况

- Cursor 在 Stop 时仍追加、替换或删除 transcript。
- worker 在 absence check 后与 hook full bundle 交错提交，或 degraded bundle 先于同一 Stop 的
  已验证 full bundle 提交。
- 同一会话多个 Stop 共用路径但 loop key 不同，或同一 key 冲突重放。
- transcript record 没有逐消息 ID/timestamp，或包含按物理 record ordinal 必须分别保留的同
  role 同文本重复项。
- 同一可信 transcript path 被多个合法 Stop snapshot 复用：append 必须匹配所有既有approved
  prefix boundaries并只增加新 ordinal；fork、truncation或替换既有ordinal稳定字段必须冲突，
  不能因 content hash 相同/不同而合并或静默覆盖。
- worker 在 raw archive、summary checkpoint、follow-up checkpoint 或 task Done 前后崩溃。
- 未知 Cursor 工具返回 malformed、超限或含密钥的 JSON-stringified payload。
- Windows 路径、可信根、transcript max、`loop_count` 非零/缺失/null/乱序语义和
  `status: error` 未被 #822/PR #914 证明。

## 发布说明

这是新增 Cursor host 的受控能力。发布说明必须标注 #822/GH-823 版本合同、人工冻结的 transcript
上限、completed/aborted grounded总结、error保留但跳过LLM、Stop/task原子性、显式
degraded/doctor语义，以及不新增schema、不改变Claude/Codex。#824 installed-hook E2E 未通过前
不得宣称 Cursor 自动捕获可用。
