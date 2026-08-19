# Hook Binary Split Product Spec

Status: Current contract
Date: 2026-08-18

## Problem

Every host hook currently execs the full `remem` binary. That binary also
links local ONNX embeddings and the eval suite. The Stop dispatcher already
returns in milliseconds, but process image size and unused code remain a
long-term tax on every PostToolUse and SessionStart spawn.

## Goals

- Ship a dedicated `remem-hook` entry that only accepts hook subcommands:
  `context`, `session-init`, `observe`, `summarize`.
- That entry must compile without the `eval` and `local-onnx` features.
- Default `remem` install, plugin, and npm wrapper stay on the full binary
  when `remem-hook` is not sitting next to it.
- When a sibling `remem-hook` exists beside the installed `remem`, hook
  commands that remem-hook can run (`context`, `session-init`, `observe`,
  `summarize`) are written to that slim binary. `rules eval` and MCP stay on
  full `remem`.

## Non-Goals

- Requiring package managers to ship `remem-hook` in this slice.
- A unix-socket daemon or a second crate/workspace package.
- Removing SQLCipher from the hook path (capture still writes the DB).

## Behavior

`remem-hook <hook-command>` runs the same capture/context/summarize behavior
as `remem <hook-command>`. Any other subcommand is rejected. `REMEM_DISABLE_HOOKS`
still no-ops hook commands.

Build the slim image with:

```bash
cargo build --release --no-default-features --bin remem-hook
```

## Done when

- `cargo check --no-default-features --bin remem-hook` passes.
- `remem-hook worker` / `remem-hook eval` fail closed.
- Default `cargo test` still includes eval commands.
- `remem install` writes slim hook commands to sibling `remem-hook` when
  that file exists, and leaves `rules eval` plus MCP on `remem`.
- Doctor accepts that mixed layout and does not treat `remem-hook` as a
  second remem install.
