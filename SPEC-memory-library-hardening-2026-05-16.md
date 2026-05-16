# Spec: Memory Library UX Hardening

Status: Implemented in PR branch
Date: 2026-05-16
Branch: `codex/memory-library-spec`

## 1. Background

The memory-system review found that a good memory library is not just a larger
store or a louder retrieval claim. It needs predictable scope, explainable
retrieval, safe administrative controls, and context output that remains
auditable when it is clipped for prompt budget.

`origin/main` already contains the most urgent scope fix in PR #58:

- `save_memory(type="preference")` defaults to `project` unless `scope=global`
  is explicit.
- global preferences are queried only from explicit `scope='global'` rows.
- SessionStart project preferences no longer use the project/global overlay.

This spec intentionally does not reopen that fixed path. It covers three small,
user-visible hardening slices that remain in the current code:

1. CLI search does not expose the canonical memory-service search behavior.
2. failed pending-observation mutation commands do not have a dry-run mode.
3. total context truncation can remove the final context statistics line.

## 2. Goals

- Make `remem search` use the same service-level search path as MCP and REST.
- Add CLI flags for `offset`, `branch`, `include-stale`, and `multi-hop`.
- Surface `has_more`, multi-hop metadata, and raw archive fallback hits in CLI
  search output.
- Add `--dry-run` to `remem pending retry-failed` and
  `remem pending purge-failed` so data-changing commands can be previewed.
- Keep the final context statistics footer visible when
  `REMEM_CONTEXT_TOTAL_CHAR_LIMIT` truncates SessionStart output.
- Keep changes code-only. Do not migrate or mutate existing user data.

## 3. Non-Goals

- No new vector database, embedding pipeline, graph database, or reranker.
- No schema migration for governance fields in this slice.
- No change to memory scope semantics already fixed by PR #58.
- No change to MCP or REST search response shapes.
- No change to default `include_stale` semantics for MCP/REST.
- No automatic deletion, cleanup, or repair of existing memories.

## 4. Current Behavior

### 4.1 CLI Search

`src/cli/actions/query/search.rs` calls `crate::retrieval::search::search`
directly with fixed values:

- `offset = 0`
- `include_stale = false`
- no branch filter
- no multi-hop path
- no `has_more`
- no raw archive fallback display

Meanwhile `src/memory/service/search.rs` is the canonical path used by MCP and
REST. It supports:

- `offset`
- `branch`
- `include_stale`
- `multi_hop`
- over-fetching for `has_more`
- raw archive fallback when curated results are sparse

The mismatch makes CLI debugging unreliable.

### 4.2 Pending Admin

`remem pending retry-failed` and `remem pending purge-failed` immediately mutate
rows. This is risky when failed rows are the only debugging evidence.

Existing queries already support listing failed rows. We can implement preview
counts without changing the mutation SQL.

### 4.3 Context Truncation

`enforce_total_char_limit()` currently keeps the first `N` chars and appends a
truncation marker. Because the stats footer is appended before truncation, the
footer can be removed. The user then sees that context was truncated but loses
the most useful audit signal: how many memories/preferences/sessions were
actually loaded.

## 5. Design

### 5.1 CLI Search Parity

Add CLI fields to `Commands::Search`:

```rust
Search {
    query: String,
    #[arg(long, short)]
    project: Option<String>,
    #[arg(long, short = 't')]
    memory_type: Option<String>,
    #[arg(long, short = 'n', default_value = "10")]
    limit: i64,
    #[arg(long, default_value = "0")]
    offset: i64,
    #[arg(long)]
    branch: Option<String>,
    #[arg(long)]
    include_stale: bool,
    #[arg(long)]
    multi_hop: bool,
}
```

`run_search()` should build `crate::memory::service::SearchRequest` and call
`crate::memory::service::search_memories()`.

Output rules:

- Keep the existing compact memory lines for curated memories.
- Print `Found N result(s)` from returned memories, not raw hits.
- If `has_more` is true, print a short line such as:
  `More results available; use --offset <next>.`
- If `multi_hop` metadata exists, print `Multi-hop: hops=N` and discovered
  entities when non-empty.
- If curated results are empty but raw hits exist, do not print `No results
  found.`; instead print raw fallback hits under `Raw archive fallback`.
