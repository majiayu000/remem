# Tech Spec

## Linked Issue

GH-864

## Product Spec

[`product.md`](product.md)

## Codebase Context

以下锚点基于 `origin/main@5896e0be22e6b70b31316ab46ab9d0f99d0b3dfa`。

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| Transcript evidence budgeting | `src/session_rollup/transcript_evidence.rs:135-169` | 脱敏后调用 `db::truncate_str`，单消息和总预算缩短均未统一 `trim_end` | 持久化后再次脱敏/校验可能得到不同字节串 |
| Soft Git metadata probes | `src/db/core.rs:216-272` | branch/commit 各自通过同步 Git subprocess 等待，无共享 deadline | soft capture probe 可被异常 Git 仓库阻塞 |
| Commit evidence metadata | `src/git_util.rs:75,122-223`, `src/git_evidence.rs:71,158` | `resolve_toplevel` 与 `git_stdout_required` 使用无界 `Command::output()`；成功 commit 捕获会多次调用 | 这是 observed/Codex commit evidence 的真实运行路径，必须接入同一 timeout executor |
| Pending CLI schema | `src/cli/types.rs:742-759` | list/retry/quarantine 仅有 project、limit，写命令另有 dry-run | 不能表达 exact range 操作、证据查询或参数冲突 |
| Pending CLI execution | `src/cli/actions/pending.rs:211-243` | dry-run 只计数，执行只调用批量 DB API | CLI 无法验证/操作单个 ID |
| Replay range DB API | `src/db/extraction_replay.rs:59-145` | retryable 查询仅接受 project+limit，批量事务 oldest-first | 需要复用同一 predicate 增加 exact-ID 事务 |
| Topic segment parser | `src/session_rollup/parse.rs:115-137,264-268` | 只接受 ASCII lower/digit/`-`/`_`，合法版本点号直接失败 | 应复用已存在的统一 topic slug |
| Shared topic slug | `src/memory/promote/slug.rs:1-39` | `slugify_for_topic` 统一小写、标点替换、连字符折叠和长度处理 | 避免 parser 自建第二套 normalization |
| Failure lifecycle contract | `docs/specs/failure-lifecycle/{PRODUCT,TECH}.md` | 当前合同治理 `extraction_replay_ranges` 与手工 retry escape hatch | exact list/retry/quarantine 必须同步权威生命周期合同 |
| Operator documentation | `README.md:670-680,895-910` | pending 示例和 JSON 表只覆盖 legacy 命令 | exact-ID 恢复必须可发现且可审计 |
| Release surfaces | `Cargo.toml`, `Cargo.lock`, `plugins/remem/.codex-plugin/plugin.json`, `plugins/remem/runtimes/remem-releases.json`, `npm/remem/package.json`, `server.json` | patch 发布要求所有版本面同步 | 防止可执行文件、插件和 registry manifest 漂移 |

## 设计方案

### 1. 统一 transcript 截断终结步骤

在 `src/session_rollup/transcript_evidence.rs` 增加私有 helper，输入已脱敏文本和字节上限，先调用
`db::truncate_str` 保证 UTF-8 边界，再调用 `trim_end`。单消息初始限制和总预算缩短都只能调用
该 helper。预算器以 helper 返回值的真实字节数更新 `total_bytes`；空结果沿用当前丢弃逻辑。

不修改 `PromptTranscriptEvidence::validate_for_range` 的角色、range、redaction、count/byte 或
citation invariants。回归测试构造恰好在单消息上限处保留尾部空白的输入，断言生成结果再次经过
redactor 后字节相同且可通过 range validation。

### 2. 一个覆盖真实 metadata 路径的有界 Git executor

在 `src/git_util.rs` 提取 crate-visible `command_output_with_timeout(Command, Duration)`，由
`git_stdout_required`、`resolve_toplevel` 以及 `src/db/core.rs` 的 soft branch/commit probe 共用：

1. 在 Unix release targets 上复用仓库现有 `CommandExt::process_group(0)` 模式，把每个 Git probe 放入
   独立进程组；`spawn` 后记录 `Instant` deadline，并立即 take piped stdout/stderr；
