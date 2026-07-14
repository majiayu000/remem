# Tech Spec

## Linked Issue

GH-818

## Product Spec

Product: `product.md`

## Codebase Context

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| Job enqueue | `src/db/job/enqueue.rs:29`, `src/db/job/enqueue.rs:87` | `enqueue_job` 先 SELECT active row 再 INSERT；`maybe_enqueue_dream_job` 另有 project-wide inflight 查询、pending profile 更新和 recent-done cooldown，但两条路径都没有数据库唯一约束保护。 | 这是 ordinary/Dream 并发重复与 Dream disposition 失真的根因。 |
| Job state transitions and lease recovery | `src/db/job/state.rs:4`, `src/db/job/state.rs:21`, `src/db/job/state.rs:48`, `src/db/job/state.rs:65`, `src/db/job/state.rs:85` | lease-owned UPDATE 只按 id/owner 过滤，忽略 affected rows；`mark_job_failed_or_retry` 还在 UPDATE 前单独读取 attempt counters；expired lease 通过一次 bulk UPDATE 全部回到 pending。 | wrong owner、expired/reclaimed lease 和 read-update race 可被误报成功；CompileRules predecessor 与既有 successor 相遇时 bulk release 会撞上新 slot 约束并冻结本轮其他 recovery。 |
| Claim ordering | `src/db/job/claim.rs:6` | pending job 按 `priority ASC, created_at_epoch ASC, id ASC` 取第一条并 claim，没有排除同 project 已有 processing predecessor 的 CompileRules successor。 | 必须保留全局顺序，同时跳过暂不可 claim 的 successor 并继续选择下一条 eligible job。 |
| Queue schema | `src/migrations/v001_baseline.sql:142`, `src/migrations/v001_baseline.sql:257`, `src/migrations/v003_host_identity.sql:4` | `jobs` 有 claim/project/lease/identity 普通索引，没有 active identity UNIQUE 约束；`session_id` 可为 NULL，host 自 v003 起非空。 | 新不变量必须在 SQLite 层跨进程成立，并显式规范化 NULL session。 |
| Failure lifecycle | `src/migrations/v057_failure_lifecycle.sql:16`, `src/db/failure_lifecycle.rs`, `src/db/failure_lifecycle/maintenance.rs:93`, `docs/specs/failure-lifecycle/TECH.md` | failed jobs 已有 class、failed/archive timestamps、bounded recovery 与 status/doctor 统计；due jobs 当前以一个 bulk UPDATE requeue。 | active identity UNIQUE 生效后，预期 collision 必须逐行收敛并保留来源审计，不能让一条 collision 回滚无关 recovery，也不能反复自动重试同一来源 row。 |
| Summary retirement | `src/migrations/v064_reject_legacy_summary_jobs.sql:31`, `src/worker/job.rs:21` | v064 将 non-terminal/retryable Summary 永久失败；worker 拒绝执行 Summary。 | v069 不得把 Summary 选成 active survivor 或重新 enqueue。 |
| Migration runner | `src/migrate/run.rs:9`, `src/migrate/types.rs:343`, `src/migrate/schema_drift/invariants.rs:769` | migrations 在 `BEGIN IMMEDIATE` 中串行、失败时整体 ROLLBACK；当前最新版本为 v068，schema drift 会验证声明对象。 | reconciliation 与唯一索引创建可作为一个 all-or-nothing v069 事务。 |
| Worker truth | `src/worker.rs:156`, `src/worker.rs:175` | worker 先处理副作用，再调用 state transition；`done id=...` 位于 `mark_job_done` 返回之后。 | transition error 必须在成功日志前中断，并以 error 级别带出诊断。 |
| Shared status/doctor stats | `src/db/query/stats.rs:253`, `src/doctor/database.rs:52`, `src/cli/actions/query/status.rs:186`, `src/cli/actions/query/status.rs:605` | status、status JSON 和 doctor 共用 persisted pending/processing/failed/stuck 计数。 | 保持共享数据库事实源即可证明失败转换未成为成功；迁移 conflict 进入 actionable failed jobs。 |
| Existing tests | `src/db/job/tests.rs:25`, `src/db/job/tests.rs:98`, `src/migrate/tests.rs:288`, `src/db/query/stats/tests.rs:253`, `src/doctor/tests.rs:158` | job dedup/CompileRules 仅有单连接测试；migration 已有 file-backed WAL + Barrier 模式；stats/doctor 已有共享计数 fixture。 | 新测试复用现有结构，但必须使用两个真实连接证明跨进程不变量。 |

## 设计方案

### 1. Lease-owned transition 使用单事务 CAS

在 `src/db/job/state.rs` 收口一个内部 transition helper，所有以下路径必须经过它：

