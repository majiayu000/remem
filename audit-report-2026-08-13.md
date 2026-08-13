# remem Codebase Audit Report
> Date: 2026-08-13
> Target: `/private/tmp/remem-gh910-closure.dvuAcU/remem`
> Branch: `agent/legacy-unverified-governance`
> Audit snapshot: `bb53d084` (`feat: quarantine legacy unverified context`)
> Repair commit: `e6cb0593` (`fix: keep unverified memory and spill poison off live paths`)
> Stack: Rust 2021, remem-ai 0.6.70, SQLite/SQLCipher + FTS5 + sqlite-vec, tokio, axum REST, rmcp MCP, clap CLI (~329k LOC in `src/`)
> Mode: full
> Agents: API contract, dataflow, errors/security, architecture, config/persistence, tests, concurrency
> Previous audit: 2026-07-09 (reconciled 2026-07-16 vs main `6e4734cc`); Critical/High ledger had no open items

## Summary

| Level | Count | Verified | Key Areas |
|---|---:|---|---|
| Critical | 2 | 2 | Unbounded Claude/Codex hook stdin; `HOME`-missing data-dir fallback to cwd |
| High / P1 | 7 | 7 | G2 visibility holes (`current_state`, UserPromptSubmit); global preference default; spill poison loop; project-alias miss on user-context recall; two untested critical paths |
| Medium / P2 | 18 | 0 (unverified) | Dual current-truth engines, host extension cost, advertised-but-empty G3 fields, blocking I/O, redaction gaps |
| Refuted C/H | 24 | 24 | MCP↔REST shape drift, plugin empty assets, FTS injection, SSRF, unimplemented extraction kinds, G3-as-current-bug, most persistence/concurrency Highs |

This tree is a local-first memory runtime with three user surfaces (hooks, MCP, REST) on one SQLite store. The 2026-07 ledger’s Critical/High defects are still closed. At audit time the live risk was **G2 quarantine real on SessionStart and incomplete on sibling current-context readers**, plus fail-open config and capture-path bugs. Phases 1–3 of the repair roadmap landed in `e6cb0593`.

## Repair status after e6cb0593

| Phase | Roadmap item | Status |
|---|---|---|
| 1 | G2 on `current_state`, UserPromptSubmit, recall alias filters | Done |
| 2 | `preference_global_limit=0`, `data_dir()` fail-closed, bounded Claude/Codex stdin | Done, with leftovers below |
| 3 | Spill dead-letter + `Drop`; bundle G2 test; stolen-lease archive test | Done, with leftovers below |
| 4 | Shared HostProfile registry; `hook_integrity` exhaustive match; file-size ratchet | Not started |
| 5 | G3: route production current-state through `project_current_truth` | Not started |
| 6 | G4/G5 governance writes + review throughput | Not started |

Closed Critical/High: C2 (HOME/cwd), H3 (`current_state`), H4 (prompt-submit), H5 (global prefs), H7 (recall aliases), H8 (bundle G2 test), H9 (stolen-lease archive test).

Leftovers from otherwise-closed C1 / H6 / Phase 3:

- Stdin timeout still does not abort the reader thread; context still warns and continues empty (`src/context/invocation.rs`).
- Parse/decrypt poison is dead-lettered, but retryable spill still has no N-failure cap, there is no old-key drain before rekey, and `append_file_then_remove` still reads outside the cross-process lock.

Not started product/design work: G3 CurrentTruth as the production boundary, G4 atomic claim writes, G5 review-queue throughput, Cursor install v1 hook registration, `context_bundle` as a stable API. Medium/P2 items remain unverified and unfixed.

## Delta vs Previous Audit

Resolved: 0 (previous Critical/High items were already resolved/refuted)
Still-open: 0
New Critical/High: 9
Re-confirmed refutations: summarize-lock CAS (`concurrency--summarize-lock-non-cas`), unimplemented `RuleCandidate`/`IndexUpdate` (`arch--unimplemented-extraction-kinds`), empty plugin assets (`config--empty-release-assets`)

No regression of a previously **resolved** Critical/High item.

