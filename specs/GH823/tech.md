# Cursor Hook I/O Protocol Technical Spec

Status: Draft, needs human approval before implementation
Date: 2026-07-23

Tracking:
- Spec/tracking issue: #823
- Product spec: specs/GH823/product.md
- Epic: #821 · Prerequisite PoC: #822
- Evidence: PR #914 exact head
  `c0802c42c3fc22770aecb0b7b2eec88f117f795c` (Cursor 3.12.17), merged;
  adoption by this packet remains pending fresh exact-head human approval
- Readiness: blocked on human evidence/spec approval and remaining bounded gates; Cursor
  summarize is additionally blocked on #825's verified transcript reader.

## Planned Changes Manifest

<!-- specrail-planned-changes
{
  "version": 1,
  "issue": 823,
  "complete": true,
  "paths": [
    "README.md",
    "docs/ARCHITECTURE.md",
    "docs/specs/README.md",
    "specs/GH823/product.md",
    "specs/GH823/tech.md",
    "specs/GH823/tasks.md",
    "src/adapter/common.rs",
    "src/adapter/mod.rs",
    "src/adapter/redaction.rs",
    "src/cli/dispatch.rs",
    "src/cli/types.rs",
    "src/context.rs",
    "src/context/host.rs",
    "src/context/invocation.rs",
    "src/context/render.rs",
    "src/context/tests/cursor_hook.rs",
    "src/context/tests/mod.rs",
    "src/cursor_hook.rs",
    "src/cursor_hook/identity.rs",
    "src/cursor_hook/input.rs",
    "src/cursor_hook/tests.rs",
    "src/db/capture.rs",
    "src/db/capture/tests.rs",
    "src/db/capture_drop.rs",
    "src/hook_stdin.rs",
    "src/identity.rs",
    "src/lib.rs",
    "src/observe.rs",
    "src/observe/cursor.rs",
    "src/observe/hook.rs",
    "src/observe/session_init.rs",
    "src/observe/spill.rs",
    "src/observe/spill/tests.rs",
    "src/observe/tests.rs",
    "src/runtime_config.rs",
    "src/summarize.rs",
    "src/summarize/input.rs",
    "src/summarize/summary_job/hook.rs",
    "src/summarize/summary_job/hook/tests.rs",
    "src/summarize/summary_job/host.rs",
    "src/summarize/summary_job/replay.rs",
    "src/summarize/summary_job/spill.rs",
    "src/summarize/summary_job/spill/tests.rs",
    "tests/cursor_hooks.rs"
  ],
  "spec_refs": [
    "specs/GH823/product.md",
    "specs/GH823/tech.md"
  ]
}
-->

The shared Cursor boundary is deliberately split into `src/cursor_hook/`
modules instead of extending the already broad command-specific parsers. The
command entrypoints retain only dispatch and adaptation. If implementation
requires a path outside this complete manifest, the packet must be amended and
receive fresh exact-head human approval before that path is changed.

## Existing Implementation Facts

Verified against `origin/main` (`f612b4a1`), 2026-07-15:

- `src/identity.rs:18` defines `pub enum InstallHost { ClaudeCode, CodexCli }`
  with `as_db_value()` returning `"claude-code"` / `"codex-cli"`
  (`src/identity.rs:29-30`) and `parse()` rejecting other values with an error
  that enumerates the valid set (`src/identity.rs:39-42`).
- `src/context/host.rs:4` defines `pub enum HostKind`; per-host behavior is
  dispatched through `ContextHostProfile` implementations
  (`ClaudeCodeContextProfile`, `CodexCliContextProfile`,
  `UnknownContextProfile`, `src/context/host.rs:47-49`) selected by
  `resolve_profile()` (`src/context/host.rs:135`).
- `src/context/invocation.rs:15` carries `transcript_path: Option<String>`;
  `parse_hook_input()` (`src/context/invocation.rs:154`) parses hook stdin,
  with an existing Codex-shaped test fixture at
  `src/context/invocation.rs:198-212`.
