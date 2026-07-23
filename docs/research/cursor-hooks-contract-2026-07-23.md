# Cursor hooks real-host contract probe v2 (2026-07-23)

Tracking: GH-822; informs GH-823, GH-824, and GH-825.

## Verdict

This probe used Cursor IDE 3.12.17 on macOS and exercised two foreground
sessions plus one read-only multitask subagent. It closes the main real-host
gaps left by the 3.6.31 v1 probe:

- `sessionStart.transcript_path` is still `null`; foreground prompt, tool,
  response, compaction, and Stop events use one stable JSONL path per parent
  session.
- The JSONL transcript is appended during the turn. It now contains
  `{role,message}` rows, `tool_use` content blocks, and standalone
  `turn_ended` rows. A cancelled turn is represented as
  `{type:"turn_ended",status:"error",error:"User aborted request"}`.
- Built-in tool names observed at the generic tool hooks were `Read`, `Shell`,
  `Task`, and `MCP:browser_tabs`. The missing-file path produced
  `postToolUseFailure` with `failure_type:"error"` and
  `is_interrupt:false`.
- A short `postToolUse.additional_context` marker was model-visible. In
  contrast, a short `sessionStart.additional_context` marker was not visible
  in 3.12.17 even though the hook fired and exited successfully. This differs
  from the successful 3.6.31 v1 observation.
- A normal manual `/summarize` operation emitted `preCompact` with
  `trigger:"manual"` and explicit context/window/message counts.
- Cancelling generation emitted `stop.status:"aborted"` and omitted all four
  token-count fields. Completed Stops included those fields.
- User-level config changes were recognized without restart. A non-zero hook
  and a timed-out hook did not block the prompt and produced no visible error
  in the inspected UI.

The sanitized evidence bundle is in
[`fixtures/cursor-hooks-contract-2026-07-23/`](fixtures/cursor-hooks-contract-2026-07-23/).
This report does not authorize runtime implementation. GH-823 and GH-825 must
consume the version-sensitive and null-path behavior explicitly.

## Environment and method

- Host: Cursor IDE 3.12.17 (`com.todesktop.230313mzl4w4u92`).
- OS: macOS; user-level `~/.cursor/hooks.json`.
- Workspace: one clean detached worktree at an exact `origin/main` commit; the
  local path is intentionally omitted.
- Agent mode: foreground Agent on This Mac; one `explore` multitask subagent.
- Probe storage: private `0700` root and run/backup directories, `0600` raw
  payloads, no repository writes by Cursor.
- Safe actions: README reads, one nonexistent synthetic path, `pwd`,
  `git status --short`, one list-only browser MCP call, one read-only
  subagent, one cancelled text generation, and `/summarize`.
- Config safety: existing user hooks were preserved in order and probe entries
  appended. After the final event, the original config was restored
  byte-for-byte. The private pre/post SHA-256 comparison succeeded; mode,
  owner, and group were also restored.
- Privacy: raw account, path, model, identifier, transcript, and token values
  were never copied into the repository. Published fixtures preserve keys,
  JSON types, safe event/tool/status enums, and representative structure with
  replacement values.

The first no-tool turn proved hot recognition: probe entries added while
Cursor was already running fired on the next prompt. A full application quit
did not emit `sessionEnd`, and relaunch alone did not emit `sessionStart`.
Creating and submitting the first prompt in a genuinely new Agent did emit
`sessionStart`.

## Event coverage

| Event | Count | Key result |
|---|---:|---|
| `sessionStart` | 1 | New Agent first prompt only; transcript path `null`. |
| `beforeSubmitPrompt` | 8 | Hot-loaded; first new-session prompt path `null`, later prompts string. |
| `preToolUse` | 6 | `Read`, `Shell`, `Task`, `MCP:browser_tabs`. |
| `postToolUse` | 4 | Successful Read/Shell/MCP; marker model-visible after Read. |
| `postToolUseFailure` | 1 | Missing-file Read; correlated by `tool_use_id`. |
| `beforeReadFile` | 2 | Successful parent and subagent reads. |
| `beforeShellExecution` / `afterShellExecution` | 1 each | Read-only command; sandbox boolean present. |
| `beforeMCPExecution` / `afterMCPExecution` | 1 each | `cursor-ide-browser.browser_tabs`; string-encoded input/result. |
| `subagentStart` / `subagentStop` | 1 each | `explore`, non-parallel, completed. |
| `preCompact` | 1 | Manual summarize. |
| `afterAgentThought` | 28 | Text and duration fields. |
| `afterAgentResponse` | 7 | Text plus token-count fields. |
| `stop` | 9 | Eight completed and one aborted. |
| `sessionEnd` | 0 | Not emitted by normal full-app quit. |

The complete key/type shapes are published in
[`payload-shapes.synthetic.json`](fixtures/cursor-hooks-contract-2026-07-23/payload-shapes.synthetic.json).

## 1. `transcript_path`

### Event behavior