## Critical (Fix Immediately)

### 1. Claude/Codex hook stdin is unbounded
- **File:** `src/hook_stdin.rs:21`
- **Verify:** confirmed
- **Risk:** A huge or slow hook payload can grow without a byte cap. Cursor already caps stdin at 1 MiB; Claude and Codex do not. The timeout only abandons the parent wait — the reader thread keeps `read_to_string`. Context then **warns and continues** with no session data, so SessionStart can look successful while empty.

```21:35:src/hook_stdin.rs
        let input = std::io::read_to_string(std::io::stdin());
        let _ = tx.send(input);
    });

    match rx.recv_timeout(Duration::from_millis(timeout_ms)) {
        Ok(Ok(input)) => { /* keep full String, no size check */ }
        Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
```

- **Fix:** Reuse Cursor’s bounded reader (`read_bounded_hook_stdin`) for Claude/Codex. Abort or drop the reader on timeout. Surface stdin failure in the injected context instead of `warn` + empty success (`src/context/invocation.rs:62`).

### 2. Missing `HOME` silently puts the DB and cipher key in cwd
- **File:** `src/db/core.rs:93`
- **Verify:** confirmed
- **Risk:** If `REMEM_DATA_DIR` is unset and `dirs::home_dir()` is `None` (CI, containers, stripped env), remem creates `./.remem/remem.db` and the SQLCipher key next to whatever the process cwd is. That is an unexpected plaintext-adjacent key placement and a split-brain store versus `~/.remem`.

```85:95:src/db/core.rs
pub fn data_dir() -> PathBuf {
    std::env::var("REMEM_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".remem")
        })
}
```

- **Fix:** Fail closed: require `REMEM_DATA_DIR` or a resolvable home directory. Never default to `"."`.

## High / P1 (Fix This Week)

### Visibility / G2 (this branch’s contract)

#### 3. MCP `current_state` skips `truth::classify_memory`
- **File:** `src/memory/current_state.rs:318`
- **Category:** layer-violation
- **Verify:** confirmed
- **Risk:** G2 keeps `legacy_unverified` rows as `status='active'` and excludes them only via a read-time classifier. SessionStart, MCP `search`, CLI search, and REST helpers call `classify_memory`. `current_state` does not. An active G2 row is returned as `status: "current"` with no exclusion label — the surface documented as the answer for one stable state key.

- **Fix:** Call `classify_memory` (or a shared `current_context_filter_sql`) inside `load_active_memory` / `load_active_state_key_rivals`. Surface excluded rivals with `classification_reason`. Add a regression test that inserts a provenance-missing active row and asserts `current_state` does not call it current.

#### 4. Claude UserPromptSubmit re-injects G2-excluded memories
- **File:** `src/context/prompt_submit.rs:57`
- **Category:** layer-violation
- **Verify:** confirmed
- **Risk:** SessionStart runs `exclude_non_current_context_memories`. The installed Claude `UserPromptSubmit` path (`remem session-init`) retrieves via `query_hybrid_context_memories` (SQL `status='active'` only) and injects on prompt relevance. A row dropped at session start is therefore eligible on the next user prompt. G2’s acceptance claim does not hold for this channel.

- **Fix:** Run the same exclusion helper before the relevance loop. Prefer a single `admit_for_injection()` entry point used by SessionStart, prompt-submit, and any future channel.

### Config drift

#### 5. Global preference injection is on by default; `=0` cannot turn it off
- **File:** `src/context/policy.rs:54` and `parse_usize` at line 281
- **Category:** config-drift
- **Verify:** confirmed
- **Risk:** Docs (`docs/ARCHITECTURE.md`) say `REMEM_CONTEXT_PREFERENCE_GLOBAL_LIMIT` defaults to `0` / disabled. Code defaults to `5`. `parse_usize` treats `0` as unset, so even an explicit `REMEM_CONTEXT_PREFERENCE_GLOBAL_LIMIT=0` falls back to `5`. SessionStart therefore injects up to five global preferences, including legacy `owner_scope IS NULL AND scope='global'` rows from unrelated projects.