- `src/context/render.rs:765` `context_stdout_for_invocation()` wraps context
  output as `{"hookSpecificOutput": {"hookEventName": "SessionStart",
  "additionalContext": ...}}` only when `is_codex_session_start_hook()`
  matches (`src/context/render.rs:783`); all other hosts get plain stdout.
- `src/observe/hook.rs` consumes observe events with `tool_name`; file-edit
  extraction is gated on `matches!(event.tool_name.as_str(), "Write" | "Edit")`
  (`src/observe/hook.rs:190`).
- `src/summarize/input.rs:6-12` carries optional `session_id`, cwd, and
  transcript fields into summarize. `src/summarize/summary_job/hook.rs:34-45`
  currently logs malformed JSON as a warning and returns success, and returns
  before enqueueing when `session_id` is absent. The transcript reader itself is
  `src/memory/raw_transcript.rs:13 read_transcript_content()` (owned by #825).
- `src/runtime_config.rs:170-176 normalize_host()` currently passes arbitrary
  host strings through, while `src/context/host.rs:117-140` can resolve an
  explicit unsupported value as `HostKind::Unknown`. Neither behavior is a
  sufficient closed-set boundary for Cursor hook commands or persistence.
- Install-side hook command lines pass the host explicitly:
  `hook_command()` renders `<bin> <subcommand> --host <runtime_host>`
  (`src/install/config.rs:51-58`), with `runtime_host()` returning
  `"claude-code"` / `"codex-cli"` (`src/install/config.rs:43-48`). The install
  side is #824's surface; listed here because the `--host cursor` value this
  spec introduces is what #824 will write into `hooks.json`.

The official [Cursor Hooks documentation](https://cursor.com/docs/hooks)
(rechecked 2026-07-15) remains schema context, not runtime proof. PR #914 now
provides sanitized installed-host payload/type evidence for Cursor 3.12.17,
including equal `session_id`/`conversation_id`, context-dependent null/string
transcript paths, generic and MCP tool events, failed Read, manual
`preCompact`, and completed/aborted Stops. It also proves capability drift:
short post-tool context was model-visible, while short session-start context
was not. Platform path forms, multi-root/background behavior, numeric limits,
Write/Edit/Delete failures, and `status:error` remain unproved.

## Proposed Design

### 1. Host identity (B-001, B-011)

- `src/identity.rs`: add `InstallHost::Cursor` with db/env value `"cursor"`;
  extend `parse()` and its error message's valid-value list.
- Introduce one exact hook-host parser shared by the `context`, `session-init`,
  `observe`, and `summarize` CLI boundaries and by hook persistence entrypoints.
  Its recognized values are exactly `claude-code`, `codex-cli`, and `cursor`.
  `runtime_config::normalize_host()` and `HostKind::Unknown` are not validation
  boundaries; aliases and arbitrary strings must fail before rendering,
  adapter dispatch, config creation, enqueueing, or database writes.
- Recognition is distinct from command support: after exact host parsing,
  dispatch rejects `session-init --host cursor` as unsupported before prompt
  capture, context generation, stdout, or any other side effect.
- `src/context/host.rs`: add `HostKind::Cursor` and a
  `CursorContextProfile`; `resolve_profile()` dispatches it. Profile policy
  (gating, budget) must not inherit `ClaudeCodeContextProfile`: PR #914 proves
  the session-start capability is blocked on 3.12.17 while post-tool context is
  separately proven. A version/capability matrix is required.

### 2. Stdin parsing (B-002, B-007, B-008, B-009)

- `src/context/invocation.rs`: represent stdin as an explicit no-input, valid,
  or parse-error result instead of collapsing malformed input into `None`.
  Under `--host cursor`, invalid JSON, read failure, missing required fields,
  or wrong field types return an error to CLI dispatch. Context generation is
  not called, the process exits non-zero, stdout remains empty, and no current
  cwd/CLI fallback is permitted.
- Before any Cursor hook entrypoint allocates a `String`, calls serde, or
  retains a payload preview, read stdin through a bounded byte reader with the
  proposed human-frozen
  `CURSOR_HOOK_STDIN_MAX_BYTES: usize = 1_048_576`. The implementation reads at
  most `CURSOR_HOOK_STDIN_MAX_BYTES + 1` bytes (for example through
  `Read::take` into a byte buffer), rejects the one-byte-over sentinel before
  UTF-8 conversion, and only then converts/parses the exact-limit-or-smaller
  buffer. This whole-payload limit is distinct from
  `CURSOR_TOOL_FIELD_MAX_BYTES`. The error records only the configured bound
  and a generated correlation id; it never contains raw bytes or a payload
  preview. The shared bounded reader is used by Cursor `context`, `observe`,
  and `summarize`; unsupported `session-init --host cursor` exits at dispatch
  before reading stdin. Tests prove exact-limit success reaches normal
  validation and one-byte-over returns non-zero, empty stdout, and zero
  persistence, enqueue, spill, adapter, or LLM calls for every entrypoint.
- The Cursor input model requires `hook_event_name` and validates it before
  dispatch: `context` structurally accepts only exact `sessionStart`,
  `observe` accepts human-approved exact `postToolUse` and
  `postToolUseFailure`; when B-016 selects MCP-specific ownership, `observe`
  additionally accepts exact `afterMCPExecution` with its observed field
  schema. `beforeMCPExecution` remains unregistered and unsupported because it
  has no terminal result. When B-016 selects generic ownership, both specific
  events remain unregistered and unsupported. `summarize` accepts only exact `stop`. PR #914
  provides the Read failure shape and matching `tool_use_id`; no other failure
  event is accepted by analogy. Unknown values and event/command mismatches return non-zero before
  adapter or renderer selection; they never fall through to plain-text or
  Claude-shaped output.
- Cursor `sessionStart` requires a non-empty `session_id` and a
  `workspace_roots` array whose total length is exactly one and whose sole
  string is non-empty after trimming. Map only that trimmed
  `workspace_roots[0]` to invocation `cwd`, then derive project identity from
  it. `[]`, `[""]`, `["", "/repo"]`, `["/repo", ""]`, and two-non-empty-root
  arrays all fail closed. After structural validation, pass the trimmed sole
  root through one platform-aware normalizer before canonical path, git-root,
  or project derivation. The accepted Windows `/c:/...` conversion is frozen
  only from a sanitized #822 real-host fixture; unverified shapes fail closed
  and raw path identity is never persisted. Do not filter blank entries before
  validating length, silently select a root, use the hook process cwd, or use
  an unverified `CURSOR_PROJECT_DIR`. The base `transcript_path` remains
  null-tolerant. PR #914 additionally requires null tolerance on the first
  prompt/tool events and inner subagent tool events; never substitute the
  parent transcript path.
- Treat parent-session `sessionStart.session_id`, parent tool-event identity,
  and `stop.conversation_id` as one canonical-session contract, not unrelated
  strings. Separately require exact `session_id == conversation_id` equality
  within every event that carries both fields. PR #914 also observed inner
  subagent tool events with a distinct internally equal identity and a null
  transcript path. Preserve that child identity as its own canonical event
  identity; do not compare or coerce it to the parent, and do not manufacture a
  parent-child link without a verified producer field. Missing, blank,
  wrong-typed, or event-local mismatched identity returns non-zero before
  rendering, adapter dispatch, transcript access, enqueue, spill, or database
  writes. No event-local alias or fallback may manufacture continuity.
- `src/observe/hook.rs`: accept the Cursor `postToolUse` shape only after #822
  verifies the exact identity field types. Before adapter dispatch or capture,
  map required non-empty `conversation_id` to the canonical `session_id` and
  apply the same total single-root validation plus platform normalization used
  for `sessionStart`; the verified normalized root becomes cwd/project. Missing
  or wrong-typed identity, `[]`, `[""]`, mixed-empty, multi-root, or unverified
  platform shapes return non-zero with zero writes. In the observed generic
  event, `tool_input` is an object and successful `tool_output` is a string;
  MCP-specific `tool_input`/`result_json` are strings. Validate the exact
  variant before any classification or filter; decode string-encoded MCP fields
  once, measure raw generic strings (including successful `tool_output`) as
  their exact UTF-8 bytes, serialize object/decoded JSON canonically, and
  validate every applicable representation against one numeric
  `CURSOR_TOOL_FIELD_MAX_BYTES` frozen by the #822 evidence and human approval.
  Invalid nested JSON or any over-limit raw/encoded/canonical representation
  returns non-zero before capture, spill, adapter dispatch, or raw-data
  diagnostics.
  Tool-name mapping lives in one place; unknown names bypass known-tool
  classification but still use the existing generic capture path with the
  original `tool_name` and decoded input/output. They are never remapped or
  discarded after diagnosis (B-007, B-015).
- The B-016 ownership decision controls a closed event-variant set, not only a
  dedup flag. MCP-specific ownership adds exact `afterMCPExecution` parsing
  with string `tool_input`/`result_json`, leaves `beforeMCPExecution`
  unregistered, and makes generic MCP `postToolUse` a successful zero-write
  event. Generic ownership accepts the proven generic post-tool path and keeps
  both specific events unregistered. Either route maps to one canonical
  call/upsert key and dual delivery never writes twice.
- PR #914 proves one failed Read emits `postToolUseFailure` with
  `failure_type:"error"`, `is_interrupt:false`, and the same `tool_use_id` as
  its pre-tool event. Write/Edit/Delete and failed Shell remain unobserved.
  Human approval must freeze `tool_use_id` as the canonical `event_id`/upsert
  key for approved variants and the failure-
  precedence semantics, then either add the verified failure discriminator to
  the Cursor parser and map it
  into a new explicit canonical failure outcome/discriminator carried through
  canonical capture, spill/replay, and database persistence, then consume that
  outcome in downstream classification/extraction wherever success and failure
  differ. Current `ParsedHookEvent`/capture persistence has no general failure
  field, so the implementation must not infer this capability from `exit_code`
  or the event name. Otherwise mark Cursor observe incomplete and keep its #824
  installation entries disabled. The implementation cannot ship a success-only
  observe hook as complete capture. If #822 cannot prove a safe shared call
  identity for dual-event delivery, do not synthesize correlation from mutable
  payload content: the disabled/incomplete branch is required. Tests for an
  approved mapping must assert
  the stored failure outcome for the observed failed Read, prove both-event
  delivery persists exactly once with failure precedence, cover spill/replay
  and downstream consumption, and prove malformed failure payloads produce zero
  writes. Unobserved Bash and edit/write failures may only have disabled-path
  or generic-capture assertions; they must not be assigned a stored failure
  outcome until real-host evidence and human approval freeze their variants.
- `src/summarize`: accept the Cursor `stop` shape only after #822 verifies its
  exact fields. Map required non-empty `conversation_id` to
  `SummarizeInput.session_id` before the existing missing-session early return,
  enqueue, spill identity, or persistence paths. PR #914 records
  `workspace_roots` and equal identity fields on Stop. Apply the same total
  single-root validation and platform-aware normalization before project
  derivation. Until that evidence exists,
  or if the verified field is absent, blank, multi-root, or ambiguous, return
  non-zero before any enqueue, spill, or database write. After verification,
  map only one validated root/cwd to remem project identity; never fall back to
  the process cwd or `CURSOR_PROJECT_DIR`. #825 is a separate hard prerequisite:
  until its Cursor reader is merged and approved, discard/defer Cursor
  `transcript_path` inside the Cursor-specific parser and return explicit
  unsupported/non-zero before calling `read_transcript_content()`, enqueueing,
  spilling, or invoking an LLM. A Cursor path must never be passed to the
  existing Claude/Codex parser merely because it is a string. Require `status`
  to be a string in the human-approved observed set. PR #914 proves
  `completed` and `aborted`; `error` remains unobserved. Require numeric
  `loop_count` (observed `0`) and build the proposed canonical Stop key from
  `(session_id, generation_id, loop_count)` only after identity equality and
  after validating `generation_id` as a required non-empty string. Validate
  every key component before transcript-reader selection, enqueue, spill,
  persistence, or LLM dispatch; missing, blank, or wrong-typed
  `generation_id`, plus missing/blank/wrong-typed/unapproved status or loop
  values, returns non-zero with zero downstream calls. After #825 lands,
  decision (B-008):
  `aborted`/any later-approved `error` still preserve already captured events;
  the LLM summary call
  runs for `completed` and `aborted`, and is skipped with an error-level log for
  `error`. This decision remains subject to the real payload evidence.
- Apply the same explicit error result to Cursor `postToolUse` and `stop`:
  malformed or contract-incomplete input returns non-zero, emits no stdout, and
  performs no partial adapter dispatch, enqueue, spill, or database write.

### 2a. Cursor privacy boundary (B-014)

- Add one Cursor-specific payload sanitization boundary immediately after the
  outer JSON object is recognized and before canonical event construction or
  any raw-payload preview. Drop `user_email` and any other #822-observed direct
  user PII fields; do not copy them into a generic extras map. Parse errors log
  only field names/types and a generated correlation id, never the raw Cursor
  object.
- Downstream capture, database, spill, adapter, and summary code accepts only
  the sanitized canonical event. No unsanitized payload reference or clone may
  cross that boundary. This ordering makes database-open failure safe: the
  spill record is built from the already sanitized event.
- Tests place a unique email sentinel in every valid and malformed Cursor event
  fixture, force normal capture, capture persistence failure, spill replay,
  adapter/LLM invocation, and parse failure, then scan database fields, spill
  bytes (after test-only decode where encryption is enabled), logs, adapter
  request bodies, prompts, and produced summaries for sentinel absence.

### 3. Stdout rendering (B-003, B-004, B-005)

- `src/context/render.rs`: alongside `is_codex_session_start_hook()`, add the
  Cursor branch in `context_stdout_for_invocation()`: only when the strict
  parser has validated `HostKind::Cursor` plus exact
  `hook_event_name: "sessionStart"`, emit
  `{"additional_context": "<ansi-stripped body>"}` and nothing else. Empty
  context body → empty stdout (existing early return already does this).
  Unknown or command-mismatched events never reach this function and cannot
  receive plain-text or Claude-shaped fallback output.
- The rendered body is the existing host-independent context; no new
  instruction text is added (B-004). Add a regression test asserting the
  Cursor payload contains none of the GH668 instruction markers.
- PR #914 proves the smallest session-start marker is not model-visible on
  Cursor 3.12.17, so this renderer is structurally testable but the capability
  is disabled/uninstalled for that version. It separately proves a short
  post-tool marker is model-visible; a later human-approved post-tool output
  contract may enable it without promoting session-start. Once a chosen
  capability works at the smallest bounded size, evidence and human approval freeze a
  numeric `CURSOR_ADDITIONAL_CONTEXT_MAX_BYTES`, the exact point at which UTF-8
  bytes are measured, and one over-limit behavior. Exactly-at-limit output must
  serialize successfully. Exactly one byte over must either fail closed with
  empty stdout or be truncated deterministically at a valid UTF-8 boundary and
  include a model-visible truncation marker; the implementation must not choose
  between those policies. Tests cover ASCII and multibyte boundary inputs and,
  for truncation, prove stable bytes plus marker visibility to the real agent.
- Rendering unit tests prove serialization only. Capability tests must replay
  PR #914's split result: post-tool proven, session-start blocked. A marker
  present only in hook stdout or logs is a failed injection result (B-012).

### 4. session-init (B-006)

Decision for the product spec's B-006: option (a) — `session-init` is NOT
wired on Cursor unless #822 disproves the documented permit/block-only
`beforeSubmitPrompt` contract and a later human-approved spec changes the
decision. The CLI dispatch entry accepts `cursor` as a recognized host value
but returns an explicit unsupported-command error and non-zero exit before
prompt-event persistence, context generation, stdout, enqueue, or spill.
Running session-init purely for side effects would duplicate the proposed
`sessionStart` capture path. `remem doctor` host diagnostics (surface added in
#824) reports "session-init: not supported on cursor" so the gap is visible,
not silent.

## Product-to-Test Mapping

| Behavior invariant | Implementation area | Verification |
|---|---|---|
| B-001 canonical host recognition plus per-command support | shared hook-host parser + `src/cli/dispatch.rs` + hook entrypoints | exact three host values parse; aliases/unknown/empty fail at all commands and persistence boundaries; `session-init --host cursor` returns explicit unsupported/non-zero before prompt write, stdout, or any side effect |
| B-002 sessionStart maps one normalized workspace root; parent events retain one cross-event identity while child events retain distinct event-local identity; null transcript_path valid | `src/context/invocation.rs` + shared Cursor identity validator + platform path normalizer | PR #914 event-local equal session_id/conversation_id fixtures succeed; mismatch fails before side effects; a distinct subagent identity is accepted without parent coercion or invented linkage; null start/early/child paths stay null and never inherit parent; `len == 1` plus trimmed non-empty root normalizes before cwd/git/project derivation; unknown platform shapes and invalid/multi-root arrays fail without raw identity persistence or cwd/env fallback |
| B-003 exact event discriminator drives bounded additional_context JSON | Cursor parser + `src/context/render.rs` | exact sessionStart serialization is testable but 3.12.17 capability remains disabled from PR #914 absent marker; postToolUse capability is separately proven and needs its own approved contract; missing/unknown/mismatched events exit non-zero; any later-approved numeric limit has exact/one-byte-over tests; other hosts remain green |
| B-004 no control instructions in payload | `src/context/render.rs` | regression test asserting absence of GH668 marker strings in Cursor output |
| B-005 failure → empty stdout + error log, never broken JSON | context entrypoint + `src/context/render.rs` | tests: empty body and generation failure emit no stdout; serialization is atomic |
| B-006 Cursor session-init is rejected, doctor-visible | CLI dispatch + #824 doctor surface | subprocess asserts explicit unsupported non-zero plus empty stdout and zero prompt writes/enqueues/spills; no UserPromptSubmit-equivalent in #824 hooks fixture; doctor line test in #824 |
| B-007 observe maps verified identity before success/failure capture; unknown tool_name uses verbatim generic capture | Cursor parser + canonical event/capture/spill/DB schema + adapter boundary + #822 failure probe | PR #914 pre-tool Read/Shell/Task/MCP and successful post-tool Read/Shell/MCP fixtures validate without inventing Task success; raw generic input/output exact-byte max and one-byte-over fixtures run before classification; failed-Read tool_use_id validates before capture; `SomethingNew` remains verbatim; approved failure stores explicit outcome exactly once; unobserved Task-success/Write/Edit/Delete/failed-Shell paths remain disabled or generic rather than guessed |
| B-008 stop maps identity, status, loop, and #825 reader gate | Cursor parser + `src/summarize` + #822 + #825 | before #825, path never reaches Claude/Codex reader/enqueue/spill/LLM; after prerequisites, PR #914 completed/aborted + numeric loop 0 fixtures and proposed `(session_id,generation_id,loop_count)` replay/conflict matrix pass; missing/blank/wrong-typed generation_id and error/nonzero/missing/null loop shapes remain rejected until approved |
| B-009 malformed, oversized, or mismatched stdin fails closed | shared bounded Cursor stdin reader + context/observe/summarize command entrypoints | subprocess tests prove exact 1,048,576-byte input reaches normal UTF-8/JSON validation while one-byte-over is rejected before String/serde with a size-only error, empty stdout, and zero writes/enqueues/spills/adapter/LLM calls; invalid JSON, missing fields (including stop generation_id/status), blank/wrong-typed generation_id, wrong-typed/unknown stop status, identity mismatch, unknown event, and every event/command mismatch assert the same fail-closed boundary |
| B-010 Claude/Codex zero regression | whole crate | `cargo test` full suite; no existing test modified |
| B-011 DB host value is `cursor` | shared host parser + capture/enqueue/persistence boundaries | `as_db_value()` unit test plus DB integration tests proving only canonical `cursor` reaches each hook-origin host column |
| B-012 capability-specific real-agent marker gate | #822/PR #914 evidence | Cursor 3.12.17 postToolUse is proven and sessionStart is blocked; version/mode/size mismatch cannot promote either state |
| B-013 invalid and multi-root arrays remain fail-closed | context/observe/summarize parsing + #822/human gate | each event fixture covers `[""]`, `["", "/repo"]`, `["/repo", ""]`, and two non-empty roots and returns non-zero with no stdout/write/enqueue/spill; implementation cannot enable multi-root until a recorded human decision |
| B-014 user_email/PII removed before every sink | Cursor sanitization boundary + capture/spill/adapter/summarize paths | unique email sentinel is absent from DB, decoded spill fixture, logs/errors, adapter request, LLM prompt, and generated summary across success and forced-failure paths |
| B-015 bounded variant validation/decode precedes classification | Cursor parser + generic/known-tool dispatch | generic object input/string output and MCP string input/result fixtures validate exact types; raw generic UTF-8, encoded, and canonical lengths at approved limit succeed; every one-byte-over/malformed case fails with zero writes/calls |
| B-016 real MCP event gate | #822/PR #914 evidence + conditional Cursor observe variants | browser_tabs fixture proves generic and before/after MCP delivery with string input/result; approved MCP-specific ownership registers and parses only afterMCPExecution, keeps beforeMCPExecution unregistered, and makes generic MCP postToolUse zero-write; generic ownership keeps both specific events unregistered; either mode has exactly one canonical capture/upsert |

## Risks

- R1. Cursor hook schema is younger than Claude's and may drift; every
  external-contract claim above is re-verified by #822 before implementation.
- R2. `tool_name` vocabulary mismatch could starve the observe matcher
  (`src/observe/hook.rs:190`); mitigated by B-007 (record verbatim) so data is
  not lost even before the mapping table is tuned.
- R3. `additional_context` may be rendered visibly in Cursor's UI (the GH668
  Codex lesson); B-004 keeps the payload free of instruction text so worst
  case is a visible context block, not a leaked directive.
- R4. Choosing the wrong root in a multi-root workspace can inject or persist
  another project's memory. Filtering blanks before checking array length is
  also ambiguous and could hide a malformed producer. B-002/B-008/B-013 require
  one root and block ambiguity; neither current cwd nor an undeclared
  environment variable is a fallback.
- R5. A raw Cursor workspace string may not be a native path on the current
  platform (for example `/c:/...` on Windows). Normalizing without real-host
  evidence can create a false project identity; #822-backed fixtures define
  accepted conversions and every unverified form fails before persistence.
- R6. Treating a valid host as valid for every command can activate unsupported
  side effects. The shared parser and dispatch support matrix are separate, and
  Cursor session-init is explicitly rejected before execution.
- R7. Cursor account PII such as `user_email` can leak through generic capture,
  spill, error previews, or LLM prompts unless removed before canonicalization;
  B-014 makes the sanitized event the only downstream representation.
- R8. Raw strings or nested JSON strings can exceed the bound or alter
  classification during decode. B-015 bounds raw, encoded, and decoded forms
  before dispatch.
- R9. PR #914 proves generic plus before/after MCP-specific delivery for one
  call. B-016 selects the terminal after event as the only MCP-specific writer,
  leaves the before event unregistered, and makes the generic copy zero-write
  so the evidence cannot be persisted twice.

## Verification Plan

- `cargo fmt --check && cargo check`
- `cargo test` (full suite; new tests listed in the mapping)
- Fix PR #914 exact-head Cursor 3.12.17 sanitized fixtures as the v2 evidence
  baseline, subject to human adoption:
  - replay exact field/type shapes for `sessionStart`, `postToolUse`,
    `postToolUseFailure`, `stop`, `preCompact`, generic/MCP tool events, and
    subagent events;
  - assert equal `session_id`/`conversation_id`, null start/early/subagent
    transcript paths, stable later parent path, and no parent-path substitution;
  - replay exact generic tool types/names and failed-Read `tool_use_id`;
  - replay the real MCP generic plus before/after-specific delivery and prove
    MCP-specific ownership registers/writes only the terminal after event while
    generic ownership keeps both specific events unregistered;
  - replay the capability split: short post-tool marker proven, short
    session-start marker blocked;
  - replay completed/aborted, numeric `loop_count:0`, conditional token fields,
    and the approved Stop-key replay/conflict matrix.
- Follow-up #822 evidence or explicit fail-closed human decisions:
  - observe Write/Edit/Delete and failed Shell before adding known-tool/failure
    shortcuts;
  - compare foreground with true background/cloud and multi-root sessions;
  - measure context/tool field limits only on an enabled capability;
  - exercise zero, one, and multiple `workspace_roots`, including `[""]`,
    `["", "/repo"]`, `["/repo", ""]`, and two non-empty roots; keep every
    shape other than `len == 1` plus a trimmed non-empty sole element blocked;
  - record sanitized native and Windows/UNC workspace-root fixtures; freeze
    normalization only for observed shapes and assert unverified forms cannot
    become stored project identity;
  - exercise exact, missing, unknown, and command-mismatched
    `hook_event_name` values and confirm only `context` + `sessionStart` selects
    Cursor `additional_context` rendering;
  - invoke `session-init --host cursor` and assert explicit unsupported
    non-zero, empty stdout, and zero prompt/capture side effects;
  - exercise missing, blank, single-root/cwd, and multi-root project identity on
    `stop`; fail closed
    without process-cwd or `CURSOR_PROJECT_DIR` fallback; before #825, prove a
    Cursor path cannot reach the Claude/Codex transcript reader; exercise exact
    `completed`/`aborted`, and reject unapproved `error`, nonzero/missing/null
    loop, blank, wrong-typed, and unknown values before downstream calls.
- After the evidence/spec human gates pass, pipe approved sanitized payload fixtures through
  `remem context --host cursor`, `remem observe --host cursor`, and
  `remem summarize --host cursor`; assert exact stdout, exit status, and
  persistence outcomes.
- Run the B-014 sentinel suite through capture success, forced database-open
  spill, replay, parse error, adapter request, and LLM fake-provider paths.
- Run B-015 malformed, raw-generic exact/one-byte-over, encoded-over-limit, and
  decoded-expansion-over-limit cases before classifier invocation.
- For an approved failed-tool mapping, run the observed failed-Read fixture
  through canonical capture/spill/replay/database and downstream consumers;
  assert its explicit stored failure outcome, exactly-once persistence when
  both events fire, and failure precedence. For unobserved Bash and edit/write
  failure variants, assert only the approved generic/disabled behavior and no
  invented outcome. Otherwise, verify #824 installs no Cursor observe hook and
  doctor reports capture as explicitly incomplete.

## Rollback

Runtime-only change behind the `--host cursor` value; nothing writes it until
#824 ships. Cursor summarize also stays unwired until #825 ships. Rollback is
reverting the PR; no data migration is involved
(B-011 values only appear once #824 installs hooks).
