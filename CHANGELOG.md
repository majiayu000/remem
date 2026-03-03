# Changelog

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

### Notes
- This release focuses on improving throughput stability and reducing queue noise under high-frequency tool usage.
