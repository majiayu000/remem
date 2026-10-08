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
as a human prompt. Historical seed events are not ingested. The plugin does not
claim filesystem transcript import, automatic MCP tools, or host installation via
`remem install`. A DSH adapter preserves arbitrary tool names and results. JSON tool arguments
are decoded before capture so secret-key redaction applies; malformed arguments
produce a visible safe capture error without retaining their raw text.

Missing executables fail activation. Context failures reject request preparation;
the original claimed prompts remain in the DSH inbox for an explicit retry.
Cancellation promptly stops context preparation without committing a snapshot.
Background capture failures are logged and surface once through an awaited
checkpoint, after which healthy work can recover. No silent loss or automatic binary downloads.
