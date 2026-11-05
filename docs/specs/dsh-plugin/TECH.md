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
Replacements are prepared without mutating history in `agent/pre-step`, then
appended in the awaited `agent/request` waterfall after `step/start`. This obeys
the published session invariant; post-commit `session/event` cannot append them
reentrantly. DSH omits empty developer nodes from derived requests; the append-only
snapshot and human prompt events stay intact. The fresh snapshot still enters through
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
paths. The plugin places the native event type in its own
`tool_input.dsh_event_type` wrapper, outside tool-controlled `arguments`, and
retains it in the fixed envelope when projecting oversized evidence. Classification
uses that source field, never the registered tool name. The synthetic
`assistant/message` observe input is classified as `message`
and persisted with `role=assistant` and no tool name; actual tool results keep
their tool provenance. Both aggregate trust derivation and per-claim evidence
binding pass the existing database host identity to the trust classifier. It treats
all non-human DSH evidence as external content, including assistant prose and
arbitrarily named tools, during live capture and spill replay. Tool names alone
cannot prove local provenance; other hosts retain their existing classifications. No new trust class or
promotion exception is introduced. The context profile reports no MCP
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
Cancellation, timeout and output limits stop the owned process tree, including
remem behind its npm launcher: a dedicated process group on POSIX and the system
`taskkill.exe /T /F` operation on Windows. Tree-termination errors remain visible.
The turn AbortSignal kills an outstanding context child and preserves cancellation.
Plugin disposal aborts and awaits outstanding context children before returning;
preparation waiting on capture cannot start a context child after disposal.
A cancelled preparation stops waiting for capture promptly; the capture itself
continues to the awaited flush, retaining any failure for that checkpoint.
Errors within a running remem `agent/pre-step` callback park the original claimed
batch in the DSH inbox before propagating; an explicit later wake retries those
identities, order and sources. User cancellation still discards its claimed batch.
The `agent/request` callback holds admission state across `await next()` and
recovers uncommitted originals on downstream failure, snapshot replacement failure
or remem lifetime-abort, even after disposal clears the session map and Cordis
removes `agent/error`. Recovery consumes that admission once and
checks committed message IDs in session history, so a commit after listener
removal is not duplicated; synthetic context is never restored. This callback-local
recovery lasts until the callback exits. While remem remains loaded, `agent/error`
also restores uncommitted admitted originals on later preparation failures,
including public `LlmAdapter.prepareCall` failures, before `turn/end` clears admission.
Disposal drains remem-owned work, not arbitrary downstream middleware or host
request preparation.

DSH 0.2.0-rc.2 claims input before `agent/pre-step`. `agent/request` returns
configuration; the host then awaits `LlmAdapter.prepareCall` before committing
`user/message`. Once the callback has returned and Cordis has unloaded remem, no
plugin error listener remains. A later `prepareCall` rejection follows the host's
no-plugin baseline: claimed input is absent from both inbox and committed history,
and remem has not captured it. Reload plus explicit retry cannot reconstruct it.
A successful late call still commits the host-owned accepted batch once after
unload; events committed while unloaded are not imported on reload. The guarantee
that unload prevents a memory snapshot commit applies only to preparation actually
interrupted by remem's lifetime abort. It does not cover later host preparation
after the callback returns. No post-unload host transaction guarantee is provided.

A later retry derives snapshot state from committed history, including any
replacements accepted before a subsequent failure.
Each capture error remains an error-level diagnostic and is retained in the
session queue until an uncancelled awaited checkpoint consumes the batch. A single
failure is thrown unchanged; multiple failures use AggregateError with each cause
and its safe message. Later healthy captures and steps can then proceed.
Without an explicit profile, the existing runtime host defaults resolve DSH
extraction to the existing codex profile; this does not modify models or credentials.
Tool-call argument JSON is parsed into structured values before capture. Invalid
JSON raises a fixed capture error that omits the argument text and parser message.
Captured payloads go through remem's redaction and governance; subprocess stderr
is not copied into plugin diagnostics (it can contain provider secrets).
Before capture, the adapter measures the complete serialized envelope
against the unchanged 1 MiB core stdin boundary. Envelopes that fit are unchanged.
For oversized tool and assistant observations it reserves the identity/omission envelope, then budgets
one quarter of the remaining JSON bytes to structured arguments and three quarters
to the result event (including presentation metadata). Nested objects keep their
original keys; collections retain a bounded prefix and oversized strings retain
only complete leading lines with an omission marker. An oversized single line is
omitted whole, preventing partial credentials from losing their redaction context.
`tool_input.remem_capture` records the original envelope/argument/result JSON byte
counts and explicitly declares omitted entries/lines. An identity envelope that
alone exhausts the bound returns a visible error. No secret policy is implemented
in TypeScript; retained values go through the existing core redaction, capture
ledger and event-blob path. Oversized user prompts and Stop payloads use the same
projection with three quarters of the remaining budget for message text and one
quarter for structured details (including the turn-end reason). Their omission
metadata is embedded in `prompt` or `last_assistant_message`, the actual fields
persisted by core capture, and records original envelope/message/details JSON
byte counts. This also bounds the assistant answer forwarded again at turn end.
The original DSH message and result history is unchanged.

## Validation

Typecheck/build and package installation must use published DSH packages. Tests
enable the published invariant registry and official session companion for real
Cordis SessionStore events and awaited middleware, including changed/removed
snapshots, request preparation failures, replacement failures and cancellation; a live remem
executable smoke verifies DSH host rows, prompt/tool/assistant capture, Stop and
extraction-task queueing in an isolated store. A deterministic offline executor
returns an assistant-only decision through the real rollup worker, proving that
the candidate remains pending and no active memory is created. A Rust capture/
summary regression additionally checks repeated claims, unrelated tool success,
and independently corroborating tool evidence. This proves governance behavior,
not live-provider extraction quality.
Regressions also unload during pending request preparation (including downstream
failure, partial commitment and intentional cancellation) and exercise 2 MiB
presentation metadata, meaningful oversized arguments/results and escaped wide
collections through the real tool host. Real remem checks the retained evidence,
omission metadata, core redaction and Stop capture in an isolated store. Large
messages cover UTF-8 and JSON escaping, oversized single lines, exact-limit and
one-byte-over envelopes, and large turn-end metadata. Trust regressions cover
arbitrary names and local-tool name collisions in both classification paths;
DSH-only corroboration cannot promote a claim, while genuine local evidence from
another host retains its existing eligibility.
CI and local preflight install the pinned npm dependencies, resolve the current
remem executable from Cargo's artifact output, and run the entire lifecycle suite
with `REMEM_DSH_BINARY` set, so the real CLI test is required.
