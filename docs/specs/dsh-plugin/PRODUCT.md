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
a cancelled preparation does not consume later delivery. Prompt capture itself
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
background capture failures are logged and surface through the awaited session
flush checkpoint. No silent loss or automatic binary downloads.