- `mark_job_done`；
- `mark_job_failed`；
- `mark_job_exhausted`；
- `mark_job_failed_or_retry` 的 retry 与 terminal 两个分支。

每次调用只取一次 `now_epoch`，并在短 `IMMEDIATE` transaction 内完成：

1. 以参数化 SQL 执行 guarded UPDATE；guard 必须同时包含 job id、
   `state='processing'`、expected `lease_owner`、非空 lease expiry，以及
   `lease_expires_epoch >= now_epoch`。该边界与当前 stuck/recovery 的
   `lease_expires_epoch < now` 保持一致。
2. UPDATE 以 affected-row/`RETURNING` 结果作为唯一成功依据。恰好一行才 COMMIT；零行不
   得当作幂等成功；超过一行按不变量破坏返回错误。
3. 零行时仍在同一 write transaction 中读取 id 对应的 current state、owner 和 expiry，
   构造包含 job id、expected owner 与 current snapshot 的错误；missing row 使用显式
   `current=missing`。随后 ROLLBACK，保证诊断读取不参与授权，也不会与另一个 writer
   交错形成 TOCTOU。
4. `mark_job_failed_or_retry` 在同一 transaction 中读取并验证当前 lease 后计算
   `next_attempt`、classification 和 retry/terminal 分支，再执行仍带完整 guard 的 UPDATE。
   不保留当前“无 lease guard 的 attempt SELECT + 独立 UPDATE”窗口。

state helper 不把 transition conflict 写进目标 job 的 `last_error`，因为 `B-002` 要求失败
时 row 完全不变。worker 在 error 级别记录返回诊断并向上返回；持久化状态本身继续通过
processing/stuck 计数表达真实结果。

### 2. 数据库 active identity 约束

v069 在 reconciliation 后创建三个 partial UNIQUE expression indexes。三类 identity 必须
与 application lookup 使用同一闭集定义：

1. Ordinary：`job_type NOT IN ('dream', 'compile_rules')`，active predicate 为
   `state IN ('pending', 'processing')`，key 为
   `(host, job_type, project, COALESCE(session_id, ''))`。这保留 host isolation，并让 NULL 与
   empty session 等价。
2. Dream：`job_type='dream'` 且 active，key 为 `(project)`。host 和 session 不参与 key，
   保留当前 project-wide、cross-host coalescing；生产 enqueue 继续写 NULL session。
3. CompileRules：`job_type='compile_rules'` 且 active，key 为 `(project, state)`。因此同一
   project 最多各有一条 pending 与 processing，恰好表达 one pending successor 例外。

建议稳定对象名为：

- `idx_jobs_active_ordinary_unique`；
- `idx_jobs_active_dream_unique`；
- `idx_jobs_active_compile_rules_unique`。

三个 index 都加入 `src/migrate/schema_drift/invariants.rs`。application 侧使用穷举
`JobType` 的 identity classifier；新增 job type 必须先明确归入 ordinary 或新增获批例外，
避免 schema 与 lookup 漂移。`JobType::Summary` 仅保留历史读取能力，enqueue 入口必须明确
拒绝创建新 Summary。

### 3. Atomic enqueue 与 canonical id

将 enqueue 分为 public transaction wrapper 与 transaction-scoped core：

- wrapper 对无外层 transaction 的调用开启短 `IMMEDIATE` transaction；
- 已在持久化 transaction 中的调用（例如 SessionRollup follow-up）调用 core，并要求外层
  在进入 core 前已取得 write lock；不得尝试嵌套 `BEGIN`；
- ordinary core 在 write transaction 内查询 canonical active row；不存在时 INSERT，并以
  INSERT 返回的 row id 为 `Enqueued`；unique conflict 只可按对应 identity 重新读取
  canonical row，不能用 broad `INSERT OR IGNORE` 吞掉 NOT NULL/CHECK/其他约束错误；
- conflict 后 canonical row 已 terminalized 或不可读取时，整个操作明确失败。调用方可重新
  发起一次新的 enqueue；本次不得返回不存在的 id。

Dream core 在同一 transaction 中依次处理 active row、recent done 和 insert：

1. active Dream 存在时返回 `CoalescedInflight(id)`；若它仍 pending 且 incoming profile 与
   现有不同且 incoming profile 非空，沿用当前行为更新 host/payload，并把 priority 收敛为
   两者最小值；空 profile、相同 profile 或 processing Dream 均不改写 payload/host/priority；
2. 无 active row 但 cooldown 内有 done row 时返回 `SuppressedRecentDone(id)`；
3. 否则插入并返回 `Enqueued(id)`；
4. 若唯一约束表明另一个 writer 已创建 active row，必须读取并返回
   `CoalescedInflight(id)`，不得映射成 `Enqueued`。