| Context | Observed type |
|---|---|
| Foreground `sessionStart` | `null` |
| First foreground prompt/thought events | `null` until the transcript exists |
| Later foreground prompt/tool/response/Stop events | string |
| Parent `subagentStart` / `subagentStop` | string |
| Inner subagent Read events | `null` |
| `subagentStop.agent_transcript_path` | `null` |

Every observed payload had equal `session_id` and `conversation_id`. Within
each foreground session, all non-null transcript paths were equal. The
subagent's inner Read events used a distinct session identity and a null
transcript path, so a consumer must not assume every tool event is attached to
the parent transcript.

### Live append evidence

The first foreground session used one transcript path throughout. Size/line
samples taken by the hook show:

| Boundary | Bytes | JSONL rows |
|---|---:|---:|
| Initial completed Stop | 523 | 3 |
| Next completed response | 890 | 6 |
| Before the two Read calls | 890 | 6 |
| After the Read failure/success pair | 1,337 | 7 |
| Final response and Stop | 1,942 | 10 |

`postToolUseFailure`, `beforeReadFile`, and `postToolUse` all saw the
seven-row snapshot. The final response and Stop both saw the ten-row snapshot.
This establishes mid-session append behavior and Stop completeness for the
tested turns.

### JSONL grammar

Observed records were newline-delimited JSON. Three representative shapes
were present:

```json
{"role":"user","message":{"content":[{"type":"text","text":"<redacted>"}]}}
{"role":"assistant","message":{"content":[{"type":"text","text":"<redacted>"},{"type":"tool_use","name":"Read","input":{"file_path":"<redacted>"}}]}}
{"type":"turn_ended","status":"success"}
```

The cancelled turn added:

```json
{"type":"turn_ended","status":"error","error":"User aborted request"}
```

No per-message id, timestamp, sequence, or token metadata was observed.
Tool-use blocks appeared inside assistant content. No separate `tool_result`
row was observed in these transcripts. File order is therefore the only
observed record order.

### Consequence for GH-825

- Accept a null start path and null inner-subagent tool paths.
- Bind capture to a stable, validated Stop path rather than assuming the path
  exists at session start.
- Parse both `{role,message}` and `turn_ended` records.
- Preserve unknown content-block and record types fail-closed; this probe does
  not freeze the complete producer grammar.
- Do not invent message identity. Use snapshot file order and the existing raw
  occurrence ordinal.

## 2. `additional_context`

### `sessionStart`

The hook returned the short top-level object:

```json
{"additional_context":"GH822_SMALL_MULTIBYTE_SUFFIX_20260716"}
```

The first user prompt did not contain the marker, prohibited tools/file reads,
and asked for the initial-context marker. The model returned
`NOINITIALMARKER`. The `sessionStart` payload was captured successfully, and
no hook failure was visible.

This means short `sessionStart.additional_context` was not model-visible in
the tested Cursor 3.12.17 Agent surface. The 3.6.31 v1 probe observed the
opposite. The contract is version-sensitive and cannot treat session-start
injection as a currently reliable capability.

Because the smallest bounded case failed, medium and large cases were not run.
There is no useful length threshold to measure until short injection works
again. GH-823 must not select a numeric max from these probes.

### `postToolUse`

After a successful `Read`, the hook returned:

```json
{"additional_context":"GH822_POSTTOOLUSE_CONTEXT_20260716"}
```

The user prompt asked the model to append any hidden post-tool marker. The
final response included the exact marker. It was not shown as a separate user
message. This establishes model visibility for short post-tool context in
3.12.17, but not an exact internal system/user placement.

### Consequence for GH-823

`sessionStart` and `postToolUse` injection are not equivalent in current
Cursor. An adapter must capability-gate them separately. The current
session-start path must fail closed or report unsupported rather than silently
claiming that memory was injected.

## 3. Payload schemas and tool names

Common outer fields on observed events were:

- `conversation_id`, `generation_id`, `model`, `session_id`,
  `hook_event_name`, `cursor_version`, `workspace_roots`, `user_email`;
- `transcript_path`, whose type depends on event/session context.

`sessionStart`, prompt/response/Stop events also carried `model_id` and
`model_params`; generic tool events did not. `cwd` appeared on Shell generic
tool events and shell-specific events, but not on Read or lifecycle events.
`is_background_agent` appeared only on the foreground `sessionStart` and was
`false`.

### Generic tool hooks

| Tool | Success/failure observation | Notable fields |
|---|---|---|
| `Read` | success and failure | `tool_input.file_path`; successful `tool_output` string; failure fields below |
| `Shell` | success | `tool_input`; `cwd`; successful `tool_output` string |
| `Task` | start observed | parent subagent launch |
| `MCP:browser_tabs` | success | generic wrapper around MCP-specific hooks |

The failed Read payload included:

- `error_message: string`
- `failure_type: "error"`
- `duration: number`
- `tool_use_id: string`
- `is_interrupt: false`

Both pre/success/failure payloads used the same per-call `tool_use_id` for the
corresponding call. The probe did not perform Write/Edit/Delete because GH-822
forbids product mutations; their names remain unobserved.

