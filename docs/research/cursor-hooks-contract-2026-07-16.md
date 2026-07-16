# Cursor hooks real-host contract probe (2026-07-16)

Tracking: GH-822; informs GH-823, GH-824, and GH-825.

## Verdict

This probe used Cursor IDE 3.6.31 on macOS and captured one foreground,
single-root session from `sessionStart` through a successful `stop`.

The two implementation-blocking facts established by this run are:

1. `sessionStart.transcript_path` was present but `null`. The matching `stop`
   payload carried a readable JSONL transcript path. The two-line transcript
   contained one `user` record and one `assistant` record, with each record
   shaped as `{role, message}` and text under `message.content[].text`.
2. A unique marker returned by the `sessionStart` hook as top-level
   `additional_context` was visible to the model. The user prompt did not
   contain the marker and asked the model to return the marker already present
   in its initial context; the model replied exactly
   `GH822_CURSOR_SESSIONSTART_MARKER_20260716_A`.

The run did **not** exercise `postToolUse`, failed tools, MCP tools, background
agents, a context-size boundary, or compaction. Those questions remain
unobserved and must stay fail-closed in the draft specs. This report does not
approve GH-823/GH-825 or authorize runtime implementation.

## Environment and method

- Host: Cursor IDE 3.6.31 (`com.todesktop.230313mzl4w4u92`), already running.
- OS: macOS, local user-level hooks.
- Workspace: one local workspace root; the path is intentionally omitted.
- Agent mode: foreground Agent, Auto model selection.
- Probe: an appended user-level hook command captured stdin for selected event
  names into a private `0700` directory with `0600` files. Raw payloads were
  never copied into the repository or printed in this report.
- Injection response: `sessionStart` returned a single JSON object with a
  fixed synthetic `additional_context` marker. A second fixed marker was
  configured for `postToolUse`, but no `postToolUse` event occurred in this
  no-tool turn.
- User config safety: the pre-probe `~/.cursor/hooks.json` was backed up before
  modification. Existing hook entries were retained and the probe entries
  were appended. Cursor recognized the change without an application restart.
  Immediately after `stop`, the original file was restored byte-for-byte;
  its pre- and post-restore SHA-256 was
  `5926f5c7bdd1d82b3779feeaf5d1ceb3ce1a51a9e323c24da6bda9810c2d03f2`.

The captured event order was exactly `sessionStart`, `stop`. The private raw
payload hashes at analysis time were:

| Evidence | SHA-256 |
|---|---|
| `sessionStart` stdin | `e1b7da81fdafc7896c7e039b94a486608bda61f40ac3d6f8c12320da3f509d76` |
| `stop` stdin | `7b1621127cf4ec1691d5b168ee5f453aac8c697f72e1c12013774e6fb0f4f5a0` |
| transcript JSONL | `bc0b8a5cced6faf745cc6359b1e4c9312b21559d451f44ff936a8985aa288a05` |

These hashes attest to the local evidence used for the structural analysis;
they do not make the private payloads public evidence.

## 1. `transcript_path`

### Observed payload behavior

| Event | Field present | Type | File observation |
|---|---:|---|---|
| `sessionStart` | yes | `null` | No transcript location was available. |
| `stop` | yes | string | Existing regular file; JSON data; two valid JSONL rows. |

The `sessionStart` and `stop` payloads had equal 36-character `session_id` and
`conversation_id` values, and the identity remained equal across both events.
This single run therefore supports a strict equality rule when both fields are
present. It does not prove behavior for older Cursor versions or malformed
producer states.

### Sanitized transcript schema

The `stop` transcript was newline-delimited JSON, not SQLite and not one
monolithic JSON array. Each line had only these top-level fields:

```json
{
  "role": "user | assistant",
  "message": {
    "content": [
      {
        "type": "<string>",
        "text": "<redacted>"
      }
    ]
  }
}
```

