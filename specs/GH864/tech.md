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
| Git metadata probes | `src/db/core.rs:216-272` | branch/commit 各自通过 `Command::output()` 同步等待，无 deadline | capture/rollup 可被异常 Git 仓库无限阻塞 |
| Pending CLI schema | `src/cli/types.rs:742-759` | retry/quarantine 仅有 project、limit、dry-run | 不能表达 exact range 操作或参数冲突 |
| Pending CLI execution | `src/cli/actions/pending.rs:211-243` | dry-run 只计数，执行只调用批量 DB API | CLI 无法验证/操作单个 ID |
| Replay range DB API | `src/db/extraction_replay.rs:59-145` | retryable 查询仅接受 project+limit，批量事务 oldest-first | 需要复用同一 predicate 增加 exact-ID 事务 |
| Topic segment parser | `src/session_rollup/parse.rs:115-137,264-268` | 只接受 ASCII lower/digit/`-`/`_`，合法版本点号直接失败 | 应复用已存在的统一 topic slug |
| Shared topic slug | `src/memory/promote/slug.rs:1-39` | `slugify_for_topic` 统一小写、标点替换、连字符折叠和长度处理 | 避免 parser 自建第二套 normalization |
| Release surfaces | `Cargo.toml`, `Cargo.lock`, `plugins/remem/.codex-plugin/plugin.json`, `plugins/remem/runtimes/remem-releases.json`, `npm/remem/package.json`, `server.json` | patch 发布要求所有版本面同步 | 防止可执行文件、插件和 registry manifest 漂移 |

## 设计方案

### 1. 统一 transcript 截断终结步骤

在 `src/session_rollup/transcript_evidence.rs` 增加私有 helper，输入已脱敏文本和字节上限，先调用
`db::truncate_str` 保证 UTF-8 边界，再调用 `trim_end`。单消息初始限制和总预算缩短都只能调用
该 helper。预算器以 helper 返回值的真实字节数更新 `total_bytes`；空结果沿用当前丢弃逻辑。

不修改 `PromptTranscriptEvidence::validate_for_range` 的角色、range、redaction、count/byte 或
citation invariants。回归测试构造恰好在单消息上限处保留尾部空白的输入，断言生成结果再次经过
redactor 后字节相同且可通过 range validation。

### 2. 一个有界 Git probe 执行器

在 `src/db/core.rs` 提取私有 `command_output_with_timeout(Command, Duration)`：

1. `spawn` 后记录 `Instant` deadline；
2. 通过 `try_wait` 和短间隔轮询等待；
3. 正常退出后调用 `wait_with_output` 收集输出；
4. deadline 到达后依次 `kill`、`wait`，返回明确 timeout；
5. spawn/try_wait/kill/reap 任一步失败都返回 error。

`detect_git_branch` 和 `detect_git_commit` 共用 `git_probe_output`，固定
`GIT_PROBE_TIMEOUT = 2s`。命令使用 `Command::new("git")` 与参数数组
`["-C", cwd, "rev-parse", ...]`，stdout pipe、stderr null，不经过 shell。timeout 与生命周期错误
写 error 日志后返回 `None`；正常非零退出也返回 `None`，保持现有 API。

超时测试使用仓库测试进程自身的 ignored helper 作为长运行 child，避免依赖平台上的 `sleep`
命令；断言返回 timeout 且 child 已被回收。该 OS subprocess 路径在合并前必须人工安全审核。

### 3. exact range CLI 与事务

为两个 Clap variant 增加 `id: Option<i64>`。参数合同必须区分命令行显式值和 Clap 默认值：
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

CLI dry-run 使用 `open_db_read_only` 加 ensure，只输出目标可操作；实际路径使用 `open_db` 和
exact API。无 ID 分支继续调用现有 count/batch API，输出保持兼容。focused DB fixture 创建两个
独立 ranges，先 exact retry 一个，再 exact quarantine 另一个，每一步断言 sibling 状态不变。

### 4. topic key 规范化

`parse_segment` 保留“属性存在、trim 后非空”的第一道验证。若 raw key 已符合旧 parser grammar
`[a-z0-9_-]+`，直接原样保留以维持既有 topic identity；否则才传给
`crate::memory::slugify_for_topic(&raw_topic_key, 96)`。规范化输出为空时返回包含 raw key 的明确
错误。旧 grammar predicate 仅作为兼容快路径，不承担新输入的第二套 normalization。