该事务边界既提供准确 disposition，也使 session rollup 持久化的 Dream attribution 与实际
job id 一致。数据库 UNIQUE index 是最终防线，transaction wrapper 负责稳定 canonical id、
payload update 与返回语义；二者不能互相替代。

### 4. CompileRules claim、retry 与 expired recovery

`src/db/job/claim.rs` 的 candidate query 保持
`priority ASC, created_at_epoch ASC, id ASC`，但 eligible predicate 必须排除“同 project 已存在
processing CompileRules”的 pending CompileRules successor。查询必须继续扫描并 claim 下一条
eligible job，不能因全局顺序最前的 successor 暂不可运行而返回空或阻塞 ordinary/Dream/其他
project 的 CompileRules。最终 conditional UPDATE 重复同一 eligible predicate，避免 candidate
选择后 predecessor 状态变化造成越权 claim。

`src/db/job/state.rs` 将 processing CompileRules 回到 pending 的 retry 与 expired-lease release
纳入 collision-aware transaction：

1. 同 project 不存在 pending successor 时，retry 按第 1 节的 guarded transition 正常把
   predecessor 转回 pending；expired recovery 也只转换当前那一行。
2. 已存在 pending successor 时，不把 predecessor 更新成第二条 pending，也不依赖触发 UNIQUE
   error 回退。same transaction 保留 successor 为唯一 pending canonical；其
   `next_retry_epoch=max(existing_next_retry_epoch, computed_retry_epoch)`；worker retry 的
   `computed_retry_epoch=now+backoff`，expired release 的值为 recovery `now`，因此 successor
   不会早于本次应有的 ready time 运行。priority 取现有 successor 与 predecessor 中更高的
   调度优先级（数值较小者）。payload 仍由 successor 表示，不能改写已失败 predecessor 的
   执行历史。
3. predecessor 原地终结为 `failed` historical evidence：worker retry path 写入本次已发生
   执行对应的 `next_attempt=attempt_count+1`；expired-lease recovery 没有观察到一次新的执行失败，
   保持当前 `attempt_count`。两条路径都不得为了阻止重试而写成 `max_attempts`；设置
   `failure_class='permanent'`、`next_retry_epoch=0`、清空 lease 且保留或首次设置
   `failed_at_epoch`，即可确保不再进入 auto-retry，同时保留真实 attempt evidence。
   `last_error` 以本次 worker retry error（expired recovery 则以 source 当前
   `last_error`，缺失时使用固定 `expired lease`）的既有截断文本作为主证据，并在 2000-char
   上限内确定性追加固定、非 secret 的
   `[compile_rules_retry_coalesced_to_successor id=<canonical_id>]` marker：先为完整 marker 预留
   空间，再截断原错误并追加 marker，禁止用 marker 覆盖原错误。state API 返回包含 source id、
   canonical id 与 identity kind 的 structured coalesced result；worker 只用这些 safe fields 和
   固定 marker 记录结果，不得输出原始 error 文本，也不得记录 done/retry success。
4. permanent、exhausted 或 done terminal transition 不消费、不改写 pending successor；
   predecessor terminal 后 successor 才成为可 claim，并按自身 ready time 运行。
5. `release_expired_job_leases` 先取 bounded、稳定排序的 expired candidates，再为每一行使用
   独立 transaction/savepoint 执行上述普通 release 或 CompileRules collision 收敛。一条
   project collision 必须提交为可审计结果并继续下一行；unexpected SQL/IO/schema error 明确
   返回错误，但不能把已经独立提交的无关 recovery 伪装成未发生。

所有 collision 查询与写入均用参数化 SQL，并在同一 write transaction 内再次验证 source
仍为目标 processing row、lease 确已过期（expired path）或仍归 expected owner 且未过期
（worker retry path），以及 successor 仍为同 project pending CompileRules。

### 5. v069 deterministic reconciliation

新增 `src/migrations/v069_job_queue_atomicity.sql`，由现有 migration runner 的
`BEGIN IMMEDIATE` 包住“冻结写入 → reconcile → validate → create indexes → mark applied”。
任一步失败均 ROLLBACK，不留下部分 terminalization 或部分 index。

只处理 `pending|processing` rows；既有 `done`、`failed`、failure class、archive marker、
attempt/error/timestamp 全部不改。先执行 active-only 的 Summary retirement pre-pass：仅将
由 pre-upgrade 进程晚写入的 `pending|processing` Summary 变为 permanent failed，使其不参与
survivor 选择。这里不得直接重放 v064 的完整 predicate，因为 v064 还会重写 retryable
`state='failed'` Summary；v069 必须保留所有 terminal Summary 的原始审计字段。