For this one-turn no-tool session, the final file contained exactly the user
prompt and assistant response. No tool-call record, system record, timestamp,
message id, or token metadata appeared in the transcript rows.

### Not observed

- Real-time append behavior could not be measured because `transcript_path`
  was `null` at `sessionStart` and no intermediate event was captured.
- Completeness is established only for this one-turn, no-tool transcript at
  `stop`; long sessions, tool calls, retries, interrupted generations, and
  compaction remain unverified.
- `postToolUse.transcript_path` was not observed.

### Consequence for GH-825

GH-825 must not require a non-null transcript path at `sessionStart` and must
not route a null value to the existing Claude/Codex reader. A Cursor reader can
be specified against JSONL `{role,message.content[]}` only after additional
fixtures cover tool calls, longer sessions, interrupted/error stops, and
append/boundary behavior. The two-row sample is insufficient to freeze a
complete transcript grammar.

## 2. `additional_context`

### `sessionStart`

Observed end-to-end behavior:

1. The hook emitted
   `{"additional_context":"GH822_CURSOR_SESSIONSTART_MARKER_20260716_A"}`.
2. The user prompt did not contain that marker and prohibited tool/file use.
3. The model replied with exactly the marker.

This proves model visibility for a short `sessionStart.additional_context` in
the tested foreground Cursor 3.6.31 session. Accessibility inspection showed
the marker as the assistant response; no raw payload or screenshot was
committed because adjacent UI contained unrelated local workspace data.

The exact internal placement (system message versus another model-context
segment) is not externally observable from this run. The marker was visible to
the model and was not rendered as a separate user message before the response.

### `postToolUse`

Unobserved. No tool was invoked, so the configured `postToolUse` hook did not
fire. This run provides no evidence that its `additional_context` is equivalent
to `sessionStart`, awaited by the host, or model-visible.

### Size and truncation

Unobserved. Only a short ASCII marker was tested. No numeric maximum, UTF-8
measurement point, exact-limit success, one-byte-over rejection, or truncation
marker can be selected from this evidence. GH-823 must keep
`CURSOR_ADDITIONAL_CONTEXT_MAX_BYTES` and the over-limit policy open.

## 3. Payload schemas

The following tables contain field names and JSON types only. Values that can
identify the account, model, generation, conversation, or filesystem are not
included.

### `sessionStart`

| Field | Observed type / property |
|---|---|
| `conversation_id` | string, length 36 |
| `generation_id` | string |
| `model` | string |
| `is_background_agent` | boolean (`false` in this foreground run) |
| `composer_mode` | string |
| `session_id` | string, length 36; equal to `conversation_id` |
| `hook_event_name` | string; exact observed value `sessionStart` |
| `cursor_version` | string |
| `workspace_roots` | array of one string |
| `user_email` | string; present and therefore must be removed before every remem sink |
| `transcript_path` | null |

No top-level `cwd` field was present. The only observed project-root carrier
was `workspace_roots`.

### `stop`

| Field | Observed type / property |
|---|---|
| `conversation_id` | string, length 36 |
| `generation_id` | string |
| `model` | string |
| `status` | string; exact observed value `completed` |
| `loop_count` | number; observed value `0` |
| `input_tokens` | number |
| `output_tokens` | number |
| `cache_read_tokens` | number |
| `cache_write_tokens` | number |
| `session_id` | string, length 36; equal to `conversation_id` and the start identity |
| `hook_event_name` | string; exact observed value `stop` |
| `cursor_version` | string |
| `workspace_roots` | array of one string |
| `user_email` | string; present |
| `transcript_path` | string |

`is_background_agent` and `composer_mode` were absent from the observed
`stop` payload. No top-level `cwd` field was present.

### Tool payloads and names

Unobserved. The session intentionally used no tools to isolate the injection
test. There is no real-host evidence here for `tool_name`, `tool_input`,
`tool_output`, JSON-string encoding, failure events, a stable tool invocation
id, or mappings to Claude Code `Write`/`Edit`/`Bash`. GH-823 must keep generic
unknown-tool preservation and failed-tool correlation behind another probe.