测试至少覆盖版本点号、纯标点、既有 kebab-case 和既有 snake_case key。此变更只影响旧 grammar
原本会拒绝的新解析输出，不迁移已持久化 topic，也不修改全局 slug helper。

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
| B-004 | `command_output_with_timeout`, `git_probe_output` | `cargo test command_output_with_timeout_kills_long_running_child --locked` and log review |
| B-005 | Git probe lifecycle error branches | unit failure injection where practical; `cargo clippy --all-targets -- -D warnings`; manual error-log review |
| B-006 | `Command::new` argument construction | source review proves no shell; mandatory human security review |
| B-007 | Clap `PendingAction` variants | `cargo test pending_exact_range_id_conflicts_with_batch_filters --locked`, `cargo test pending_exact_range_id_accepts_implicit_default_limit --locked`, and non-positive DB test |
| B-008 | read-only CLI dry-run + shared ensure predicate | CLI/DB focused tests for missing, archived, active-task and non-retryable targets |
| B-009 | exact retry/quarantine transaction APIs | `cargo test exact_replay_range_operations_do_not_mutate_sibling_ranges --locked` |
| B-010 | exact range filters and rollback | sibling fixture plus concurrent/not-retryable negative fixture |
| B-011 | existing batch APIs and CLI branches | existing pending maintenance tests + no-ID parser/output snapshots |
| B-012 | `parse_segment` + shared slug helper | `cargo test normalizes_version_punctuation_in_topic_key --locked` |
| B-013 | empty-normalized error branch | `cargo test rejects_topic_key_that_normalizes_to_empty --locked` |
| B-014 | old-grammar compatibility fast path + shared slug determinism | `cargo test preserves_existing_snake_case_topic_key --locked` plus kebab-case fixture and existing slug tests |
| B-015 | release manifests and operational handoff | version-sync/version-bump scripts; GH-864 comment with authenticated range 308 replay result |

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
  -> success output | nonzero None | timeout kill+reap+error

pending CLI
  -> --id? -------------------------- no --> existing project/limit batch path
       |
       yes
       v
  read-only ensure (dry-run) OR write transaction
  -> exact retryable predicate
  -> retry one / quarantine one
  -> commit or rollback; sibling ranges untouched

LLM topic_key
  -> required + trim
  -> shared slugify_for_topic(..., 96)
  -> non-empty normalized key OR parse error
```

## 备选方案

- **四个 PR 分别发布**：拒绝。修复相互正交但都很小，仓库 patch 版本同步要求会让四个 PR 产生重复
  bump/rebase；一个 PR 保留四个原子 commits 更易审计。
- **只增加 Git timeout，不 kill/reap**：拒绝。返回调用方但遗留 child 会把阻塞变成进程泄漏。
- **exact CLI 在 list 结果中按位置选取**：拒绝。列表顺序和并发状态会变化，不能证明目标身份。
- **dry-run 只检查 ID 存在**：拒绝。会把 archived、active-task 或非法状态误报为可执行。
- **topic parser 单独允许点号**：拒绝。继续产生第三套 slug 规则，无法覆盖其它语义标点。
- **截断后只在 validator trim**：拒绝。持久化表示仍不稳定，且预算会按错误字节数计算。

## 风险

- **Security**：Git subprocess 涉及 OS 命令执行。必须保持 argv 调用、固定 executable/arguments、
  有界等待和可靠 reap；合并前执行人工安全审查。cwd 只作为单个参数，不进入 shell。
- **Data integrity**：错误的 exact predicate 或事务边界可能操作 sibling range。共享 predicate、
  range ID SQL filter、单事务和双-range fixture共同约束。
- **Compatibility**：无 schema migration；批量 CLI 保持。新增 `--id` 与 batch filters 冲突是有意
  fail-fast。topic normalization 会接受此前拒绝的语义 key，但不重写存量数据。
- **Performance**：Git probe 最坏增加 2 秒，但消除无界等待；正常仓库路径只增加轻量轮询。
- **Reliability**：kill/reap 平台差异、SQLite 竞争和 provider 认证均可能失败；失败必须可见且不能
  改选其它 range。
- **Maintenance**：四项修复共用一个 release bump，但保持四个原子 commits，便于回溯和 cherry-pick。

## 测试计划

- [ ] `cargo test per_message_budget_keeps_redaction_idempotent_at_whitespace_boundary --locked`
- [ ] `cargo test exact_replay_range_operations_do_not_mutate_sibling_ranges --locked`
- [ ] `cargo test command_output_with_timeout_kills_long_running_child --locked`
- [ ] `cargo test normalizes_version_punctuation_in_topic_key --locked`
- [ ] `cargo test rejects_topic_key_that_normalizes_to_empty --locked`
- [ ] `cargo test preserves_existing_snake_case_topic_key --locked`
- [ ] `cargo test pending_exact_range_id_accepts_implicit_default_limit --locked`
- [ ] 现有 transcript、pending CLI/batch、slug 与 session rollup focused suites
- [ ] `cargo fmt --check`
- [ ] `cargo check --locked`
- [ ] `cargo test --locked --quiet`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `node --test plugins/remem/scripts/remem-runtime.test.js plugins/remem/apps/remem/request-security.test.js plugins/remem/apps/remem/server.test.js npm/remem/scripts/install.test.js`
- [ ] `python3 scripts/ci/check_plugin_version_sync.py`
- [ ] `python3 scripts/ci/check_version_bump.py <base-sha> HEAD`
- [ ] `git diff --check`
- [ ] PR preflight 与 Git subprocess/exact DB transaction 人工 review
- [ ] Claude profile 可用后执行 `remem pending retry-extraction-ranges --id 308 --dry-run`，
      再执行非 dry-run 并记录最终 range/task 状态

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
