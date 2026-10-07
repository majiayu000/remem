# DSH plugin technical contract

Refs #1111.

## Decision and evidence

Adapt the existing remem CLI instead of building another memory service. Official
DSH `agent/pre-step` uses a middleware decision; `session/event` is a post-commit
notification and `session/flush` is an awaited durability checkpoint. Verified
against deepseek-ai/deepseek-harness origin/master on 2026-10-07 and the published
`@deepseek-ai/dsh-agent` / `dsh-session` 0.2.0-rc.2 packages. Cordis is 4.0.4.

The plugin lives in `plugins/dsh-remem`. It serializes CLI work per Session object
and waits for capture before returning pre-step context. Human `user/message`
events use `session-init`; assistant messages and tool results use `observe`.
`turn/end` invokes `summarize`, with the latest assistant answer from that turn.
Each capture uses the session's immutable `header.cwd`; absent cwd is an explicit
error rather than falling back to the plugin process directory.

A narrow Rust adapter accepts the normalized hook wire format under
`--host deepseek-harness`. It uses the existing capture/spill/redaction/extraction
paths and a generic tool-result summary. The context profile reports no MCP
registration. Summaries consume the existing capture ledger; no DSH log parser,
new schema, host migration, or extraction implementation is added.

Subprocesses use argv and stdin, never a shell. Output and runtime are bounded.
Captured payloads go through remem's redaction and governance; subprocess stderr
is not copied into plugin diagnostics (it can contain provider secrets).

## Validation

Typecheck/build and package installation must use published DSH packages. Tests
exercise real Cordis SessionStore events and awaited middleware; a live remem
executable smoke verifies DSH host rows, prompt/tool/assistant capture, Stop and
extraction-task queueing in an isolated store. Memory AI promotion requires the
operator's configured executor and is not implied by offline capture verification.