## 4. Configuration and failure behavior

- A user-level `~/.cursor/hooks.json` change was recognized by the already
  running Cursor application: creating a new Agent session immediately fired
  the newly appended `sessionStart` probe. A full Cursor restart was not needed
  in this run.
- This proves hot recognition for an appended valid hook entry only. It does
  not establish file-watch latency, replacement semantics during an active
  generation, or behavior after invalid JSON.
- Timeout behavior was not exercised. The probe completed normally under a
  ten-second hook timeout.
- Hook failure UI behavior was not exercised. No non-zero exit, timeout, or
  malformed stdout was injected, so no claim about user-visible diagnostics is
  justified.

## 5. `stop` and `preCompact`

The successful turn emitted one `stop` with `status: "completed"` and numeric
`loop_count: 0`. The payload also provided token counts, stable session
identity, one workspace root, and the final transcript path. This is enough to
route a successful Stop only after GH-825 supplies a verified Cursor reader.

The values `aborted` and `error` were not observed. The draft closed set
`completed | aborted | error` is therefore only partially supported and must
not be frozen from this run alone. There is also no evidence here for how
cancelled or failed turns preserve already-captured data.

`preCompact` was registered but did not fire in this short session. This is a
non-observation, not evidence that Cursor lacks the event. No mid-session
summarization semantics or ordering can be inferred.

## Spec decision matrix

| Draft question | Evidence from this run | Decision |
|---|---|---|
| Cross-event identity | Both fields present, equal, and stable across start/stop | Supports strict equality for this version; add more fixtures before freezing compatibility. |
| Single project root | `workspace_roots` was a one-string array on both events; no `cwd` | Use only a strictly validated single root for the observed happy path; multi-root remains blocked. |
| Cursor PII | `user_email` present on both events | B-014 sanitization is mandatory at the outer boundary. |
| Start transcript | field present but null | Null must be accepted at start and must not reach a transcript reader. |
| Stop transcript | JSONL `{role,message}` in a one-turn sample | Reader design may use this as a minimal fixture, not a complete grammar. |
| Start context injection | Unique short marker reached the model | B-012 passes for this foreground 3.6.31 sample. |
| Post-tool context injection | No event | Remains blocked/unverified. |
| Context byte limit | Not tested | No numeric constant or overflow policy may be selected. |
| Stop contract | `completed`, `loop_count: 0` observed | Successful Stop shape is supported; aborted/error remain unverified. |
| Background agent | Not tested | No injection/capture policy selected. |
| Failed tools | Not tested | Capture must remain incomplete or uninstalled until correlation is verified. |
| MCP hooks | Not tested | B-016 remains blocked. |
| `preCompact` | Registered, not emitted in a short session | No equivalent summarize trigger established. |
| Config reload | New valid entry fired without restart | Hot recognition observed; invalid/failure behavior remains open. |

## Required follow-up probe

Before GH-823 or GH-825 can be approved as complete, a second bounded run must
exercise only the remaining gates:

1. one successful shell-equivalent, edit/write-equivalent, read, and unknown
   or MCP tool, capturing `preToolUse`, `postToolUse`, and
   `postToolUseFailure` behavior without retaining file contents;
2. one failed shell-equivalent and one failed edit/write-equivalent to test
   event ordering and a stable per-call identity;
3. one real MCP call plus explicit `beforeMCPExecution` and
   `afterMCPExecution` probes;
4. foreground/background comparison;
5. exact-limit and one-byte-over `additional_context` tests with multibyte
   input after a candidate bound is chosen for measurement, not implementation;
6. a longer session observed before and after a tool call to establish
   transcript append/boundary behavior, followed by cancelled/error Stop;
7. a controlled compaction to establish whether and when `preCompact` fires.

Until those observations exist, implementations must preserve the draft
specs' fail-closed gates instead of filling gaps from Cursor documentation or
event-name analogy.
