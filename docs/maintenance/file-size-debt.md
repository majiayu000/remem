# File Size Debt

The repository keeps a hard ceiling of 800 lines for source and test files.
`scripts/ci/check_file_size.py` prevents new oversized files and prevents the
current oversized baseline from growing while the backlog is split down.

## Current Baseline

Captured from `origin/main` on 2026-06-25:

| Lines | File | Split Priority |
|---:|---|---|
| 2088 | `src/user_context/extraction/tests.rs` | Test split |
| 2041 | `src/api/tests.rs` | Test split |
| 1221 | `src/graph_candidate/tests.rs` | Test split |
| 1193 | `src/memory/current_state/tests.rs` | Test split |
| 1181 | `tests/benchmark.rs` | Test split |
| 982 | `src/mcp/server/tests.rs` | Test split |
| 922 | `src/cli/tests.rs` | Test split |
| 896 | `src/db/extraction/tests.rs` | Test split |
| 884 | `src/api/tests/web_regressions.rs` | Test split |
| 858 | `src/doctor/tests.rs` | Test split |
| 838 | `src/migrate/tests.rs` | Test split |
| 833 | `src/git_trace.rs` | First production split target |
| 823 | `src/context/tests/load.rs` | Test split |
| 822 | `src/db/query/stats/tests.rs` | Test split |
| 818 | `src/context/tests/render.rs` | Test split |
| 809 | `src/retrieval/search/memory/tests.rs` | Test split |
| 803 | `src/memory/staleness/tests.rs` | Test split |
| 803 | `plugins/remem/apps/remem/server.test.js` | Test split |

## Policy

- New source files above 800 lines fail CI.
- Allowlisted oversized files fail CI if they grow above the captured baseline.
- When a file is split below 800 lines, remove it from
  `scripts/ci/check_file_size.py`.
- Split tests by behavior or API surface, not by arbitrary line ranges.
- Split `src/git_trace.rs` before adding substantial new git-trace behavior.

This guard is intentionally non-regression first. It avoids a risky broad
refactor while making the remaining debt visible and enforceable.
