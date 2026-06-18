# Spec: Audit Remediation (2026-05-29)

> Companion to `docs/audit-2026-05-29.md`. Covers the 5 Critical + 7 High findings.
> Guiding principle (CLAUDE.md): every decision is judged by **memory quality**. Several findings are live recurrences of the two recorded disasters — capture channel producing nothing, and memories that look empty — so they take priority over cost/latency.

## 1. Background

A 4-agent opus audit (data-integrity / security / architecture / persistence) traced the full `captured_events → extraction_tasks → memory_candidates → memories(+FTS)` pipeline plus retrieval, the MCP/API/CLI surface, and the migration/persistence layer. It surfaced five Critical issues where records silently vanish or where the encrypted-at-rest promise is silently broken, and seven High issues. The dual-database problem was independently confirmed by all four agents; the silent-plaintext problem by three.

## 2. Goals

1. No memory or raw turn is silently dropped between capture and retrieval.
2. `include_stale=true` actually returns stale/archived memories.
3. The encryption posture is explicit and fail-closed; no silent plaintext, no false `doctor` failure, no plaintext schema-helper copies or dry-run clones.
4. One database schema, one migration engine.
5. Every error that causes user-visible missing memory/context is `error`-level or raised (U-29).
6. The local HTTP API is not a zero-auth read/write surface for any localhost origin.

## 3. Non-Goals

- The P2 architecture refactors (connection pooling, host-enum unification, god-object splits, typed errors). Tracked separately; behavior-preserving and lower urgency.
- Re-enabling any capture path that was deliberately disabled. We fix silent-drop, not policy.
- Rewriting the LLM extraction prompts (already correct vocabulary-wise except the shared-table change in §5.5).

## 4. Current Behavior (verified `file:line`)

- `apply_cipher_key_if_available` returns `Ok(false)` with no key; `open_db` discards it (`src/db/crypto.rs:23-29`, `src/db/core.rs:70`). Installer never generates a key.
- FTS triggers gate on `status='active'` (`v012_memory_search_context.sql:40,46,52,55,61`); `fts.rs:67-71` JOINs `memories_fts` while filtering `status IN ('active','stale','archived')`.
- `raw_messages` `UNIQUE(project, role, content_hash)` omits `session_id` (`v002_raw_messages.sql:14`); `ON CONFLICT DO NOTHING`; FNV-64 hash.
- `AUTO_PROMOTE_TYPES` (4) vs `OBSERVATION_TYPES` (6) disjoint on `architecture` (`memory_candidate.rs:28`, `db/models.rs:3`).
- `context/query.rs:93,101` swallow DB errors with no log; siblings `:41-64` log at `error`.
- Two `memories` schemas: `v001_baseline.sql:74` (`content`) vs `schema_001_baseline.sql:147` (`text`), two `run_migrations`.

## 5. Design

### 5.1 Fix C2 — explicit, fail-closed encryption (Phase 0)
Introduce a single keyed-open helper and use it everywhere the real DB is opened.

```rust
// src/db/core.rs
fn apply_standard_setup(conn: &Connection) -> Result<()> {
    let encrypted = crate::db::crypto::apply_cipher_key_if_available(conn)?;
    if !encrypted && std::env::var_os("REMEM_ALLOW_PLAINTEXT_DB").is_none() {
        anyhow::bail!(
            "no cipher key (REMEM_CIPHER_KEY unset, {} missing); refusing to open an unencrypted \
             memory DB. Run `remem encrypt` to create/key the DB, or set REMEM_ALLOW_PLAINTEXT_DB=1.",
            data_dir().join(".key").display());
    }
    if !encrypted { crate::log::warn("db", "opening UNENCRYPTED database (REMEM_ALLOW_PLAINTEXT_DB set)"); }
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")?;
    Ok(())
}
```
- `open_db()` calls `apply_standard_setup` before migrations.
- Update `remem encrypt` so it succeeds on a fresh install: generate `.key`, initialize an empty keyed `remem.db` through the canonical migration path when the DB is absent, and only SQLCipher-export an existing plaintext DB when one already exists. The fail-closed error must point only at a command that works on an empty data dir.
- `src/doctor/schema.rs:14` and `src/db/schema.rs:27` (if kept per §5.4) call `apply_cipher_key_if_available` after `Connection::open` — fixes the false `doctor` FAIL (CFG-2) and the plaintext schema-helper copy (CFG-3).
- `doctor` dry-run migration checks must key the destination clone or use an in-memory/keyed clone; copying a decrypted source into a plain temp database is still a plaintext leak.
- This also closes CFG-7 (FK PRAGMA) by centralizing PRAGMA setup.

### 5.2 Fix C3 — stale/archived memories searchable (Phase 0, keystone)
Add migration `v019_memory_fts_all_status.sql`:
- Drop the `WHERE … status='active'` clause from the `memories_ai/au/ad` triggers so **all** rows are indexed.
- Rebuild `memories_fts` from `memories` (full repopulate).
- Visibility is then governed solely by the post-JOIN `m.status` filter (`memory_status_filter_sql`), so FTS and LIKE channels converge (fixes DI-7).
- Re-evaluate `archive_stale_memories` (DI-9): keep archived rows searchable; if age-archiving stays, it no longer removes them from recall.