Canonical 规则固定如下，解决 Product Spec 的两个开放问题：

- Ordinary 与 Dream 每个 identity 只保留一个 active slot。若存在 processing，优先选择
  lease 未过期者，再按 `lease_expires_epoch DESC, updated_at_epoch DESC, id DESC` 选一条；
  若全部已过期，仍按同一稳定次序选一条，升级后交给 collision-aware stuck recovery。
- 无 processing 的 ordinary group 按现有 claim 顺序
  `priority ASC, created_at_epoch ASC, id ASC` 选择 pending survivor。
- Dream group 若存在 processing survivor，保留该 row 的 payload/host/priority 原值，任何
  pending duplicate 都不得改写它。若只有 pending duplicates，则按
  `(created_at_epoch ASC, id ASC)` 稳定重放：最早 row 是 base，此后逐条解析 incoming/current
  profile，仅当 incoming profile 非空且不同于 current profile 时，才用 incoming snapshot
  替换 survivor 的 payload/host，并把 priority 取当前值与 incoming 值的最小值；incoming
  profile 为空或与 current 相同时，payload/host/priority 全部不变。其他 Dream rows 保留各自
  原 payload 作为 terminal history。fixtures 必须覆盖 profiled→empty、empty→profiled、
  same-profile/lower-priority、different-profile 与 processing+pending。
- CompileRules 分两个 slot：processing 按上述 processing 次序保留一条；pending 按
  `priority ASC, created_at_epoch ASC, id ASC` 保留一条 successor。两者可同时存在。

每个非 survivor active row 原地转为：`state='failed'`、lease fields=NULL、
`attempt_count=max(attempt_count,max_attempts)`、`next_retry_epoch=0`、
`failure_class='permanent'`、`failed_at_epoch=COALESCE(failed_at_epoch, migration_now)`、
`archived_at_epoch=NULL`。`last_error` 不得覆盖既有错误：固定、非 secret 的 suffix marker 为
`[job_queue_atomicity_migration_duplicate duplicate_id=<duplicate_id> canonical_id=<canonical_id>
identity_kind=<kind> manual_review=<true|false>]`。既有非空 `last_error` 的截断文本保持为主证据；
先为完整 marker 预留空间，再确定性保留 existing error 的前
`2000-marker_length` 个字符并追加 marker，最终总长不得超过 2000。只有 existing error 为
NULL/empty 时才单独存 marker。marker 不包含 payload、project 内容或凭据；migration 日志
只报告计数与 marker 中的非 secret metadata，绝不输出既有 error。多个 processing owner 或
Dream payload 分歧标记 `manual_review=true`；普通 pending 重复为 false。

reconciliation 后先运行三类 duplicate-count assertions；通过临时 CHECK guard 或等价的
SQLite-valid 约束语句让任一非零结果直接使 migration 失败，而不依赖只能在 trigger 中使用
的 `RAISE()`。成功后在同一 transaction 创建三个 indexes。迁移日志报告各 identity kind
的 reconciled 数量与 manual-review 数量，不打印 payload。再次启动看到 v069 已 applied
时不重跑，因此 survivor 与历史不会漂移。静态 SQL 不能承担日志输出：v069 SQL 执行成功后、
`mark_applied` 前，`src/migrate/run.rs::run_post_migration_hook`（或等价 Rust hook）在同一
migration transaction 内按固定 marker 查询各 identity kind 的 reconciled 数量与
`manual_review=true` 数量，只输出 kind/count，不读取或打印 payload/project/既有 error。
hook 查询或日志准备失败必须返回错误并使整个 v069 回滚；focused migration test 捕获日志，
证明计数准确且不含 fixture secrets。

### 6. Failure lifecycle 的 identity-aware auto-recovery

`src/db/failure_lifecycle/maintenance.rs::requeue_due_jobs` 不再用一个 bulk UPDATE 更新最多 25
条 rows。它先按既有 due order 读取 bounded candidate ids，然后逐条在独立 transaction 或
savepoint 内重新验证 eligibility，并按与 enqueue 完全相同的 identity classifier 处理：

- 没有 active identity：source failed row 正常 requeue 为 pending。
- Ordinary 已有 active：active row 吸收待执行工作；source 保持 failed/auditable，保留真实
  `attempt_count`，设置 `failure_class='permanent'`、`next_retry_epoch=0` 与固定、非 secret 的
  `[auto_recovery_coalesced_to_canonical id=<canonical_id>]` marker，使其永久不再符合 auto-retry。
  source 既有 `last_error` 仍是主证据：为完整 marker 预留空间，把既有 error 确定性截断到
  `2000-marker_length` 后追加 marker；不得只保留 marker 或覆盖原 error。source id、payload、
  failed timestamp 与其他 audit fields 保留；不删除、不归档、不伪装成 done。