2. 为 stdout/stderr 启动只读 drain worker，在 child 运行期间持续 `read_to_end`，避免任一 OS pipe 填满；
3. 主线程通过 `try_wait` 和短间隔轮询等待，不得在 child 退出前等待 drain worker 完成；
4. drain workers 通过 channel 报告 completion/bytes/error；direct child 正常退出后仍只等待到同一
   reader deadline，completion 已到达才 join 并构造 `Output`；若后代持 pipe 导致 completion 未到，
   必须进入第 5/7 步的整组 cleanup，不能因 direct child 已退出而无界等待；
5. deadline 到达后先向整个进程组发送 TERM，经过有界 grace 后仍存活则发送 KILL，再 reap direct child；
6. spawn 前错误可直接返回；spawn 成功后的 `try_wait`、kill、reap、pipe read 或 worker join 错误必须
   进入统一 lifecycle error；
7. cleanup 至少尝试有界终止进程组并 wait/reap direct child；reader completion 只能等待到 cleanup
   deadline，超时必须作为 lifecycle/cleanup error 返回且不得无界 join；cleanup 自身失败附加到原
   error，任何分支不得进入无界等待。

所有命令继续使用 `Command::new("git")`、`current_dir(cwd)` 和参数数组，不经过 shell。固定
`GIT_PROBE_TIMEOUT = 2s`（移动到共享 helper 所在模块，避免两套时限）。调用语义保持分层：

- `db::detect_git_branch` / `detect_git_commit` 和 `resolve_toplevel` 的 soft/optional 路径在 timeout 或
  lifecycle error 时写 error 日志并返回 `None`；
- 新增 `resolve_toplevel_required`（或等价的 `Result<PathBuf>` 路径）供 `resolve_commit_metadata` 使用；
  它不得经由返回 `Option` 的 soft helper 丢失失败原因，并必须保留 argv 类别和 cwd 上下文；
- `git_stdout_required` 在 timeout、lifecycle error 或非零退出时返回带 argv 类别和 cwd 的 contextual
  error；`git_evidence.rs` 现有 observed-event propagate 与 Codex-transcript log-and-skip 语义不变；
- 正常非零 soft probe 继续表示信息不可用，不伪造成 branch/commit。

超时测试使用仓库测试进程自身的 ignored helper 作为长运行 child，避免依赖平台 `sleep`；断言
timeout child 已被回收。另加可控 poll-error fixture，证明 spawn 后的 `try_wait` error 会调用统一
cleanup，而不是通过 `?` 直接返回。大输出 fixture 必须让 stdout 和 stderr 均超过常见 OS pipe buffer，
证明 drain 与 child 执行并发且不会误报 timeout。真实路径测试必须证明
`resolve_commit_metadata`/`git_stdout_required` 调用共享 executor，并证明 required toplevel timeout
保留 argv/cwd 上下文。另一个递归 test-helper fixture 必须生成同进程组后代并让它保持 pipe 打开，
证明 timeout 会终止整组且 reader completion 不超过 cleanup deadline。该 OS subprocess 路径在合并前
必须人工安全审核。

### 3. exact range CLI 与事务

为 list/retry/quarantine 三个 Clap variant 增加 `id: Option<i64>`。参数合同必须区分命令行显式值和 Clap 默认值：
`--id` 与用户显式提供的 `--project`/`--limit` 冲突，但 `--id`-only 命令即使 limit 字段取得默认值
也必须成功解析；实现可使用 Clap 的 command-line value-source predicate，或把默认 limit 移入 batch
分支。DB 层把当前
retryable ID 查询扩展为可选 `range_id` filter，并新增：

- `ensure_extraction_replay_range_retryable(conn, id)`
- `retry_extraction_replay_range(conn, id)`
- `quarantine_extraction_replay_range(conn, id)`

`ensure` 拒绝非正 ID，并要求查询结果精确等于目标 ID。exact retry/quarantine 各自在一个 SQLite
事务中重新运行 ensure 后变更；retry 复用现有 `enqueue_replay_extraction_task` idempotency，
quarantine 复用 `clear_terminal_failures_for_quiesced_range`。任何失败回滚整个目标事务，不触碰
sibling ranges。

