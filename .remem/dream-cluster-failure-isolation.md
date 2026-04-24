---
source: remem.save_memory
saved_at: 2026-04-21T16:01:46.115377+00:00
project: remem
---

# Dream apply writes are now atomic and retryable

Symptom: `process_dream_job()` could log and continue after `apply()` errors, so the worker marked the dream job done even when the merged memory insert had succeeded but stale-marking the superseded rows failed later. That left a half-applied cluster visible as both the new merged memory and the old active memories.

Root cause: `src/dream/apply.rs` performed the merged-memory upsert and the stale-mark updates as separate statements without a transaction, and `src/dream.rs` downgraded `apply()` failures to warnings instead of returning an error to the worker.

Fix: wrap `apply()` in a SQLite transaction and commit only after every superseded row is marked stale; if any update affects the wrong number of rows or the DB raises an error, the transaction aborts and no merged memory remains. `process_dream_job()` now logs apply failures at error level and returns the error so `worker.rs` retries the job instead of marking it done.

Prevention: keep multi-step dream writes atomic, and treat write-path failures differently from model/merge failures. Add regressions that force post-insert update failure and verify both rollback and job retry semantics.