- Dream 已有 pending：以 source snapshot 作为 incoming，应用第 3/5 节同一 profile predicate；
  仅非空且不同 profile 可更新 canonical pending payload/host 并降低 priority。Dream 已
  processing 时不改写它；两种情况 source 都按上述 coalesced failed evidence 终结。
- CompileRules 已有 pending：使用它作为 canonical successor，source 保持 coalesced failed
  evidence。若只有 processing predecessor，则把 source requeue 为唯一 pending successor；
  它在 predecessor terminal 前由第 4 节 claim predicate 排除。

预期 identity collision 是该 candidate 的成功收敛结果，提交后继续下一 candidate；不得回滚
同 batch 已处理或后续无关 recovery。unexpected busy/SQL/schema/读取错误仍 fail loudly，并
保留当前 source row；不得把未知错误误分类成 coalescing。返回计数/日志区分 `requeued` 与
`coalesced`，只记录 ids、identity kind 与非 secret 诊断，不输出 payload/project 内容。

### 7. Worker、status 与 doctor truth

- `src/worker.rs` 保持“transition 成功后才写 success log”的顺序；为四类 transition error
  增加统一 error context，包含 job id/type/project hash 或安全 project label、expected owner
  和 state helper 返回的 current lease snapshot。error 必须传播，不能继续打印 retry/done
  成功文案。
- 不新增“副作用执行过即成功”的状态，也不新增平行 transition ledger。CAS 失败时 row
  不变；`query_system_stats` 因而继续把它计入 processing，并在 expiry 后计入 stuck。
- v069 产生的 redundant failures 使用现有 non-archived permanent failure lifecycle，自动
  进入 actionable failed jobs；auto-recovery collision 留下的 permanent coalesced source 也
  继续作为 failed audit history 显示，但不会反复成为 due candidate。status JSON、CLI status
  和 doctor 继续从共享 stats 显示。
- 在 shared stats、CLI status JSON/text、doctor fixture 中加入回归断言，而非复制新的统计
  SQL。若实现需要显示 migration-conflict 子计数，只能从固定 migration duplicate suffix
  marker 在共享 stats 层派生，并同步所有三个消费者；本期最低契约不要求增加公开字段。
- 更新 `docs/specs/failure-lifecycle/PRODUCT.md` 与 `TECH.md`，记录 lease transition conflict
  不改写 job row、migration duplicates 作为 permanent actionable failures 进入既有生命周期，
  以及 auto-recovery collision 保留来源真实 attempt evidence、由 canonical active work 承接
  执行；不修改该 contract 的 retention 或 cleanup 规则。

### 8. 两连接并发测试结构

`src/db/job/tests.rs` 增加 file-backed temp database helper：两个独立 `Connection` 都启用
WAL、foreign keys 和 30s busy timeout；两个线程在 `Barrier` 后调用真实 public enqueue，
主线程最后用第三连接检查 rows。禁止用同一 connection、共享 transaction 或进程内 mutex
模拟并发。

至少包含两条相互独立的 barrier tests：

- `enqueue_job_two_wal_connections_coalesce_ordinary_identity`：两个 caller enqueue 相同
  Compress identity；两个结果 id 相同，active count=1。
- `compile_rules_two_wal_connections_share_one_pending_successor`：预置一条 processing
  CompileRules 后，两个 caller 并发 enqueue；两个结果等于同一 pending successor，最终
  `(processing,pending)=(1,1)`。

另加 `dream_two_wal_connections_coalesce_across_hosts`，用不同 host 和不同 profile/priority
并发请求同一 project；active count=1，返回值只有一个 `Enqueued`、一个
`CoalescedInflight`，最终 payload/priority 符合 transaction serialization 后的现有更新规则。
再用无 processing 的 CompileRules barrier case 证明首次 enqueue 只有一条 pending。

除 enqueue barriers 外，增加以下精确的状态/maintenance tests：

- `claim_next_job_skips_compile_rules_successor_while_predecessor_processing`；
- `claim_next_job_continues_to_unrelated_eligible_job`；
- `compile_rules_retry_collision_coalesces_to_pending_successor`；
- `release_expired_compile_rules_collision_preserves_unrelated_job_progress`；
- `v069_replays_pending_dream_duplicates_with_current_profile_predicate`，用五组 fixture 覆盖第 5
  节列出的顺序，并由
  `v069_does_not_rewrite_processing_dream_payload` 单独锁定 processing case；
- `v069_preserves_existing_duplicate_last_error_and_appends_marker`，证明既有 error prefix 与完整
  migration marker 同时保留；
