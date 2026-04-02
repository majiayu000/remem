# Changelog

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
