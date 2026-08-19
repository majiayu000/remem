# Hook Binary Split Technical Spec

Status: Current contract
Date: 2026-08-18

## Features

```toml
default = ["local-onnx", "eval"]
eval = []
local-onnx = ["dep:fastembed", ...]
```

`src/eval/**` and CLI eval/bench commands compile only with `eval`.
`remem-hook` is a second `[[bin]]` that calls `remem::hook_cli::run()` on a
`current_thread` runtime and never enters the full clap graph.

## Shared hook behavior

`hook_cli` owns hook-host parsing, `REMEM_DISABLE_HOOKS`, and the four hook
command bodies. `cli::dispatch` calls the same functions so `remem context`
and `remem-hook context` cannot drift.

## Install and doctor

`install::config::hook_command` calls `hook_invocation_binary`. Doctor
integrity treats a present sibling `remem-hook` as an allowed executable for
slim subcommands only. `is_remem_command_token` accepts both stems.
`expected_hook_executable_from_hooks` prefers the full `remem` path when
hooks mention both binaries, so `rules eval` still matches the full image.
Doctor install-path probes ignore `remem-hook` so the sibling is not a
second install.

Install/plugin/npm may start shipping `remem-hook` beside `remem` later; this
slice already prefers it when the file exists.