### MCP-specific hooks

The list-only call emitted:

- `mcp_server_name: "cursor-ide-browser"`
- `tool_name: "browser_tabs"`
- `tool_input: string`
- `command: string` on `beforeMCPExecution`
- `result_json: string` and `duration: number` on `afterMCPExecution`

Consumers must not assume MCP input/result arrive as parsed JSON objects.

### Subagent hooks

`subagentStart` carried `subagent_id`, `subagent_type:"explore"`, `task`,
`parent_conversation_id`, `tool_call_id`, `subagent_model`, and
`is_parallel_worker:false`.

`subagentStop` added `status:"completed"`, `duration_ms`, `message_count`,
`tool_call_count`, `loop_count`, `description`, and
`agent_transcript_path:null`.

## 4. Configuration, timeout, and failure behavior

### Hot recognition

Appending valid user-level hook entries while Cursor was running caused the
very next prompt to emit the newly added prompt/thought/response/Stop events.
No restart was required. The same behavior applied to the temporary failure
entries.

### Lifecycle boundary

- Full application quit: no `sessionEnd`.
- Relaunch with the Agents home visible: no `sessionStart`.
- First prompt in a genuinely new Agent: `sessionStart` immediately before the
  prompt event.

### Non-zero exit and timeout

Two temporary `beforeSubmitPrompt` entries were appended:

```json
{"command":"/bin/sh -c 'exit 42'","timeout":10}
{"command":"/bin/sh -c 'sleep 2'","timeout":1}
```

The prompt continued and the model returned the requested response. No
user-visible error, toast, inline failure, or blocked state appeared in
accessibility inspection during or after the turn. The entries did not use
`failClosed`; this establishes default fail-open behavior only.

Invalid JSON, malformed hook stdout, and explicit `failClosed:true` were not
tested because they could interfere with the user's active configuration.

## 5. Stop and `preCompact`

### Completed versus cancelled Stop

Completed Stops had:

- `status:"completed"`
- numeric `loop_count` (observed `0`)
- numeric input/output/cache-read/cache-write token fields
- string transcript path

The cancelled text generation had:

- `status:"aborted"`
- numeric `loop_count` (observed `0`)
- no input/output/cache token fields
- string transcript path

The corresponding transcript ended with a `turn_ended` error row. A Stop with
`status:"error"` was not observed.

### Manual compaction

Cursor exposes `/summarize`, not `/compact`, in the tested UI. Running it
produced `preCompact` before summarization with:

- `trigger:"manual"`
- numeric `context_usage_percent`, `context_tokens`, and
  `context_window_size`
- numeric `message_count` and `messages_to_compact`
- `is_first_compaction:true`
- string transcript path

The UI then reported `Chat context summarized`, and displayed context usage
dropped from 13% to 10%. The transcript remained readable afterward.

## Spec decision matrix

| Draft question | Decision from v2 evidence |
|---|---|
| Cross-event identity | Require equal `session_id`/`conversation_id` when both are present; correlate tool calls by `tool_use_id`. |
| Start transcript | Accept `null`; never send it to a reader. |
| Parent Stop transcript | String JSONL path; validate and snapshot at Stop. |
| Subagent transcript | Inner tool events and `agent_transcript_path` may be null; do not silently substitute the parent path. |
| Transcript grammar | Support role/message content plus `turn_ended`; preserve unknown records fail-closed. |
| Start context injection | Unsupported/unreliable on 3.12.17 despite v1 success on 3.6.31. |
| Post-tool injection | Short context is model-visible on 3.12.17. |
| Context byte limit | No numeric limit may be selected because the smallest start case failed. |
| Built-in tool matching | Exact observed names: `Read`, `Shell`, `Task`, `MCP:browser_tabs`; Write/Edit remain unobserved. |
| Failed tools | `postToolUseFailure` exists and correlates by `tool_use_id`. |
| MCP | Input/result are strings; use `mcp_server_name` plus `tool_name`. |
| Background/subagent | Explicit start/stop events exist; subagent transcript paths may be null. |
| Cancel | `stop.status:"aborted"`; token fields absent; transcript error row present. |
| Compaction | Manual `/summarize` emits `preCompact` with quantitative context fields. |
| Config reload | Valid changes hot-load. Default hook exit/timeout behavior is silent fail-open. |

## Remaining bounded unknowns

The following are not blockers for documenting the observed host contract, but
must remain explicit implementation gates where relevant:

1. Write/Edit/Delete tool names and payloads were not exercised because the
   PoC stayed read-only.
2. No `stop.status:"error"` sample exists.
3. `sessionEnd`, `agent_transcript_path:string`, multi-root, and a true
   background/cloud agent were not observed.
4. Session-start context size limits cannot be measured while the smallest
   case is ignored.
5. `failClosed:true`, invalid config JSON, and malformed stdout behavior remain
   untested.

These are bounded compatibility cases, not reasons to retain the old
assumptions about transcript format, event names, or injection equivalence.