- `v069_truncates_near_limit_duplicate_last_error_without_losing_marker`，使用接近/达到 2000-char
  的既有 error，证明确定性截断后总长不超过 2000、error prefix 与完整 marker 仍保留；
- `failure_lifecycle_auto_recovery_coalesces_mixed_active_identities_per_row`，同一 bounded batch
  同时包含 ordinary active、Dream pending、Dream processing、CompileRules pending、仅有
  CompileRules processing 以及无 collision row，验证每条 source/canonical 与 batch progress。

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| `B-001` exact current/unexpired lease and exactly one row | `src/db/job/state.rs` CAS helper | `cargo test --no-default-features lease_owned_job_transitions_require_current_unexpired_lease -- --nocapture` |
| `B-002` rejected transition leaves row unchanged | `src/db/job/state.rs`, `src/db/job/tests.rs` | `cargo test --no-default-features rejected_job_transition_preserves_every_persisted_field -- --nocapture` |
| `B-003` complete zero-row/missing diagnostics | state helper diagnostic snapshot | `cargo test --no-default-features job_transition_error_reports_expected_and_current_lease -- --nocapture` |
| `B-004` ordinary active identity and terminal reuse | v069 ordinary UNIQUE index, ordinary classifier | `cargo test --no-default-features ordinary_job_identity_normalizes_null_session_and_allows_terminal_history -- --nocapture` |
| `B-005` Dream cross-host identity/update/cooldown/disposition | Dream transaction core, Dream UNIQUE index, v069 stable replay | `cargo test --no-default-features dream_two_wal_connections_coalesce_across_hosts -- --nocapture` and `cargo test --no-default-features v069_replays_pending_dream_duplicates_with_current_profile_predicate -- --nocapture` |
| `B-006` one CompileRules processing plus one unclaimable successor and collision-safe recovery | CompileRules UNIQUE index, enqueue core, `claim.rs`, retry/expired recovery, safe worker result logging | `cargo test --no-default-features claim_next_job_skips_compile_rules_successor_while_predecessor_processing -- --nocapture`; `cargo test --no-default-features compile_rules_retry_collision_coalesces_to_pending_successor -- --nocapture`; `cargo test --no-default-features worker_compile_rules_retry_collision_logs_safe_coalesced_result -- --nocapture`; `cargo test --no-default-features release_expired_compile_rules_collision_preserves_unrelated_job_progress -- --nocapture` |
| `B-007` ordinary concurrent canonical id | ordinary transaction wrapper | `cargo test --no-default-features enqueue_job_two_wal_connections_coalesce_ordinary_identity -- --nocapture` |
| `B-008` concurrent CompileRules successor/initial enqueue | CompileRules transaction core | `cargo test --no-default-features compile_rules_two_wal_connections_share_one_pending_successor -- --nocapture` and `compile_rules_two_wal_connections_create_one_initial_pending` |
| `B-009` compatible deterministic duplicate reconciliation, including Dream serialized semantics and existing error evidence | `v069_job_queue_atomicity.sql`, migration fixtures | `cargo test --no-default-features v069_reconciles_active_job_duplicates_before_unique_indexes -- --nocapture`; `cargo test --no-default-features v069_replays_pending_dream_duplicates_with_current_profile_predicate -- --nocapture`; `cargo test --no-default-features v069_does_not_rewrite_processing_dream_payload -- --nocapture`; `cargo test --no-default-features v069_preserves_existing_duplicate_last_error_and_appends_marker -- --nocapture`; `cargo test --no-default-features v069_truncates_near_limit_duplicate_last_error_without_losing_marker -- --nocapture` |
| `B-010` terminal/history preservation and idempotent applied migration | v069 terminal exclusion, migration registry | `cargo test --no-default-features v069_preserves_terminal_job_history_and_is_idempotent -- --nocapture` |
| `B-011` no success signal after transition error | `src/worker.rs`, worker log fixture | `cargo test --no-default-features worker_transition_conflict_logs_error_without_done_or_retry_success -- --nocapture` |
| `B-012` persisted truth plus isolated identity-aware auto-recovery | `failure_lifecycle/maintenance.rs`, shared stats/status/doctor fixtures | `cargo test --no-default-features failure_lifecycle_auto_recovery_coalesces_mixed_active_identities_per_row -- --nocapture`; `cargo test --no-default-features failure_lifecycle_auto_recovery_preserves_source_attempt_count -- --nocapture`; `cargo test --no-default-features lease_transition_failure_remains_visible_in_status_and_doctor -- --nocapture` |
| `B-013` Summary remains retired | v069 Summary pre-pass, existing v064/worker guards | `cargo test --no-default-features legacy_summary_upgrade_rejects_non_terminal_jobs -- --nocapture` and `worker_rejects_legacy_summary_job_without_retry` |
| `B-014` busy/conflict/diagnostic/migration errors fail closed | enqueue/state transaction error tests, migration rollback fixture | `cargo test --no-default-features job_queue_atomicity_errors_roll_back_without_assumed_success -- --nocapture` |