exact list 使用只读连接和独立 ID 查询，不复用当前只列 active statuses 的 batch query；它必须能返回
`pending|failed|requeued|quarantined|replayed` 目标 range，并 LEFT JOIN 其 `replay_task_id` 对应任务，
输出 range status/attempt/error 以及 replay task id/status/attempt/error。不存在的 ID 返回错误，不能
回退到最近 N 条。JSON 只增加这些声明字段，不输出 captured payload 或未脱敏 provider secret。

CLI dry-run 使用 `open_db_read_only` 加 ensure，只输出目标可操作；实际路径使用 `open_db` 和
exact API。无 ID 分支继续调用现有 count/batch API，输出保持兼容。focused DB fixture 创建两个
独立 ranges，先 exact retry 一个，再 exact quarantine 另一个，每一步断言 sibling 状态不变。
README 增加 exact list/retry/quarantine 示例；failure-lifecycle PRODUCT/TECH 增加精确手工恢复、terminal
状态证据和不触碰 sibling ranges 的合同。

### 4. topic key 规范化

`parse_segment` 保留“属性存在、trim 后非空”的第一道验证。若 raw key 已符合旧 parser grammar
`[a-z0-9_-]+` 且至少包含一个 ASCII 字母或数字，直接原样保留以维持既有 topic identity；否则才传给
`crate::memory::slugify_for_topic(&raw_topic_key, 96)`。规范化输出为空时返回包含 raw key 的明确
错误。旧 grammar predicate 仅作为兼容快路径，不承担新输入的第二套 normalization。

测试至少覆盖版本点号、纯标点、既有 kebab-case 和既有 snake_case key；`---`、`___` 等只有旧
grammar 标点的输入不得命中兼容快路径。此变更只影响旧 grammar 原本会拒绝的新解析输出，不迁移
已持久化 topic，也不修改全局 slug helper。

### 5. Release housekeeping

实现作为一个 patch release，同步六个 manifest 加 `Cargo.lock`，更新 `CHANGELOG.md`。版本同步
脚本和 version bump gate 必须通过。range 308 的真实 exact retry 作为 issue 运维证据单独记录，
不写入自动化测试，也不使用生产 payload 作为 fixture。

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| B-001 | `transcript_evidence.rs` unified truncate helper | `cargo test per_message_budget_keeps_redaction_idempotent_at_whitespace_boundary --locked` |
| B-002 | `EvidenceBudget::push` total budget loop | focused UTF-8/empty/total-budget tests plus existing `total_budget_never_retains_empty_utf8_message` |
| B-003 | `PromptTranscriptEvidence::validate_for_range` unchanged gates | existing transcript evidence validation tests + `cargo test session_rollup --locked` |
| B-004 | shared `git_util::command_output_with_timeout`, soft probes, required toplevel, `git_stdout_required` | `cargo test git_metadata_commands_use_bounded_executor --locked`, `cargo test command_output_with_timeout_kills_process_group --locked`, `cargo test command_output_with_timeout_drains_large_output --locked`, `cargo test required_toplevel_preserves_timeout_context --locked`, and log review |
| B-005 | process-group post-spawn lifecycle and bounded drain-worker cleanup | `cargo test command_output_with_timeout_cleans_up_after_poll_error --locked`, `cargo test command_output_with_timeout_bounds_reader_completion --locked`; Clippy; manual error aggregation review |
| B-006 | shared `Command::new("git")` argv construction | source review proves no shell across `git_util.rs` and `db/core.rs`; mandatory human security review |
| B-007 | Clap `PendingAction` variants + exact range evidence query | `cargo test pending_exact_range_id_conflicts_with_batch_filters --locked`, `cargo test pending_exact_range_id_accepts_implicit_default_limit --locked`, `cargo test exact_range_list_includes_replayed_task_evidence --locked`, and non-positive DB test |
| B-008 | read-only CLI dry-run + shared ensure predicate | CLI/DB focused tests for missing, archived, active-task and non-retryable targets |
| B-009 | exact retry/quarantine transaction APIs | `cargo test exact_replay_range_operations_do_not_mutate_sibling_ranges --locked` |
| B-010 | exact range filters and rollback | sibling fixture plus concurrent/not-retryable negative fixture |
| B-011 | existing batch APIs and CLI branches | existing pending maintenance tests + no-ID parser/output snapshots |
| B-012 | `parse_segment` + shared slug helper | `cargo test normalizes_version_punctuation_in_topic_key --locked` |
| B-013 | empty-normalized error branch | `cargo test rejects_topic_key_that_normalizes_to_empty --locked` |
| B-014 | old-grammar compatibility fast path + shared slug determinism | `cargo test preserves_existing_snake_case_topic_key --locked` plus kebab-case fixture and existing slug tests |
| B-015 | release/docs surfaces and operational handoff | version-sync/version-bump scripts; README/failure-lifecycle review; GH-864 comment with authenticated range 308 exact range/task result and redacted provider/profile log evidence |