```281:287:src/context/policy.rs
fn parse_usize(value: Option<String>) -> Option<usize> {
    let parsed = value?.trim().parse::<usize>().ok()?;
    if parsed == 0 {
        None
    } else {
        Some(parsed)
    }
}
```

- **Fix:** Default `preference_global_limit` to `0`. Parse this limit with `read_usize_allow_zero` (already used for `sessionstart_relevance_k`). Add a test that `from_env_reader(|_| None)` and `from_env_reader` with `"0"` both yield `0`.

### Capture / persistence

#### 6. Unparseable spill lines replay forever
- **File:** `src/summarize/summary_job/spill.rs:91` (via `SpillClaim::finish`)
- **Category:** silent-degradation
- **Verify:** confirmed
- **Risk:** Decode/decrypt/parse failures append the original line to `failed_path`. `finish()` merges that file back into the active queue with no retry cap or dead-letter. Corrupt or post-rekey lines loop on every replay.

- **Fix:** Dead-letter after N failures. Drain/replay spill with the old key before rekey. Do not merge parse-poison into the live queue.

#### 7. User-context recall ignores project aliases
- **File:** `src/user_context/recall/sources.rs:260`
- **Category:** silent-drop
- **Verify:** confirmed
- **Risk:** Capture writes through `canonical_project_path_for_write` (v082 aliases). Memory/context/current_state reads use `push_project_value_filter`. Recall binds the caller’s raw project/cwd string. Querying via an alias path returns empty sessions/claims even though the canonical rows exist. This is the same class of identity bug G1 was meant to close, on a path G1 missed.

- **Fix:** Use `project_filter_values` / `push_project_value_filter` in `collect_recent_sessions`, `collect_memories_for_recall`, and `load_claim_candidates`. Canonicalize MCP/REST/CLI recall project the same way as search.

### Tests for critical paths

#### 8. `compile_session_start_bundle` has no G2 exclusion test
- **File:** `src/context/tests/bundle_candidates.rs:325`
- **Verify:** confirmed
- **Risk:** `load_context_data` G2 tests exist. The bundle compiler (MCP `context_bundle` / GH-932 DTO) never inserts a `legacy_unverified` row. A remap of `context_preselection_drops` in the executor can regress without failing CI.

- **Fix:** Insert a provenance-missing memory, compile a bundle, assert `current_truth`/`core` excludes it and the audit reason is the G2 code.

#### 9. Wrong-owner `archive_claimed_exact_replay_task` is untested
- **File:** `src/db/extraction/lifecycle.rs:145`
- **Verify:** confirmed
- **Risk:** After an AI call, `extraction_worker.rs:208` archives the exact-replay task under the original lease owner. If the lease was stolen, the SQL owner predicate must fail closed. That branch has no test; matching-owner archive is covered, stolen-lease archive is not.

- **Fix:** Claim, expire, let a second owner claim, archive as the stale owner, assert the row still belongs to owner B.

## Medium / P2 (Plan to Fix) — unverified

### Architecture and design

| ID | File | Summary |
|---|---|---|
| Dual current-truth engines | `src/memory/current_state.rs:267` vs `src/truth/projection.rs:222` | `current_state` can return `unresolved_conflict` **and** a populated `current` (newest-wins for clients that ignore status). `truth::projection` abstains. Only `doctor` uses the projection. |
| Advertised G3 fields are always empty | `src/context_bundle/domain.rs:137` | `projection_ref` / `evidence_refs` are in the MCP output schema; every producer sets `None` / `[]`. |
| `RetrievalPlan.channels` hashed, not executed | `src/retrieval_router/planner.rs:488` | 15 channel plans fold into durable `plan_hash`; SessionStart never reads `plan.channels`. |
| Three-way import cycle | `src/context_bundle/compile.rs:12` | `context` ↔ `context_bundle` ↔ `retrieval_router`. Planner/executor/loader cannot be tested in isolation. |
| Unequal channel taxonomies | `src/retrieval_router/domain.rs:52` | 15 `RetrievalChannel` vs 6 `ChannelKind` vs 7 `SectionKind`; no exhaustive mapping. |
| Host extension cost | `src/identity.rs:41` | New host ≈ 23 existing files + ~12 new ones. `hook_integrity::expected_specs` fail-opens unknown hosts to Claude’s specs (`src/hook_integrity.rs:144`). |
| 800-line cap gaming | `scripts/ci/check_file_size.py:17` | Six files at exactly 800 lines; allowlist still has headroom vs current size; 209 functions >100 lines. |
| Vestigial API DI | `src/api/types.rs` `DbState` | Handlers take `State(DbState)` then call global `open_db()`. |

