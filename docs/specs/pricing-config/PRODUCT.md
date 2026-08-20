# Pricing Config Product Spec

Status: Current contract
Date: 2026-08-18

## Problem

USD cost estimates use compiled per-family rates plus `REMEM_PRICE_*`
environment variables. Operators cannot review or persist a price override in
the same `config.toml` that already holds Memory AI and SessionStart budgets.

## Goals

- Global USD overrides have a `config.toml` home under `[pricing]`.
- Per-family overlays live under `[pricing.<family>]` using the existing
  family names (`opus`, `sonnet`, `haiku`, `gpt55`, `gpt54`, `gpt54_mini`,
  `gpt54_nano`, `gpt5_codex`, `gpt5`, `gpt4`, `codex_mini`).
- `REMEM_PRICE_*` remains an env escape hatch with the previous parse rules.
- Present-but-invalid `[pricing]` numbers fail closed.
- Missing file or empty `[pricing]` keeps today's compiled family table.

## Non-Goals

- Changing compiled family rates, including GPT-5.6 credit models staying
  unpriced unless a global override is set.
- Writing compiled family rates into `config.toml` on init (that would pin
  stale prices across remem upgrades).
- Dropping `REMEM_PRICE_*`.

## Behavior

Precedence:

1. Global env (`REMEM_PRICE_INPUT_PER_MTOK` and
   `REMEM_PRICE_OUTPUT_PER_MTOK` both set)
2. Global `[pricing]` (`input_per_mtok` and `output_per_mtok` both set)
3. Family env field overlays
4. Family `[pricing.<family>]` field overlays
5. Compiled family table

A global override still applies to GPT-5.6 credit models. Family tables do
not. Set a global override with
`remem config set pricing.input_per_mtok 1.25` and
`remem config set pricing.output_per_mtok 6.5`.

## Done when

- Fresh config text contains an empty `[pricing]` table and no default rates.
- A complete `[pricing]` pair overrides every model, including GPT-5.6.
- A valid env pair still wins over `[pricing]`.
- A present non-numeric, negative, or one-sided global `[pricing]` fails
  closed.
- Family `[pricing.haiku]` overlays only that family after env is unset.
- Doctor fails when `[pricing]` is invalid.