## 数据流

```text
bounded transcript row
  -> redact
  -> UTF-8 byte truncate
  -> trim trailing whitespace
  -> count actual bytes
  -> persist
  -> same validation result on retry

cwd
  -> git argv (no shell)
  -> spawn / 2s deadline
  -> success output
     | soft nonzero => None
     | required nonzero => contextual error
     | timeout/poll error => bounded cleanup => None or contextual error

pending CLI
  -> --id? -------------------------- no --> existing project/limit batch path
       |
       yes
       v
  read-only exact list/ensure OR write transaction
  -> exact identity / retryable predicate
  -> show one / retry one / quarantine one
  -> commit or rollback; sibling ranges untouched

LLM topic_key
  -> required + trim
  -> matches old [a-z0-9_-]+ and has ASCII alphanumeric? -- yes --> preserve verbatim
       |
       no
       v
     shared slugify_for_topic(..., 96)
  -> non-empty normalized key OR parse error
```

## 备选方案

- **四个 PR 分别发布**：拒绝。修复相互正交但都很小，仓库 patch 版本同步要求会让四个 PR 产生重复
  bump/rebase；一个 PR 保留四个原子 commits 更易审计。
- **只增加 Git timeout，不 kill/reap**：拒绝。返回调用方但遗留 child 会把阻塞变成进程泄漏。
- **只给 `db::detect_git_*` 增加 timeout**：拒绝。真实 commit capture 调用
  `git_util::resolve_commit_metadata`，只修无调用者的 soft commit probe 不能消除阻塞。
- **exact CLI 在 list 结果中按位置选取**：拒绝。列表顺序和并发状态会变化，不能证明目标身份。
- **dry-run 只检查 ID 存在**：拒绝。会把 archived、active-task 或非法状态误报为可执行。
- **topic parser 单独允许点号**：拒绝。继续产生第三套 slug 规则，无法覆盖其它语义标点。
- **截断后只在 validator trim**：拒绝。持久化表示仍不稳定，且预算会按错误字节数计算。

## 风险

- **Security**：Git subprocess 涉及 OS 命令执行。必须保持 argv 调用、固定 executable/arguments、
  独立 Unix process group、有界 TERM/KILL、reader completion 和可靠 reap；合并前执行人工安全审查。
  cwd 只作为单个参数，不进入 shell。
- **Data integrity**：错误的 exact predicate 或事务边界可能操作 sibling range。共享 predicate、
  range ID SQL filter、单事务和双-range fixture共同约束。
- **Compatibility**：无 schema migration；批量 CLI 保持。新增 `--id` 与 batch filters 冲突是有意
  fail-fast。topic normalization 会接受此前拒绝的语义 key，但不重写存量数据。
- **Performance**：每个 Git 子进程最坏等待 2 秒；一次 metadata capture 包含多条命令，因此总上限是
  有限的命令数乘以 2 秒，而不是整次 capture 仅 2 秒。正常仓库路径只增加轻量轮询。
- **Reliability**：process-group kill/reap、后代持 pipe、SQLite 竞争和 provider 认证均可能失败；失败
  必须可见且不能无界等待或改选其它 range。
- **Maintenance**：四项修复共用一个 release bump，但保持四个原子 commits，便于回溯和 cherry-pick。

## 测试计划