## 数据流

### Enqueue

```text
caller
  -> public IMMEDIATE transaction wrapper
  -> classify ordinary | Dream | CompileRules | retired Summary
  -> lookup/update/insert through transaction-scoped core
  -> SQLite partial UNIQUE index enforces active identity
  -> return persisted canonical id + accurate Dream disposition
  -> COMMIT
```

### Worker transition

```text
claimed Job + expected owner
  -> process business side effect
  -> IMMEDIATE transition transaction
  -> guarded UPDATE(state=processing, owner match, unexpired lease)
     -> one row: COMMIT -> success log
     -> zero rows: same-transaction diagnostic snapshot -> ROLLBACK -> error log/return
  -> status/doctor later read persisted row, never an in-memory success flag
```

### Claim and CompileRules recovery

```text
claim_next_job
  -> scan ready jobs in global order
  -> skip pending CompileRules whose project has processing predecessor
  -> claim next eligible row, or none

processing CompileRules retry / expired release
  -> per-row write transaction validates source
  -> no successor: guarded transition to pending
  -> successor exists: delay/prioritize canonical successor + terminalize source as permanent coalesced failed evidence
     -> worker retry stores next_attempt; expired recovery preserves current attempt_count; neither writes max_attempts
  -> commit one project; continue unrelated expired candidates
```

### Failure auto-recovery

```text
select bounded stable due candidate ids
  -> isolated transaction/savepoint per source
     -> no active identity: requeue source
     -> expected collision: converge to canonical + mark source permanent with auditable diagnostic while preserving its stored attempt_count
     -> unexpected DB error: rollback current source and return error loudly
  -> one expected collision never rolls back unrelated recoveries
```

### Upgrade

```text
foreground open_db
  -> BEGIN IMMEDIATE migration v069
  -> retire any late active Summary rows
  -> rank active groups and retain deterministic allowed slots
  -> mark redundant active rows permanent failed with non-secret diagnostics
  -> assert zero remaining duplicate groups
  -> create three partial UNIQUE indexes + mark schema object invariants
  -> Rust post-migration hook logs reconciled/manual-review counts from safe markers
  -> COMMIT all, or ROLLBACK all
```

不新增外部调用、网络请求或 secret storage。所有 SQL 参数化；migration 固定文本不拼接用户
输入。

## 备选方案

- 只给 SELECT-then-INSERT 包进程内 mutex：拒绝。hook、daemon 和多个 remem 进程不共享
  mutex，不能满足跨进程不变量。
- 只使用 `BEGIN IMMEDIATE`、不建 UNIQUE index：拒绝。未来新增调用点可能绕过 wrapper，
  schema 本身仍允许非法 active 状态。
- 只建 UNIQUE index，并用 broad `INSERT OR IGNORE`：拒绝。无法可靠区分新建/合并 Dream，
  还可能吞掉非 identity constraint 错误。
- 为 CompileRules 去掉 successor 例外：拒绝。processing compiler 可能已读取旧 canonical
  memory state；把新变化合并到 processing row 会丢失后续 recompilation。
- 删除 migration duplicates：拒绝。会抹掉 processing owner、payload 与失败审计证据，
  违反 failure lifecycle 和 `B-009`。
- 重建整个 `jobs` table：拒绝。三个 partial indexes 足以表达约束；重建会扩大 foreign-key、
  rowid 和历史兼容风险。

## 风险

- Security: transition/迁移错误禁止输出 `payload_json`、完整记忆内容、token 或凭据；SQL 使用
  parameters，固定 migration SQL 不接受字符串拼接。worker project 信息按现有日志脱敏
  规则处理。数据库中的 coalesced source `last_error` 在既有 2000-char 上限内保留原始/现有
  截断错误作为主审计证据，并追加固定 marker；v069 duplicate rows 同样保留既有 error prefix
  并追加包含 duplicate/canonical ids、identity kind、manual-review flag 的固定 marker。日志
  只输出这些非 secret ids/kind/flag、failure class/安全 hash 与 marker，不输出数据库中保留的
  原始 error 或 payload。该变更涉及后台执行完整性，需 human review。
