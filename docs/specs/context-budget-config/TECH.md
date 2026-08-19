# Context Budget Config Technical Spec

Status: Current contract
Date: 2026-08-17

## Shape

```toml
[context]
total_char_limit = 12000
candidate_fetch_limit = 120
memory_index_limit = 50
memory_index_char_limit = 4000
core_item_limit = 6
core_char_limit = 3000
session_count = 5
self_diagnostic_limit = 2
preference_project_limit = 20
preference_global_limit = 5
preference_char_limit = 1500
lesson_limit = 4
lesson_char_limit = 1200
relevance_k = 1
```

## Resolution

`runtime_config::context_budget_limits()` reads `REMEM_CONFIG` / the data-dir
`config.toml` and does not write. `ensure_config_defaults` inserts missing
`[context]` keys only on init/show/set.

`ContextLimits::from_runtime()` starts from that file result, then applies the
existing env overlay (`from_env_reader` semantics, including the
`REMEM_CONTEXT_OBSERVATIONS` alias and `0`-means-unset except relevance `k`).

Production SessionStart / Context Bundle compilation uses `from_runtime`.
Unit tests that inject env maps keep `from_env_reader`.

A present `[context]` value that is not an integer `>= 0` is an error. An
absent key uses the compiled default. An unreadable TOML file is an error.

## Files

- `src/runtime_config/context.rs`
- `src/context/policy.rs`
- `src/context/host.rs`
- production callers of `ContextLimits::from_env` / `default_policy`
