# Changelog

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
- 123 tests (82 unit + 14 benchmark + 14 promote + 13 integration)
- Benchmark suite: 9 evaluation dimensions, 14 automated tests
- Golden dataset: 30 real-world queries for `remem eval`
- IR metrics: NDCG, MRR, Precision@K, Recall@K, Hit@K

### Eval Baseline (953 real memories, 30 queries)
- MRR: 0.272
- Recall@5: 0.272
- Hit Rate@5: 0.346

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