- Compatibility: 老库可能已有多个 processing owners 或不同 Dream payload。v069 保留一个
  deterministic survivor、把其余显式 permanent-fail 并保留 payload/history；late active
  Summary 仅按第 5 节的 active-only retirement 处理，不重放 v064 会改写 retryable failed rows
  的完整 predicate。旧 binary 遇到 schema v069 继续由 schema-version gate 拒绝，不允许降级
  写入。
- Performance: 三个 partial indexes 只覆盖 active rows，远小于全部 job history；
  `BEGIN IMMEDIATE` transaction 必须只包含短查询/写入，严禁把 AI/文件副作用放入其中。
  claim 的 anti-join 需由 CompileRules active index 支撑；expired/failure recovery 每批仍 bounded
  25 rows，逐行 transaction 增加固定开销以换取隔离。busy timeout 保持现有配置，锁等待超时
  明确返回错误。
- Maintenance: identity 同时存在于 migration indexes 与 Rust classifier。schema drift
  invariant、穷举 `JobType` match 和 per-type tests 共同防漂移；未来新增例外必须先更新
  Product/Tech contract。
- Queue liveness: `(project,state)` slots 若只改 UNIQUE 而不改 claim/recovery 会造成 successor
  提前运行或 bulk recovery 冻结。claim-skip、retry/expiry collision 和 unrelated-progress tests
  是发布阻断项，不能以捕获 UNIQUE error 后整批重试代替。

## 测试计划

- [ ] Unit tests: 四类 transition happy/reject paths、字段不变、error snapshot、ordinary NULL
      normalization、Dream decision/profile/cooldown、CompileRules slot/claim-skip/retry collision/
      expiry collision、Summary enqueue rejection。
- [ ] Integration tests: 三条 file-backed WAL/independent-connection barrier tests；v069 duplicate
      reconciliation、五组 Dream replay fixtures、processing Dream preservation、terminal
      preservation、existing/near-2000-char error evidence + marker、manual-review diagnostics、
      schema invariant、all-or-nothing rollback；mixed ordinary/Dream/CompileRules failure
      auto-recovery batch；worker/status/doctor shared-truth fixtures。
- [ ] Regression tests: v064 Summary retirement、failure lifecycle archive/actionable counts、
      SessionRollup Compress/Dream attribution、existing Dream and CompileRules tests。
- [ ] Deterministic gates: `python3 checks/check_workflow.py --repo . --spec-dir specs/GH818`，
      `cargo fmt --check`，`cargo check --no-default-features`，
      `cargo test --no-default-features`。
- [ ] Manual verification: 在脱敏的旧库副本先 dry-run/备份后执行升级，确认 migration 日志
      reconciliation counts、`remem status --json` jobs counts 与 `remem doctor` Pending queue
      一致；不在用户唯一数据库上制造重复 fixture。

## 发布与兼容顺序

1. spec approval 和 `ready_to_implement` human gate 完成后才可实现。
2. 先落 migration fixtures、schema invariants 与 state/enqueue tests，再接入 worker/observability。
3. 发布说明列出 schema v069、跨进程 active uniqueness、lease conflict fail-closed、Dream/
   CompileRules 兼容语义和 Summary continued retirement。
4. 首次 foreground command 原子迁移；hook-only 旧 schema 仍按现有规则要求 foreground
   migration，不静默跳过。
5. final PR review、merge、release 继续由 human gate 决定；agent 不声明最终批准。

## 回滚方案

- v069 COMMIT 前任一步失败由现有 migration runner 整体 ROLLBACK；旧 schema 和所有 rows
  保持原样，可修复后重试。
- v069 COMMIT 后不自动删除 UNIQUE indexes、不恢复 migration-reconciled duplicate work、
  不降级数据库。旧 binary 会因 newer schema fail closed，这是预期保护。
- 若新 enqueue/state 代码出现回归，先停止 worker/hook 写入并发布 forward fix；保持 v069
  indexes 继续阻止重复 active rows。需要改变 identity 时必须新增 forward migration，不能
  手工 DROP index。
- 被 migration 标为 permanent failed 的 duplicate rows 不自动复活。maintainer 先检查
  canonical job、副作用和 payload history，再通过获批的 bounded recovery 路径处理；没有
  可验证 recovery 工具时保留失败历史，不以直接 SQL 当作发布回滚步骤。
- retry/expired/failure-maintenance collision 产生的 coalesced source rows 同样作为 permanent
  failed audit history 保留：worker retry 精确记录本次 `next_attempt`，expired 与
  failure-maintenance 保持已有计数，任何路径都不伪造 exhaustion。不能在回滚中批量 requeue
  或清除诊断；若 canonical work 需要
  重建，只能经获批的 identity-aware recovery 创建一份 work，不能复活多个 source。

本 Tech Spec 不代表 `spec_approval`，也不授权 implementation、merge 或 release。