### 5.3 Fix C4 — per-session raw archive, collision-free hash (Phase 0)
Add migration `v020_raw_messages_session_dedup.sql`: rebuild `raw_messages` with `UNIQUE(project, session_id, role, content_hash)`. Switch `raw_archive.rs` content hashing to SHA-256 (`sha2`, existing dep). The migration must backfill existing `content_hash` values to SHA-256 before the new writer runs, or version the hash and have the insert/idempotence lookup check both old FNV and new SHA-256 during the transition. Old rows cannot simply keep FNV values, because re-draining the same transcript under the SHA-256 writer would miss the old unique key and duplicate raw turns.

### 5.4 Fix C1 — one schema, one migrator (Phase 1; decision required)
**Decision gate:** is the `schema.sqlite` normalization meant to ship?
- **Default recommendation: delete it.** Nothing in the hot path opens it; it diverges from the reader (`MEMORY_COLS`) and is a live data-split trap for `reset-schema`/schema-helper code that writes a DB nobody reads. Remove `src/db/schema/`, `schema_001_baseline.sql`, and the admin/schema-status references. Keep public backup import routed through the canonical `remem.db` writer; it already uses the runtime DB path and is not the current invisible-import path.
- **Alternative:** if normalization must ship, write a separate ExecPlan to migrate all 87 `open_db()` callers and retire `migrate/` + `v001_baseline.sql`.
- Add a guard test: exactly one `memories` DDL source; every `MEMORY_COLS` name exists in it.

### 5.5 Fix C5 — align candidate/observation vocabularies (Phase 0)
One shared support table from candidate `memory_type` to acceptable source-observation evidence types. For `architecture`, update the observation prompts plus parser/model vocabulary so future `<type>architecture</type>` observations are preserved, and explicitly allow already-coerced `discovery` observations to support an `architecture` candidate when the observation/candidate content relationship otherwise matches. Relax `is_supported_by_source_observation` to consult that table instead of raw equality. Any candidate diverted to `pending_review` instead of auto-promoting logs an explicit `error`/`info` reason (no silent divert). Add a test proving the auto-promote and observation vocabularies stay consistent.

### 5.6 Fix H1/H2 — U-29 logging on the context path (Phase 0)
- `context/query.rs:93` → `unwrap_or_else(|e| { crate::log::error("context", &format!("failed to load recent memories for {project}: {e}")); Vec::new() })`.
- `context/query.rs:101` → `match … { Ok(s) => …, Err(e) => crate::log::error(…) }`.
- `context/render.rs:111` and `context/injection_gate.rs:84` → `log::warn` becomes `log::error` (keep fail-open behavior). Cross-link issue #232 (lessons path).

### 5.7 Fix H4 — reactivate on topic-key rewrite (Phase 0)
Constrain/order the lookup at `src/memory/store/write.rs:77-87` so a topic-key upsert updates the active/latest row, or inserts a new row when only stale/superseded rows match. If updating an existing row, set `status='active'` because a write to an existing `topic_key` is a reassertion of currency. This avoids reactivating an arbitrary stale row and creating duplicate active memories.

### 5.8 Fix H3 — local API auth + tight CORS (Phase 2)
Generate `~/.remem/.api-token` (`0600`) at install. Add axum middleware validating the chosen local auth header (`Authorization: Bearer ...` or `x-remem-token`) on all routes; 401 otherwise. Replace the any-localhost CORS predicate with the specific official-client origin, or remove CORS if no browser client exists. If a browser client remains, include the chosen auth header in allowed CORS preflight headers or authenticated browser requests will fail before the middleware runs.

### 5.9 Fix H5 — one capture pipeline (Phase 1; live trace required)
W-01: run a real capture and inspect which of `pending_observations` / `extraction_tasks` receives writes. Then deprecate the legacy queue's write path, remove the `flush_pending` job arm and its lease recovery, and delete the dead extraction front-end. Do not delete either side before the live trace confirms ownership.

### 5.10 Fix H6 — central memory-type enum (Phase 1/3)
A `MemoryType` enum with `as_str`/`label`/`index_order`/`weight`/`auto_promote` methods; derive `context/format.rs`, `sections/index.rs`, `sections/core.rs`, `policy.rs`, `memory_candidate.rs` from it. Add an exhaustiveness test so a new variant fails to compile until all tables cover it. Fixes the live `procedure` label/order/core-policy gap; the index fallback still renders unlisted types, so this is not a total index drop.

