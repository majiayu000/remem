# Pricing Config Technical Spec

Status: Current contract
Date: 2026-08-18

## Shape

```toml
[pricing]
# optional global override; both input and output are required together
# input_per_mtok = 1.25
# output_per_mtok = 6.5
# reasoning_per_mtok = 6.5
# cache_creation_per_mtok = 1.25
# cache_read_per_mtok = 1.25

# [pricing.haiku]
# input_per_mtok = 1.0
# output_per_mtok = 5.0
```

Accepted global and family keys: `input_per_mtok`, `output_per_mtok`,
`reasoning_per_mtok`, `cache_creation_per_mtok`, `cache_read_per_mtok`.
Integers and finite floats `>= 0` are accepted. Unknown keys or family
tables fail closed.

## Resolution

`runtime_config::global_pricing_override()` reads `REMEM_CONFIG` / the
data-dir `config.toml` and does not write. `ensure_config_defaults` inserts
an empty `[pricing]` table only on init/show/set; it does not write rates.

`ai::pricing` applies env first, then that global override, then family env,
then family `[pricing.<family>]`, then the compiled table.

A global `[pricing]` table with neither input nor output and no other known
keys is a no-op. One of input/output without the other, or optional keys
without both input and output, is an error. An unreadable TOML file is an
error.

Invalid pricing config makes usage recording skip the row and log at error
level instead of writing a fabricated `$0` estimate. Doctor
`check_runtime_config` calls `validate_pricing_config()`.

## Files

- `src/runtime_config/pricing.rs`
- `src/ai/pricing.rs`
- `src/ai/usage.rs`
- `src/doctor/runtime_config_check.rs`
