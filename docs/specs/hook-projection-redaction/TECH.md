# Hook and Projection Redaction Technical Spec

Status: Current contract
Date: 2026-09-13

## Module layout

`src/adapter/redaction/` owns shared scrubbing:

- `mod.rs` — public entry points and hook/value walkers
- `tokens.rs` — token walks, shell-word parsing, inline assignments, attached
  short-option recognition
- `keys.rs` — sensitive-key predicates and single-token heuristics

`src/adapter/common.rs` re-exports the projection and capture entry points used
by MCP, CLI, and hook callers. Workstream matching continues to use canonical
DB values; only output projection calls the projected helpers
(`src/workstream/projection.rs`).

## Mode flags

`redact_tokens(line, redact_sensitive_options, preserve_filesystem_paths)`:

- `redact_sensitive_options=false` for `redact_sensitive_text` so attached
  short-option detection and option-argument lookahead stay off.
- `redact_sensitive_options=true` for projection and hook payload preview.
- Quote grouping activates for Bearer/option arguments and for attached quoted
  short options (`-u"..."`), using `take_shell_like_argument`.
- Rebuilt output preserves the original leading/internal/trailing whitespace
  spans between tokens instead of joining with a single ASCII space.

## Path exemption

`preserve_filesystem_paths=true` only through
`redact_projected_project_text`. High-entropy path segments stay readable for
project correlation; other projected fields use strict token redaction.

## Validation

```bash
python3 scripts/ci/check_file_size.py
cargo test --lib adapter::common::tests -- --nocapture
cargo fmt --check
cargo check
```