### Incomplete product (documented, not a regression)

These are **not done**, and they are load-bearing for the governance tracker in `docs/todo/README.md`:

| Work | Status | Why it matters |
|---|---|---|
| G3 — CurrentTruth as production current-state boundary | Not started | SessionStart still maps G2-eligible core memories into the bundle’s `current_truth` **channel name**. Equal-trust conflicts are not abstentions. Doctor-only `project_current_truth`. |
| G4 — Atomic claim + evidence on new writes | Not started | Compound candidates and weak evidence can still become current after G3. |
| G5 — Review-queue throughput | Not started | Tracker baseline: 12,525 pending, 0 resolved in 7d, median age ~18 days. |
| Cursor install v1 hook registration | Documented gap | Runtime observe/summarize exist; install does not register hooks, so capture is not automatic. |
| `context_bundle` stable API | Experimental | Schema v1, local embeddings only, not the production SessionStart renderer. |

G3 was filed as High and **refuted** as a current user-visible SessionStart bug (G2 already filters that path). It remains the largest **design** gap: two “current” engines, an unused projection, and a channel named `current_truth` that is not CurrentTruth.

### Data / registry (unverified)

- `RESOLVED_STATUSES_SQL` omits `accepted` / `auto_promoted` (`src/memory_candidate/review_stats.rs:5`) — dashboard undercount. Preference writers do not use `accepted`; still a registry drift vs `truth/lifecycle.rs`.
- `ensure_config_defaults` bootstraps Claude+Codex only; `CURSOR_HOST` omitted at most call sites (`src/runtime_config.rs:95`).
- `memory_status_filter_sql(include_inactive=true)` is `active|stale|archived` and skips `superseded`/`quarantined` (`src/memory/types.rs:192`).
- MCP `save_memory` defaults `host` to `"codex-cli"`; REST defaults to `"api"` (`src/mcp/server/write_tools.rs:80`). Provenance pollution, not a shared DTO bug.
- MCP `govern_memory` lacks `expected_version` / `idempotency_key` / `deny_unknown_fields` that REST safe-mutation requires.
- MCP `recall_user_context` falls back to process cwd; REST returns 400.
- Embedding upsert key is `(memory_id, model, dimensions)` — no provider/base_url (`src/retrieval/vector.rs:36`).
- MCP `search` / `search_raw` have no upper `limit` (REST caps at 100). Local stdio, so Medium not High.

### Security / silent degradation (unverified)

- `constant_time_eq` returns early on length mismatch (`src/api/auth.rs:179`).
- `redact_token` misses short prefixed secrets (e.g. 20-char `AKIA…`) (`src/adapter/redaction.rs:409`).
- MCP search logs full query text at info (`src/mcp/server/search_tools.rs:103`).
- Cursor `validate_transcript_path` accepts any non-empty path (`src/cursor_hook/identity.rs:115`).
- Key backup `fs::copy` may not re-apply `0o600` (`src/db/crypto.rs:473`).
- `SpillClaim` has no `Drop`; panic orphans the claim until next startup.
- `append_file_then_remove` reads outside the cross-process lock (TOCTOU / duplicate append).

### Concurrency / performance (unverified; Highs refuted)