- [ ] `cargo test per_message_budget_keeps_redaction_idempotent_at_whitespace_boundary --locked`
- [ ] `cargo test exact_replay_range_operations_do_not_mutate_sibling_ranges --locked`
- [ ] `cargo test command_output_with_timeout_kills_process_group --locked`
- [ ] `cargo test command_output_with_timeout_cleans_up_after_poll_error --locked`
- [ ] `cargo test command_output_with_timeout_drains_large_output --locked`
- [ ] `cargo test command_output_with_timeout_bounds_reader_completion --locked`
- [ ] `cargo test required_toplevel_preserves_timeout_context --locked`
- [ ] `cargo test git_metadata_commands_use_bounded_executor --locked`
- [ ] `cargo test normalizes_version_punctuation_in_topic_key --locked`
- [ ] `cargo test rejects_topic_key_that_normalizes_to_empty --locked`
- [ ] `cargo test preserves_existing_snake_case_topic_key --locked`
- [ ] `cargo test rejects_punctuation_only_topic_key --locked`
- [ ] `cargo test pending_exact_range_id_accepts_implicit_default_limit --locked`
- [ ] `cargo test exact_range_list_includes_replayed_task_evidence --locked`
- [ ] 现有 transcript、pending CLI/batch、slug 与 session rollup focused suites
- [ ] `cargo fmt --check`
- [ ] `cargo check --locked`
- [ ] `cargo test --locked --quiet`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `node --test plugins/remem/scripts/remem-runtime.test.js plugins/remem/apps/remem/request-security.test.js plugins/remem/apps/remem/server.test.js npm/remem/scripts/install.test.js`
- [ ] `python3 scripts/ci/check_plugin_version_sync.py`
- [ ] `python3 scripts/ci/check_version_bump.py <base-sha> HEAD`
- [ ] `python3 scripts/ci/check_pr_preflight.py --base <base-sha> --head HEAD --pr-body-file <body-file>`
- [ ] `git diff --check`
- [ ] PR preflight 与 Git subprocess/exact DB transaction 人工 review
- [ ] Claude profile 可用后执行 `remem pending retry-extraction-ranges --id 308 --dry-run`，再执行非 dry-run，
      等待 worker 终态后运行 `remem pending list-extraction-ranges --id 308 --json`，并记录 exact range/task
      状态及对应 replay task 的已脱敏 provider/profile 日志

## 回滚方案

四个实现 commits 保持原子性。若某一 slice 回归，可在后续 patch 中单独 revert 对应 commit：

- transcript revert 会恢复旧持久化行为，不迁移既有 evidence；
- Git probe revert 会恢复无界风险，因此只应在修复 timeout executor 后回滚；
- exact-ID revert 不改变数据库，只移除新 CLI/API；
- topic normalization revert 只影响之后的新 rollup parse。

release manifest bump 不应单独回退到已发布版本号。任何 rollback 都重新运行完整 version sync 和
受影响 focused suites；不得通过关闭 error 日志或放宽 retryable predicate 达成。

<!-- specrail-planned-changes
{
  "version": 1,
  "issue": 864,
  "complete": true,
  "paths": [
    "specs/GH864/product.md",
    "specs/GH864/tech.md",
    "src/session_rollup/transcript_evidence.rs",
    "src/session_rollup/parse.rs",
    "src/cli/types.rs",
    "src/cli/actions/pending.rs",
    "src/cli/tests_maintenance.rs",
    "src/db/extraction_replay.rs",
    "src/db/extraction/retry_regression_tests.rs",
    "src/db/core.rs",
    "src/git_util.rs",
    "README.md",
    "docs/specs/failure-lifecycle/PRODUCT.md",
    "docs/specs/failure-lifecycle/TECH.md",
    "CHANGELOG.md",
    "Cargo.toml",
    "Cargo.lock",
    "plugins/remem/.codex-plugin/plugin.json",
    "plugins/remem/runtimes/remem-releases.json",
    "npm/remem/package.json",
    "server.json"
  ],
  "spec_refs": [
    "specs/GH864/product.md",
    "specs/GH864/tech.md"
  ]
}
-->

本文件不构成 `spec_approval`。只有维护者审阅实际 product/tech diff、批准 Git 子进程与 exact-range
事务边界，并把 GH-864 置为 `ready_to_implement` 后，才能创建 `tasks.md` 或发布实现 PR。