- If both curated memories and raw hits exist, print raw hits after curated
  results under a clearly separate section.
- Raw hit previews must include id, role, project, date, optional branch, and a
  short first-line preview. Do not print full transcript content.

### 5.2 Pending Dry Run

Add `dry_run: bool` to:

- `PendingAction::RetryFailed`
- `PendingAction::PurgeFailed`

For retry dry-run:

- Query candidate failed rows with the same project filter and limit used by the
  mutating retry path.
- Print `Would move N failed rows back to pending.`
- Do not update `status`, retry fields, lease fields, or `last_error`.

For purge dry-run:

- Count rows that match the same project and cutoff used by purge.
- Print `Would purge N failed rows older than D day(s).`
- Do not delete rows.

Implementation options:

- Add read-only helpers in `src/db/pending/admin/query.rs`, e.g.
  `count_failed_retry_candidates()` and `count_failed_purge_candidates()`.
- Keep mutation helpers in `mutate.rs` unchanged except tests may reuse shared
  SQL-building helpers if that genuinely reduces duplication.

### 5.3 Context Footer Preservation

Split context truncation into body and footer awareness.

Minimal implementation:

- Add an internal function such as
  `enforce_total_char_limit_preserving_footer(output, char_limit, footer)`.
- In `generate_context()`, build the stats footer as a separate string, append
  it, then pass the footer to the truncation helper.
- If truncation is needed and the footer plus marker fits within the limit,
  keep the beginning of the context, append the marker, then append the footer.
- If the footer plus marker does not fit, fall back to the existing simple
  truncation behavior.

The final output must always stay within `char_limit` when `char_limit > 0`.

## 6. Issue Plan

Created three GitHub issues:

1. #163 CLI search parity with MCP/REST service search.
2. #164 Dry-run mode for failed pending-observation mutations.
3. #165 Preserve context stats footer when SessionStart output is truncated.

One PR may close all three because the slices are small and share one user
theme: memory library observability and safe local operation. If review prefers
smaller patches, the implementation can be split without changing this spec.

## 7. Files Expected To Change

- `SPEC-memory-library-hardening-2026-05-16.md`
- `src/cli/types.rs`
- `src/cli/dispatch.rs`
- `src/cli/actions/query/search.rs`
- `src/cli/actions/query/tests.rs`
- `src/cli/actions/pending.rs`
- `src/db/pending/admin/query.rs`
- `src/db/pending/admin/tests.rs`
- `src/context/render.rs`
- `src/context/tests/load.rs`

No database files or user data files should be modified.

## 8. Validation Plan

Before PR:

```bash
cargo check
cargo test
git diff --check
```

Targeted tests to add or update:

- CLI search request construction/output covers branch, offset, include-stale,
  multi-hop, `has_more`, and raw fallback formatting.
- pending dry-run count tests prove no row mutation occurs.
- context truncation test proves marker and `context memories loaded` footer
  are both retained when they fit.

Smoke commands after build:

```bash
cargo run --quiet -- search memory --limit 2 --offset 0
cargo run --quiet -- pending retry-failed --dry-run --limit 5
cargo run --quiet -- pending purge-failed --dry-run --older-than-days 7
REMEM_CONTEXT_TOTAL_CHAR_LIMIT=500 cargo run --quiet -- context --cwd .
```

## 9. Risks

- CLI output changes may affect scripts that parse `remem search` text output.
  Mitigation: preserve existing memory line shape and only add metadata/footer
  lines.
- `include_stale` default differs between old CLI and canonical service default.
  Mitigation: CLI `--include-stale` should default false to preserve old CLI
  behavior while still using the service path.
- Raw archive fallback may expose transcript snippets. Mitigation: show only
  short previews and clearly label them as raw fallback.
- Pending dry-run counts must match mutation filters. Mitigation: tests should
  compare dry-run count with actual mutation count on equivalent fixtures.

## 10. Done When

- The three GitHub issues exist and link to this spec.
- A PR exists from `codex/memory-library-spec` to `main`.
- PR body closes the three issues.
- `cargo check`, `cargo test`, and `git diff --check` pass from this session.
- Final response clearly separates local code status, git commit/push status,
  PR status, and data mutation status.
