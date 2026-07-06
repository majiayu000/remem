# Changelog

## Unreleased

### Added
- Staged source version `0.5.170` for GH-672 memory poisoning defense:
  source trust metadata, deterministic instruction-pattern quarantine, and
  direct-save trust tagging. The staged line also adds explicit quarantine
  acknowledgement review, render-time poisoned-memory drops, and doctor
  reporting for quarantine/drop state.
- Staged source version `0.5.169` for the GH-671 preference rule
  compilation foundation: disabled-by-default config defaults, canonical
  preference reinforcement state, rule override state, diagnostic state, and
  schema/convergence guardrails without enabling runtime rule behavior.
- Staged source version `0.5.168` for GH-678 project memory pack export:
  deterministic `pack.json`/`memories.jsonl`/`INDEX.md` generation for active
  repo-owned startup memories, fail-loud redaction gating, and focused export
  fixtures.
- Staged source version `0.5.167` for GH-680 procedure export Phase 1:
  `remem procedures list` exposes promoted procedure memories with maturity
  metadata before any review-gated export writer is introduced.
- Staged source version `0.5.166` for GH-684 observation wording: MCP and
  architecture docs now classify `source='observation'` as a current extracted
  observation source instead of a legacy source.
- Staged source version `0.5.164` for GH-673 context stability: total context
  budget enforcement now truncates at stable item boundaries while preserving
  the truncation marker and stats footer.
- Staged source version `0.5.163` for GH-726 local PR preflight: aggregate
  CI gate checks in one command, document it as the PR preflight, and stabilize
  the log lock-open regression test surfaced by the full preflight run.
- Staged source version `0.5.162` for GH-683 review queue throughput:
  review queue health metrics, doctor deadlock findings, batch review
  operations, durable review metadata, and REST blocked-candidate reporting.
- Staged source version `0.5.160` for GH-717 downstream active semantic
  adoption: observation vector dedup, active-model preference embedding
  fallback thresholds, and focused dedup/preference regressions.
- Staged source version `0.5.159` for the GH-716 provider-comparison follow-up:
  optional local/API embedding profile probe failures are recorded as
  unavailable rows instead of aborting the whole report.
- Staged source version `0.5.158` for the GH-716 embedding provider comparison
  eval: EN/CJK paraphrase fixtures, feature-hash/local/API report rows, explicit
  default-flip criteria, and the recorded no-flip decision.
- Staged source version `0.5.157` for the GH-715 local semantic embedding
  runtime slice: fastembed-backed local model download/status, explicit
  active-profile backfill/prune, hook-safe missing-model deferral, and
  verified model manifests.
- Staged source version `0.5.156` for the GH-715 multi-model memory embedding
  storage key and active-profile backfill slice.
- Staged source version `0.5.155` for the merged embedding provider contract
  and failure lifecycle maintenance line.
- Staged source version `0.5.154` for the failure lifecycle maintenance
  feature: classify transient vs permanent failures, auto-requeue bounded
  transient extraction/replay/job failures, archive aged permanent/exhausted
  failures into history with an explicit `cleanup --archived-failures` purge
  path, and expose actionable-vs-archived failure stats in `status`/`doctor`.
