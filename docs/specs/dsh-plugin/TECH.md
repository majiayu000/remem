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
`turn/end` invokes `summarize`, preserving its structured reason and the latest
assistant answer from that turn. Context injection preserves downstream
request-series decisions. DSH requests `context --gate off`: only the most recent
committed `remem` snapshot in `session.deriveMessages()` suppresses an unchanged
snapshot. Changed or empty memory replaces each obsolete remem surface node with
an empty native `developer/message`, citing only that node's original event seq.
DSH omits empty developer nodes from derived requests; the append-only snapshot
and human prompt events stay intact. The fresh snapshot still enters through
normal pre-step admission, so cancellation cannot consume its later delivery.
Cancellation before admission therefore leaves the next turn eligible
for injection. No plugin acknowledgement ledger is maintained. Rust context emission audit is
skipped for the DSH host because CLI rendering precedes admission, cancellation,
and snapshot deduplication; committed DSH history remains delivery authority.
Human attachment blocks retain bounded reference metadata alongside text without
reading attachment bytes. DSH `session-init` is capture-only because its
post-commit callback cannot deliver prompt recall to request preparation; other
hosts retain their existing prompt-recall behavior.
Each capture uses the session's immutable `header.cwd`; absent or relative cwd is an explicit
error rather than falling back to the plugin process directory.

A narrow Rust adapter accepts the normalized hook wire format under
`--host deepseek-harness`. It uses the existing capture/spill/redaction/extraction
paths and a generic tool-result summary. The context profile reports no MCP
registration. Summaries consume the existing capture ledger; no DSH log parser,
new schema, host migration, or extraction implementation is added.

The existing InstallHost variant names and database host strings remain unchanged.
Adding DeepSeekHarness requires downstream exhaustive Rust matches on InstallHost
to handle the new variant. This source-breaking addition is staged as unpublished
0.7.0, outside the 0.6.x compatibility range; downstream crates staying on 0.6.x
retain the three-variant API.

The surface manifest retains the published baseline and records the previous
InstallHost declaration and its three variant fingerprints in the append-only
retirement ledger. The new declaration and all four variant fingerprints remain
staged until release verification. Their signature hash changes from
`7e5ec52a64cf1383c0abbc7bedfc1d20e9305731b882e957e8a198c77f664007`
to `19403bce7d126e475d839c2827eade8071ebf87c04685b4058c8b8ac384e01a6`.
Method signatures are unchanged. This inventory update does not promote a release
or waive public-surface, lifecycle, baseline, or security-review checks.

Subprocesses use argv and stdin, never a shell. Output and runtime are bounded.
The turn AbortSignal kills an outstanding context child and preserves cancellation.
Preparation errors park the original claimed batch in the DSH inbox before
propagating the error; an explicit later wake retries those identities and sources.
Capture errors remain error-level diagnostics and are consumed once an awaited
checkpoint reports them, allowing later healthy captures and steps to proceed.
Without an explicit profile, the existing runtime host defaults resolve DSH
extraction to the existing codex profile; this does not modify models or credentials.
Tool-call argument JSON is parsed into structured values before capture. Invalid
JSON raises a fixed capture error that omits the argument text and parser message.
Captured payloads go through remem's redaction and governance; subprocess stderr
is not copied into plugin diagnostics (it can contain provider secrets).

## Validation

Typecheck/build and package installation must use published DSH packages. Tests
exercise real Cordis SessionStore events and awaited middleware; a live remem
executable smoke verifies DSH host rows, prompt/tool/assistant capture, Stop and
extraction-task queueing in an isolated store. Memory AI promotion requires the
operator's configured executor and is not implied by offline capture verification.
CI and local preflight install the pinned npm dependencies, resolve the current
remem executable from Cargo's artifact output, and run the entire lifecycle suite
with `REMEM_DSH_BINARY` set, so the real CLI test is required.
