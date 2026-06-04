# Spec: memory-AI config decoupling

## Goal

Move remem's memory-AI policy out of hook commands and legacy environment
variables into one runtime config file:

```text
~/.remem/config.toml
```

`REMEM_CONFIG` may point to another config file. Hooks pass only `--host`; the
config decides executor, model, CLI path, context color/gate, and capture
adapter.

## Default Config

```toml
version = 1

[memory_ai]
default_host = "codex-cli"

[memory_ai.hosts."codex-cli"]
memory_profile = "codex"
context_gate = "strict"
context_color = true
capture_adapter = "codex-cli"

[memory_ai.hosts."claude-code"]
memory_profile = "claude"
context_gate = "off"
context_color = true
capture_adapter = "claude-code"

[memory_ai.profiles.codex]
executor = "codex-cli"
model = "gpt-5.4-mini"
reasoning_effort = "low"
path = "codex"

[memory_ai.profiles.claude]
executor = "claude-cli"
model = "haiku"
path = "claude"

[memory_ai.profiles.anthropic_http]
executor = "http"
model = "haiku"
base_url = "https://api.anthropic.com"
```

## Selection Rules

1. Explicit `--profile` wins.
2. Explicit/stored `--host` maps through `[memory_ai.hosts."<host>"]`.
3. Missing host uses `[memory_ai].default_host`.
4. `--host` and `--profile` are mutually exclusive.
5. Codex model `auto` means omit `--model` and let Codex choose.

The same selection path is used for summarize, session rollup, observation
extract, memory candidate, compress, and dream.

## Hook Contract

Installed hooks are model-free and executor-free:

```text
remem context --host codex-cli
remem summarize --host codex-cli
remem context --host claude-code
remem session-init --host claude-code
remem observe --host claude-code
remem summarize --host claude-code
```

`remem install` creates or updates `~/.remem/config.toml`. `remem doctor` warns
when hooks still contain legacy memory-AI policy strings.

## CLI Contract

```bash
remem config path
remem config show
remem config init
remem config set memory_ai.profiles.codex.model gpt-5.4-mini
remem model current
remem model list
remem model use cheap
remem model use balanced --dry-run
remem model use gpt-5.2 --reasoning medium
remem model use haiku --host claude-code
remem model test
remem model test --live
remem model rollback
remem summarize --host codex-cli
remem summarize --profile codex
remem dream --profile codex
```

`remem model` is the user-facing model switcher. `config set` remains the
low-level escape hatch. `model test` is config-only by default and calls AI only
with `--live`; `model use` saves a rollback backup before writing
`~/.remem/config.toml`.

## Breaking Changes

Runtime memory-AI routing no longer reads:

- `REMEM_EXECUTOR`
- `REMEM_SUMMARY_EXECUTOR`
- `REMEM_COMPRESS_EXECUTOR`
- `REMEM_DREAM_EXECUTOR`
- `REMEM_MODEL`
- `REMEM_CODEX_MODEL`
- `REMEM_CLAUDE_PATH`
- `REMEM_CODEX_PATH`
- `ANTHROPIC_BASE_URL`

HTTP authentication remains environment-based through `ANTHROPIC_API_KEY` or
`ANTHROPIC_AUTH_TOKEN`.

## Acceptance

- Hooks contain only `--host`, not model/executor env vars.
- Codex defaults to `gpt-5.4-mini` with `low` reasoning effort.
- Claude Code can be configured independently through its host profile.
- `cargo check`, `cargo test`, and `cargo build --release` pass.
