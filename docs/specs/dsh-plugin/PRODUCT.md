# DSH memory plugin

Refs #1111.

Status: Current contract; implementation is in this change.

`@remem-ai/dsh-remem` connects an explicitly installed remem executable to
DeepSeek Harness. It does not install a binary or implement memory retrieval,
extraction, governance, or storage itself.

Before the first step of a turn, it retrieves project memory through `context`.
Live user prompts, assistant messages, and completed tool calls are captured
through the existing CLI, including bounded image/file reference metadata.
Unchanged memory is suppressed only after DSH commits the injected snapshot;
a cancelled preparation does not consume later delivery. Changed or removed memory supersedes old remem snapshots in future model requests while retaining the original event log and all human messages. Prompt capture itself
does not mark undelivered recall as injected. Context preparation also creates no
injection audit or usage credit; committed DSH snapshots are the delivery record. A completed, interrupted, or failed turn queues the
existing `summarize` workflow. Memory AI credentials and executor configuration
remain owned by remem.

The provenance host is `deepseek-harness`. DSH plugin context is never recaptured
as a human prompt. Assistant prose retains message/assistant provenance and remains
external evidence: repetition or unrelated successful tools cannot make an
assistant-only claim eligible for automatic memory promotion. Useful candidates
stay available for review; independently supporting local evidence can satisfy
the existing promotion gates. Historical seed events are not ingested. The plugin does not
claim filesystem transcript import, automatic MCP tools, or host installation via
`remem install`. A DSH adapter preserves arbitrary tool names and results. All DSH tool results
are external evidence because arbitrary registered names do not prove local
provenance, including names matching local tools on other hosts. Tool arguments
cannot change an event's source: a tool named `assistant/message` remains a tool
result, distinct from actual assistant prose even when its evidence is oversized.
JSON tool arguments
are decoded before capture so secret-key redaction applies; malformed arguments
produce a visible safe capture error without retaining their raw text.
Prompts, assistant messages, turn summaries and tool observations exceeding the
CLI stdin budget retain bounded text and structured evidence, including turn-end
reasons and tool presentation metadata. Omitted content is
explicitly marked; retained evidence still passes through core redaction and
capture. Full original messages and results remain in DSH history, not a new adapter store.
Identity fields exceeding the budget fail visibly instead of changing provenance.

Missing executables fail activation. Errors in remem context preparation, its
awaited capture checkpoint or snapshot replacement preserve uncommitted original
inputs for explicit retry. While loaded, `agent/error` also recovers the admitted
batch on later request-preparation failures. If unload interrupts a running remem
`agent/pre-step` or `agent/request` callback, recovery lasts until that callback
exits, even after Cordis removes its listeners. Recovery preserves original
identities, order and sources, excluding synthetic memory, committed messages
and intentionally cancelled batches. Snapshot replacement obeys the published
DSH session invariant and retains committed human prompts.
Cancellation promptly stops context preparation without committing a snapshot.
Every synchronous or background capture failure is logged separately and all
pending failures surface together once through an awaited checkpoint. Cancellation
leaves them for a later checkpoint, after which healthy work can recover. Disposal awaits remem-owned
work, not remaining host request preparation. No silent capture failures or
automatic binary downloads.

Known host limitation: DSH 0.2.0-rc.2 claims input before `agent/pre-step`, then
awaits public `LlmAdapter.prepareCall` after `agent/request` returns and before
committing `user/message`. If remem has unloaded and that call fails, the claimed
input is lost; reload plus explicit retry cannot recover it. The same loss occurs
without ever loading remem. A successful call still commits once after unload.
Capture covers live events committed while remem is loaded; events committed
while unloaded are not imported on reload.