### 5.11 Fix H7 — migration integrity (Phase 1)
- `migrate/run.rs:20-27`: `error`-log rollback failure and chain it into the returned error.
- `migrate/transition.rs`: do not mark post-baseline migrations as applied unless their SQL has run or an explicit equivalence check proves the target tables/indexes already exist. Legacy transition should let v002+ migrations create missing objects such as `raw_messages` and `event_blobs`.
- `migrate/run.rs:75`: `user_version = MIGRATIONS.last().version`; move legacy detection to a separate predicate; retire `OLD_BASELINE_VERSION` arithmetic.

## 6. Issue Plan

| Issue | Findings | Severity | Label |
|-------|----------|----------|-------|
| Stale/archived memories invisible on FTS path | C3, DI-7, DI-9 | Critical | bug, p0 |
| sqlcipher silently opens unencrypted DB; doctor/schema helpers skip key | C2, CFG-2, CFG-3 | Critical | bug, security, p0 |
| Raw archive cross-session dedup drops turns | C4 | Critical | bug, p0 |
| Auto-promote type-vocab mismatch → pending-review limbo | C5 | Critical | bug, p0 |
| Dual incompatible DB schema + two migrators | C1, CFG-8 | Critical | bug, p0 |
| Context memory channel swallows DB errors (U-29) | H1, H2 | High | bug, p1 |
| Unauthenticated localhost HTTP API + open CORS | H3 | High | security, p1 |
| Topic-key upsert doesn't reactivate stale rows | H4 | High | bug, p1 |
| Two coexisting capture pipelines | H5 | High | tech-debt, p1 |
| Memory-type vocabulary drift (`procedure` label/order/core policy) | H6 | High | bug, p1 |
| Migration integrity hardening | H7 (CFG-4/5/6/7) | High | bug, p1 |

## 7. Files Expected To Change (Phase 0)

- `src/db/core.rs`, `src/db/crypto.rs`, `src/doctor/schema.rs`, `src/migrate/dry_run.rs` — encryption posture and keyed doctor dry-run clones.
- `src/migrations/v019_memory_fts_all_status.sql` (new), `src/retrieval/memory_search/fts.rs` — FTS visibility.
- `src/migrations/v020_raw_messages_session_dedup.sql` (new), `src/memory/raw_archive.rs`, `src/db/core.rs` (hash) — raw archive.
- `src/memory_candidate.rs`, `src/db/models.rs`, `src/memory/format/parse.rs`, `prompts/observation.txt`, `prompts/task_observation.txt` — vocabulary alignment.
- `src/context/query.rs`, `src/context/render.rs`, `src/context/injection_gate.rs` — U-29 logging.
- `src/memory/store/write.rs` — reactivation.

## 8. Validation Plan

- C2: `REMEM_DATA_DIR=$(mktemp -d) cargo run -- status` fails without a key; with `REMEM_ALLOW_PLAINTEXT_DB=1` succeeds + warns. Doctor test: encrypt a temp DB, assert `Status::Ok`, and assert the dry-run migration clone is keyed/in-memory rather than a plaintext temp DB.
- C3: insert → `govern_memory` to `stale` → `search(q, include_stale=true)` via FTS returns the row. Regression: short + long query tokens return the same stale row.
- C4: insert identical content under two `session_id`s; assert two rows and `search_raw` returns both with correct attribution; seed a pre-migration FNV-hash row, run the v020 path, re-drain the same content, and assert no duplicate.
- C5: already-coerced `discovery` observation + supported `architecture` candidate auto-promotes (or logs explicit reason); future `<type>architecture</type>` observations retain/support `architecture` directly.
- C1: guard test — one `memories` DDL source; all `MEMORY_COLS` names present.
- H1/H2: inject a query/open error; assert an ERROR log is emitted.
- H4: insert → archive/supersede → re-upsert same topic_key; assert exactly one current active row is visible and FTS-searchable.
- H7: synthetic v12 DB → migrate → required v002+ tables/indexes exist, `_schema_migrations` only records versions that ran or were proven equivalent, and fresh-DB `user_version` == latest migration.
- Gate before completion: `cargo check`; before merge: `cargo test`.

## 9. Risks

- v019 rebuilds `memories_fts` — large stores incur a one-time reindex; wrap in a transaction; it's idempotent.
- v020 rebuilds `raw_messages` — additive unique key; verify no existing duplicate rows violate the new key before applying (dedup-merge step in the migration).
- C1 deletion: confirm no remaining admin/schema-helper workflow intentionally relies on `schema.sqlite`; do not rewrite the public backup import path unless new evidence shows it still targets the dormant schema DB.
- H5 deletion: **must** be preceded by a live capture trace (W-01) — deleting the wrong queue stops capture.

## 10. Done When

- All Phase-0 validations pass with fresh `cargo test` output in-session (W-16).
- `include_stale=true` returns stale/archived memories on both channels.
- A default install either encrypts or refuses to open (no silent plaintext); `doctor` is green on encrypted installs.
- No raw turn is dropped across sessions; identical phrases in different sessions are both recallable.
- `architecture`/coerced-type candidates auto-promote or log an explicit reason.
- Exactly one DB schema + one migrator remain in the tree.
- Every context-load failure path emits at `error`.