Sync `rusqlite` and `reqwest::blocking` run on tokio worker threads in the API, MCP, and extraction worker. Verifiers correctly refused High: hooks are a separate process, API binds `127.0.0.1`, SessionStart embedding is already 2s-capped (GH-952). Remaining work is throughput: `spawn_blocking` for DB/HTTP, stop advancing vec-index on every `open_db()`, consider a long-lived connection pool instead of empty `DbState`.

Hybrid-context FTS preselects `status='active'` then N+1 `classify_memory`. Fail-closed on classify error, but the extra round-trips are avoidable with SQL-side G2 predicates.

### Tests (unverified)

Weak `is_some()` assertions on cache versions, dream `operation_id`, poisoning ack epochs, and retry backoff; MCP tool descriptions are substring-only; governance validator tests static JSON, not the live writer.

## Refuted by Verification

| Finding | Why it is not a current High/Critical |
|---|---|
| `try_acquire_summarize_lock` DEFERRED race | Live Stop path is SessionRollup; `process_summary_job_input` is test-only. Same as 2026-07 refutation. |
| External candidates `evidence_event_ids='[]'` → G2 | `[]` vs NULL differ in SQL, but those rows are not a SessionStart promotion path; they fail candidate proof anyway. |
| AI `base_url` SSRF | Local `config.toml` only; not remotely injectable. |
| Public FTS `MATCH` sink | All production callers run `sanitize_fts_query` first. |
| `RuleCandidate` / `IndexUpdate` Deferred | No production writer; tests only. Same as 2026-07. |
| `KNOWN_REVIEW` missing `accepted`/`noop` | Preference candidates are never written as `accepted`. |
| G3 SessionStart bypass as High | G2 already filters SessionStart; `current_truth` is the Core channel alias, not an unimplemented projection leaking bad rows. |
| MCP vs REST search JSON (`results`/`data`, `type`/`memory_type`, epoch vs clock string, staleness) | Documented separate contracts (compact MCP vs REST MemoryItem). |
| Relative `db_path` vs `absolute_data_dir` | Same directory in practice; no production `chdir`. |
| No `DELETE` on `context_injection_items` | Append-only audit log by contract. |
| `claim_next_job` Deferred | Optimistic lost-race; row stays pending. Worker is a file-lock singleton. |
| Audit cleanup leaving item rows | Intended longer-lived emission log. |
| `remem-releases.json` 0.6.70 empty assets | Unreleased staging manifest; CI requires this. Same class as 2026-07. |
| G2 tests on partial schema | Later migrations do not change G2 classification columns. |
| Vacuous policy-gate `is_ok()` | Test’s thesis is fail-closed; open-path FTS is covered elsewhere. |
| Blocking `open_db` / `reqwest::blocking` / unbounded MCP limit as High | Real code, wrong severity for a localhost/stdio single-user split from hooks. |

## Repair Roadmap

| Phase | Scope | Est. files | Why first |
|---|---|---|---|
| 1 | Close G2 on every current-context reader: `current_state`, `prompt_submit`, recall alias filters; one `admit_for_injection` / `current_context_filter_sql` | 6–10 | Users can see quarantined rows as current **now** |
| 2 | Config fail-closed: `preference_global_limit=0` + `read_usize_allow_zero`; `data_dir()` no cwd fallback; bounded Claude/Codex stdin | 4–6 | Silent wrong context + key placement |
| 3 | Spill dead-letter + Drop + lock-scoped append; tests for bundle G2 and stolen-lease archive | 5–8 | Capture-path poison and CI holes |
| 4 | Shared HostProfile registry; `hook_integrity` exhaustive match; ratchet file-size allowlist | 20+ (incremental) | Extension cost |
| 5 | G3: route SessionStart/MCP/API through `project_current_truth`; stop returning `current` alongside conflicts; populate or un-advertise `projection_ref` | 15–25 | Design end-state |
| 6 | G4/G5 governance writes + review throughput | tracker-owned | Queue cannot converge until G3 exists |

`.audit/` is not in `.gitignore`. This repo is tracked; consider ignoring `.audit/` so ledger files are not committed (do not edit `.gitignore` in this audit).

Full narrative report: `audit-report-2026-08-13.md` (this file). Ledger: `.audit/findings.json`.