- Staged source version `0.5.153`: batch session ingestion (`remem
  ingest-sessions` with per-file cursors and multi-root discovery) and raw
  time-window / session-listing queries (GH720 Phase 1, #722 #723).

### Fixed
- Staged source version `0.5.161` for the GH-717 post-merge semantic dedup
  follow-up: preserve numeric observation value changes, keep observation facts
  with narratives, and propagate preference API failures when fallback is off.
- Mapped memory-candidate extraction outputs that copy observation types
  (`feature`, `refactor`, `change`) back into the canonical candidate memory
  vocabulary instead of failing the whole extraction batch.
- Staged source version `0.5.125` without pointing plugin runtime downloads at
  an unpublished GitHub Release. The committed runtime manifest now stays local
  until the release workflow uploads checked assets.
- Hardened macOS ARM installer handling so ad-hoc codesigning failures are not
  silently ignored.

### Changed
- Added repository public-surface and file-size guardrails for release
  readiness.
- Added the `Auto Release` workflow so a passing `main` CI run tags staged
  source versions and lets the existing release workflow publish the assets.
- Staged source version `0.5.126` for the current-memory contract gates.
- Staged source version `0.5.127` for coding-bench contract artifacts.
- Staged source version `0.5.128` for workstream identity continuity.
- Staged source version `0.5.129` for usage feedback shadow ranking.
- Staged source version `0.5.130` for preference semantic-dedup calibration.
- Staged source version `0.5.131` for the coding-agent benchmark runner.
- Staged source version `0.5.132` for randomized coding-benchmark run order.
- Staged source version `0.5.133` for the public benchmark artifact verifier.
- Staged source version `0.5.134` for the remem-native memory benchmark suite.
- Staged source version `0.5.135` for the adversarial memory policy benchmark
  suite.
- Staged source version `0.5.136` for memory benchmark write-vs-retrieval
  diagnostics and baseline adapters.
- Staged source version `0.5.137` for the issue385-v1 coding benchmark task
  pack and `bench coding` dry-run alias.
- Staged source version `0.5.138` for coding-benchmark memory attribution and
  fixed failure taxonomy.
- Staged source version `0.5.139` for the directional public benchmark baseline
  report generator and checked-in baseline report.
- Staged source version `0.5.140` for preference semantic-dedup follow-ups:
  extraction source reduction, render-time cleanup, and merge cleanup clustering.
- Staged source version `0.5.141` for automatic release dispatch after
  bot-created release tags.
- Staged source version `0.5.142` for memory-candidate observation-type
  normalization.
- Staged source version `0.5.143` for review-gated temporal fact diagnostics.
- Staged source version `0.5.144` for summary promotion shadow-gate telemetry.
- Staged source version `0.5.145` for deterministic capacity eval scale curves.
- Staged source version `0.5.146` for associative multi-hop fixture headroom.
- Staged source version `0.5.147` for summary promotion enforce mode.
- Staged source version `0.5.148` for cross-process log rotation hardening.
- Staged source version `0.5.149` for foreground status schema convergence.
- Staged source version `0.5.150` for capacity degradation eval gates.
- Staged source version `0.5.151` for prefix-cache-stable context rendering.
- Staged source version `0.5.152` for Codex SessionStart context visibility.
- Staged source version `0.5.153` for the local embedding provider contract.
- Updated extraction-eval candidate prompt fingerprints for the
  memory-candidate type-vocabulary prompt change.

## [0.5.109] - 2026-06-20

### Added
- Documented the full native web API surface for remem-web, including
  capabilities, canonical memory browse/detail, stats, graph, candidate list,
  and candidate review endpoints.
- Added a local native API smoke test for the `remem api` read-model endpoints
  under bearer-token auth. This is the release-note entry for the planned
  `remem 0.5.109` web API surface; installed-binary docs should point users at
  it only after the `v0.5.109` tag and GitHub Release exist.
- After `v0.5.109` is published, remem-web should require `remem >= 0.5.109` for
  `/api/v1/capabilities.features.stats`, `memory_list`, `memory_detail`,
  `candidate_rows`, `candidate_review`, and `graph`; older clients can keep
  using `/api/v1/memory?id=` and `/api/v1/memories/list` compatibility paths.

## [0.5.104] - 2026-06-20

### Added
- Added current-state queries over `memory_state_keys` for CLI and MCP callers,
  including compact history, conflict, edge-evidence, and as-of-time output.
- Added human-editable markdown memory export and reindex import, including
  archived state, temporal facts, and current-state edge metadata.
- Added deterministic failure-trajectory lesson feeding from raw transcripts:
  repeated failed-fix evidence plus an explicit lesson now records an
  idempotent `failure` lesson outcome before summary short-circuits.

### Fixed
- Fixed current-state as-of history so mutable historical memory rows updated
  after the requested cutoff are not shown as if they were known then.
- Fixed graph-candidate review follow-ups so graph extraction only prompts for
  promotable edge types, deferred graph candidates stay visible in status, and
  graph tasks do not wait on memory tasks that already covered their range.
- Fixed markdown reindex restores so stale source hashes, cross-store
  provenance ids, older current-state slots, cross-memory fact supersession,
  and memory-edge remapping do not corrupt restored memory state.

### Changed
- Changed the npm wrapper package scope to `@remem-ai/remem` for the branded
  remem npm distribution.
- Added phase-0 extraction cursor integrity checks, model-provided confidence
  handling, and promotion metrics for extraction review workflows.
- Reframed project metadata and README docs around Claude Code and Codex as
  first-class hosts, including a Codex setup section and distribution channel
  guidance.
- Added Homebrew install docs and prepared an npm wrapper package for future
  npm publishing.

## [0.4.5] - 2026-05-26

### Fixed
- Updated the remaining GitHub Release action to a Node.js 24-compatible
  version.
- Updated Codex hook feature flag installation to use `[features].hooks` and
  remove the deprecated `[features].codex_hooks` alias.

## [0.4.4] - 2026-05-26

### Added
- Added release-binary installation docs for pinned versions, custom install
  directories, manual GitHub Release downloads, and binary-only installs.
- Added release asset checksums to future GitHub Releases.

### Fixed
- Updated GitHub release workflow artifact actions to Node.js 24-compatible
  versions.
- Fixed `remem install` binary path resolution so hooks and MCP use the current
  binary path or `REMEM_INSTALL_BINARY`, instead of always writing
  `~/.local/bin/remem`.

## [0.4.3] - 2026-05-26

### Added
- Added Codex context injection gating for SessionStart hooks: first injection
  emits full context, duplicate same-session context suppresses empty stdout,
  and changed context emits compact delta output.

### Fixed
- Fixed context gate fallback behavior so missing trusted session identity fails
  open, fallback cwd keys are canonicalized, and expired transcript-only fallback
  cooldowns re-emit full context instead of compact delta.
- Fixed context hash normalization for generated debug traces and stats footer
  totals so unchanged context is not repeatedly injected.
- Fixed migration dry-run validation to run post-migration hooks against a
  faithful on-disk backup clone while preserving owner-only temp permissions.
- Fixed backup import handling for malformed `topic_key` values and improved
  empty CLI search diagnostics.

## [0.4.2] - 2026-05-16

### Fixed
- Fixed Codex usage accounting to parse the current `codex exec --json`
  `turn.completed.usage` event instead of trying to match a run marker in
  ephemeral session logs. New Codex-backed rows now record `usage_source =
  codex_log` with cache and reasoning token breakdowns.

### Docs
- Updated usage accounting docs to describe the `codex exec --json` source for
  exact Codex token counts.

## [0.4.1] - 2026-05-16

### Packaging
- Bumped the crate and binary version to `0.4.1` for the post-`0.4.0`
  maintenance release.

## [0.4.0] - 2026-05-16

### Added
- Added `remem usage` for daily and weekly AI token/cost reporting.
- Added `ai_usage_events` token breakdown fields for input, output, reasoning,
  cache creation, cache read, raw input/output, usage source, and pricing source.
- Added Codex session JSONL token accounting keyed by a per-run remem id.
- Added historical usage repricing migration for older zero-cost rows.
- Added CLI search parity with the canonical memory service, including
  `--offset`, `--branch`, `--include-stale`, `--multi-hop`, and `--type` as a
  `--memory-type` alias.
- Added raw archive fallback previews and `has_more` guidance to CLI search.
- Added `--dry-run` previews for `pending retry-failed` and
  `pending purge-failed`.

### Changed
- Defaulted remem's Codex summarization model to `gpt-5.2`; set
  `REMEM_CODEX_MODEL=auto` to use the Codex CLI default.
- Updated model pricing to include current cache/reasoning-aware OpenAI and
  Anthropic price families.
- Serialized schema migrations with `BEGIN IMMEDIATE` to avoid concurrent
  migration races.
- Preserved the context stats footer when context output is truncated and the
  footer fits within the configured character budget.
- Propagated branch, memory-type, stale-state, and offset filters through
  multi-hop search expansion.

### Docs
- Documented usage reporting, precision levels, pricing overrides, and the
  `gpt-5.2` Codex default in English/Chinese README and architecture docs.
- Documented filtered multi-hop CLI search and pending dry-run operations in
  English and Chinese README files.

## [0.3.8] - 2026-04-03

### Packaging
- Excluded local-only artifacts from published package: `eval/local/results/` and `plan/`.
- Published `remem-ai` v0.3.8 to crates.io.

### Docs
- Fixed Cargo install command to `cargo install remem-ai --bin remem` in English and Chinese README files.

## [0.3.5] - 2026-03-26

### Packaging
- Switched SQLCipher build to `rusqlite` feature `bundled-sqlcipher-vendored-openssl`, so release builds no longer depend on runner-provided ARM64 OpenSSL packages.
- Simplified ARM64 release job back to `gcc-aarch64-linux-gnu` linker setup only.

## [0.3.4] - 2026-03-26

### Packaging
- Fixed ARM64 Linux toolchain install on GitHub Ubuntu runners by switching from multi-arch `libssl-dev:arm64` to cross package `libssl-dev-arm64-cross`.
- Updated ARM64 OpenSSL include/lib env paths (`/usr/aarch64-linux-gnu/...`) to match cross toolchain layout.

## [0.3.3] - 2026-03-26

### Packaging
- Fixed GitHub Release ARM64 Linux cross-compilation for SQLCipher by installing ARM64 OpenSSL toolchain (`libssl-dev:arm64`) and setting target-specific include/lib env vars in `release.yml`.
- Kept `reqwest` on `rustls-tls` to avoid unnecessary `native-tls` OpenSSL coupling in release builds.

## [0.3.2] - 2026-03-26

### Packaging
- Switched `reqwest` to `rustls-tls` (disabled default features) to remove `native-tls`/OpenSSL cross-build dependency.
- Fixed Linux ARM64 release build path in GitHub Actions by avoiding target OpenSSL toolchain requirement.

## [0.3.1] - 2026-03-26

### Architecture
- Introduced canonical `ProjectId` normalization and removed ad-hoc project matching paths.
- Added `MemoryService` to unify save/search behavior across MCP and REST API.
- Added `pending_admin` module and CLI commands for failed pending operations.

### Reliability
- Replaced destructive pending deletion on flush errors with recoverable pending state machine:
  `pending` / `processing` / `failed` plus retry metadata.
- Added DB migration to schema v13 for pending retry/failure fields and indexes.

### API / UX
- Unified memory write contract (`text`, `title`, `project`, `scope`, `memory_type`, etc.) for MCP and REST.
- Updated README command/API examples for failed pending inspection and retry.

## [0.3.0] - 2026-03-24

### Search
- **4-channel RRF fusion**: FTS5 + Entity Index + Temporal + LIKE, merged via Reciprocal Rank Fusion
- **Entity index**: Rule-based entity extraction (1600+ unique entities), `remem backfill-entities`
- **Temporal retrieval**: Parse "yesterday"/"上周"/"3 days ago" into time-range filters
- **OR semantics**: Multi-token FTS5 queries match ANY token (was AND)
- **Synonym expansion**: 50+ Chinese↔English term mappings (`query_expand.rs`)
- **Title-weighted BM25**: `bm25(fts, 10.0, 1.0)` — title matches weighted 10x
- **Hybrid routing**: Long tokens → FTS5, short tokens → LIKE, merged with dedup

### CLI
- `remem doctor` — 6-point system health check
- `remem search <query>` — Search memories from terminal
- `remem show <id>` — View memory details
- `remem eval` — Run search quality benchmark against golden dataset
- `remem backfill-entities` — Populate entity index from existing memories
- `remem encrypt` — Encrypt database with SQLCipher
- `remem api --port` — Start REST API server

### API
- REST API server (Axum) with 4 endpoints: search, get, save, status
- CORS support for browser-based integrations
- Binds `127.0.0.1` only

### Security
- SQLCipher encryption at rest (`bundled-sqlcipher`)
- Data directory permissions `0700`, log files `0600`
- Key file `~/.remem/.key` with `0600` permissions

### Architecture
- `ToolAdapter` trait for multi-tool support (Claude Code, future: Codex/Cursor)
- Split `memory.rs` (1308→553 lines) into `memory.rs` + `memory_search.rs` + `memory_promote.rs`
- Fine-grained memory promotion: multi-item decisions/learned split into individual memories
- SQL-layer project suffix-match filter (was post-filter)
- Content-derived titles (was request-prefix truncation)
- Search-friendly summary prompt rules

### Testing
- 128 tests (87 unit + 14 benchmark + 14 promote + 13 integration)
- Benchmark suite: 9 evaluation dimensions, 14 automated tests
- Golden dataset v1.1: 30 real-world queries, 24 with calibrated ground truth
- IR metrics: NDCG, MRR, Precision@K, Recall@K, Hit@K

### Search Quality (1001 real memories, 30 queries)
- MRR: 0.858
- Precision@5: 0.460
- Recall@5: 0.628
- Hit Rate@5: 1.000
- CJK dictionary segmentation: "数据库加密" → "数据库"+"加密" → database+encrypt
- 90+ Chinese↔English synonym mappings
- Core-token LIKE channel (CJK-segmented, no synonym noise)

## [0.2.0] - 2026-03-23

Initial public release with MCP server, hooks integration, session summaries, preferences, and FTS5 search.

## 2026-03-03

### Added
- Added a persistent job queue in SQLite (`jobs` table) with lease/retry/failure states.
- Added worker execution path (`remem worker`) for queued observation/summary/compress jobs.
- Added read-only Bash filtering coverage for `grep`/`rg`/`find`/`git grep` and polling-style `curl` commands.
- Added unit tests for Bash filter behavior to ensure read-only commands are skipped while mutating commands are retained.

### Changed
- Changed `summarize` hook behavior to enqueue jobs and return quickly, then trigger worker processing.
- Changed flush execution path to use `observe_flush` module and worker-driven orchestration.
- Updated install/runtime wiring to include new worker/queue flow.
- Tuned observation capture logic to reduce low-value shell noise in pending queue.
