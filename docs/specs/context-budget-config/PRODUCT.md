# Context Budget Config Product Spec

Status: Current contract
Date: 2026-08-17

## Problem

SessionStart budgets live as `REMEM_CONTEXT_*` environment variables. Defaults
cannot be reviewed in one file, section-vs-total compatibility is not
validated at a config layer, and `REMEM_CONTEXT_PREFERENCE_GLOBAL_LIMIT`
documentation already drifted from the code default.

## Goals

- SessionStart numeric budgets have a `config.toml` home under `[context]`.
- Environment variables remain escape hatches with the existing parse rules.
- Present-but-invalid `[context]` integers fail closed.
- Missing file or missing `[context]` keeps today's compiled defaults.

## Non-Goals

- Moving gate, host, debug, delta, bundle-render, or price knobs.
- Changing compiled defaults, including `preference_global_limit = 5`.
- Changing env `0` handling except `REMEM_CONTEXT_RELEVANCE_K`.
- Dropping `REMEM_CONTEXT_OBSERVATIONS`.

## Behavior

Precedence for each budget: valid env override, then `[context]`, then
compiled default. `total_char_limit` remains the final render cap; a larger
section budget is truncated, not rejected.

`remem config show` / `init` write the `[context]` keys so operators can see
and set them with `remem config set context.total_char_limit 8000`.

## Done when

- Fresh config text contains `[context]` with the current compiled defaults.
- A `[context]` integer overrides the default when env is unset.
- A valid env value still wins over `[context]`.
- A present non-integer `[context]` value fails closed.
- Existing `from_env_reader` tests keep their env-only behavior.
