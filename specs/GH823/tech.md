# Cursor Hook I/O Protocol Technical Spec

Status: Draft, needs human approval before implementation
Date: 2026-07-15

Tracking:
- Spec/tracking issue: #823
- Product spec: specs/GH823/product.md
- Epic: #821 · Prerequisite PoC: #822
- Readiness: blocked on #822 real-host evidence and human approval; Cursor
  summarize is additionally blocked on #825's verified transcript reader.

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

External-contract candidates for #822 to verify, not implementation facts:
the official [Cursor Hooks documentation](https://cursor.com/docs/hooks)
(rechecked 2026-07-15) and review evidence describe JSON hook payloads with
`transcript_path`, `sessionStart`, `postToolUse`, and `stop` fields; review
evidence additionally reports conversation and workspace identity on
`postToolUse` and `stop`. Documentation is not proof of the installed Cursor
version's emitted payloads, platform-specific path forms, or model-visible
behavior. #822 must record the exact real-host payloads and prove whether
top-level `additional_context` reaches a real agent before this design leaves
its blocked state.

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
  (gating, budget) starts identical to `ClaudeCodeContextProfile` unless #822
  shows a reason to differ.

### 2. Stdin parsing (B-002, B-007, B-008, B-009)

- `src/context/invocation.rs`: represent stdin as an explicit no-input, valid,
  or parse-error result instead of collapsing malformed input into `None`.
  Under `--host cursor`, invalid JSON, read failure, missing required fields,
  or wrong field types return an error to CLI dispatch. Context generation is
  not called, the process exits non-zero, stdout remains empty, and no current
  cwd/CLI fallback is permitted.
- The Cursor input model requires `hook_event_name` and validates it before
  dispatch: `context` accepts only exact `sessionStart`, `observe` only exact
  `postToolUse`, and `summarize` only exact `stop`. Unknown values and
  event/command mismatches return non-zero before adapter or renderer selection;
  they never fall through to plain-text or Claude-shaped output.
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
  null-tolerant.
- `src/observe/hook.rs`: accept the Cursor `postToolUse` shape only after #822
  verifies the exact identity field types. Before adapter dispatch or capture,
  map required non-empty `conversation_id` to the canonical `session_id` and
  apply the same total single-root validation plus platform normalization used
  for `sessionStart`; the verified normalized root becomes cwd/project. Missing
  or wrong-typed identity, `[]`, `[""]`, mixed-empty, multi-root, or unverified
  platform shapes return non-zero with zero writes. Both `tool_input` and
  `tool_output` arrive JSON-stringified. Before any classification or filter,
  validate the encoded UTF-8 byte length, decode exactly once, serialize the
  decoded JSON canonically, and validate that decoded byte length against one
  numeric `CURSOR_TOOL_FIELD_MAX_BYTES` frozen by the #822 evidence and human
  approval. Invalid nested JSON or either over-limit representation returns
  non-zero before capture, spill, adapter dispatch, or raw-data diagnostics.
  Tool-name mapping lives in one place; unknown names bypass known-tool
  classification but still use the existing generic capture path with the
  original `tool_name` and decoded input/output. They are never remapped or
  discarded after diagnosis (B-007, B-015).
- `src/summarize`: accept the Cursor `stop` shape only after #822 verifies its
  exact fields. Map required non-empty `conversation_id` to
  `SummarizeInput.session_id` before the existing missing-session early return,
  enqueue, spill identity, or persistence paths. #822 must also record the
  exact project-root field/type emitted on `stop`. Apply the same total
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
  existing Claude/Codex parser merely because it is a string. After #825 lands,
  decision (B-008): `aborted`/`error` still preserve already captured events;
  the LLM summary call runs for `completed` and `aborted`, and is skipped with
  an error-level log for `error`. This decision remains subject to the real
  payload evidence.
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
- Rendering unit tests prove serialization only. The implementation and install
  path remain blocked until #822 observes a unique synthetic marker in a real
  Cursor agent's model-visible context. A marker present only in hook stdout or
  logs is a failed injection result (B-012).

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
| B-002 sessionStart maps one normalized workspace root; null transcript_path valid | `src/context/invocation.rs` + platform path normalizer | `len == 1` plus trimmed non-empty #822-backed root normalizes before cwd/git/project derivation; sanitized Windows `/c:/...` fixture proves conversion; unknown path shapes and `[]`, `[""]`, `["", "/repo"]`, `["/repo", ""]`, and two non-empty roots fail without raw identity persistence or cwd/env fallback |
| B-003 exact event discriminator drives additional_context JSON | Cursor parser + `src/context/render.rs` | exact `hook_event_name: "sessionStart"` on context emits top-level `additional_context`; missing/unknown/mismatched events exit non-zero with empty stdout and no plain-text/Claude fallback; other hosts remain green |
| B-004 no control instructions in payload | `src/context/render.rs` | regression test asserting absence of GH668 marker strings in Cursor output |
| B-005 failure → empty stdout + error log, never broken JSON | context entrypoint + `src/context/render.rs` | tests: empty body and generation failure emit no stdout; serialization is atomic |
| B-006 Cursor session-init is rejected, doctor-visible | CLI dispatch + #824 doctor surface | subprocess asserts explicit unsupported non-zero plus empty stdout and zero prompt writes/enqueues/spills; no UserPromptSubmit-equivalent in #824 hooks fixture; doctor line test in #824 |
| B-007 observe maps verified identity before postToolUse capture; unknown tool_name uses verbatim generic capture | Cursor parser + `src/observe/hook.rs` + adapter boundary | fixture maps identity before capture; `SomethingNew` remains verbatim, bypasses known-tool classification, and still persists decoded generic input/output; no diagnostic-and-drop branch exists |
| B-008 stop maps identity; #825 gates transcript reads | Cursor parser + `src/summarize` + #822 + #825 | before #825, a valid-looking Cursor path returns explicit unsupported/non-zero and a spy proves no Claude/Codex transcript read, enqueue, spill, or LLM call; after both prerequisites, fixtures cover identity and `completed` / `aborted` / `error` |
| B-009 malformed or mismatched stdin fails closed | context/observe/summarize command entrypoints | subprocess tests for invalid JSON, missing fields, unknown event, and every event/command mismatch assert non-zero exit, empty stdout, error log, and zero writes/enqueues/spills |
| B-010 Claude/Codex zero regression | whole crate | `cargo test` full suite; no existing test modified |
| B-011 DB host value is `cursor` | shared host parser + capture/enqueue/persistence boundaries | `as_db_value()` unit test plus DB integration tests proving only canonical `cursor` reaches each hook-origin host column |
| B-012 real-agent marker gate | #822 PoC evidence | unique synthetic marker appears in a real Cursor agent's model-visible context; stdout-only marker is failure and blocks injection |
| B-013 invalid and multi-root arrays remain fail-closed | context/observe/summarize parsing + #822/human gate | each event fixture covers `[""]`, `["", "/repo"]`, `["/repo", ""]`, and two non-empty roots and returns non-zero with no stdout/write/enqueue/spill; implementation cannot enable multi-root until a recorded human decision |
| B-014 user_email/PII removed before every sink | Cursor sanitization boundary + capture/spill/adapter/summarize paths | unique email sentinel is absent from DB, decoded spill fixture, logs/errors, adapter request, LLM prompt, and generated summary across success and forced-failure paths |
| B-015 bounded nested JSON decode precedes classification | Cursor parser + generic/known-tool dispatch | encoded and canonical-decoded lengths at the approved limit succeed; either form one byte over and malformed nested JSON fail with zero writes/calls; classification spies prove decode completed first |
| B-016 real MCP event gate | #822 PoC evidence | one real MCP call records whether postToolUse fires; separate instrumented beforeMCPExecution/afterMCPExecution probes record fire/no-fire, ordering, and sanitized payloads; docs-only inference fails the gate |

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
- R8. Nested JSON strings can expand during decode or alter classification.
  B-015 bounds encoded and decoded forms and requires decode before dispatch.
- R9. MCP hook names do not prove that hooks fire or share tool-event payloads.
  B-016 permits mappings only from instrumented real-host evidence.

## Verification Plan

- `cargo fmt --check && cargo check`
- `cargo test` (full suite; new tests listed in the mapping)
- #822 real-host PoC, recording the Cursor version and sanitized raw evidence:
  - capture exact event names and payload field names/types for `sessionStart`,
    `postToolUse`, `stop`, and any observed `preCompact`, including exact
    conversation/project-root fields on `postToolUse` and `stop`;
  - invoke real Cursor tools and record their exact `tool_name` values; record
    encoded and decoded tool-field sizes and propose the numeric
    `CURSOR_TOOL_FIELD_MAX_BYTES` for human approval;
  - invoke at least one real MCP tool and record whether `postToolUse` arrives;
    separately instrument and exercise `beforeMCPExecution` and
    `afterMCPExecution`, recording fire/no-fire, sanitized payloads, and ordering
    without inferring a mapping;
  - compare foreground and background-agent sessions and record whether hooks
    fire and whether context becomes model-visible;
  - emit a unique synthetic marker from the hook and verify that a real Cursor
    agent receives it; a marker visible only in stdout/logs blocks injection;
  - probe context sizes around the largest accepted payload and record
    truncation, rejection, and agent-visible behavior;
  - exercise zero, one, and multiple `workspace_roots`, including `[""]`,
    `["", "/repo"]`, `["/repo", ""]`, and two non-empty roots; keep every
    shape other than `len == 1` plus a trimmed non-empty sole element blocked;
  - record sanitized native and Windows workspace-root fixtures, including the
    observed `/c:/...` form if emitted; freeze normalization only for observed
    shapes and assert unverified forms cannot become stored project identity;
  - exercise exact, missing, unknown, and command-mismatched
    `hook_event_name` values and confirm only `context` + `sessionStart` selects
    Cursor `additional_context` rendering;
  - invoke `session-init --host cursor` and assert explicit unsupported
    non-zero, empty stdout, and zero prompt/capture side effects;
  - exercise missing, blank, single-root/cwd, and multi-root project identity on
    `stop`; keep summarize blocked until the field is verified and fail closed
    without process-cwd or `CURSOR_PROJECT_DIR` fallback; before #825, prove a
    Cursor path cannot reach the Claude/Codex transcript reader.
- After the PoC gate passes, pipe its sanitized payload fixtures through
  `remem context --host cursor`, `remem observe --host cursor`, and
  `remem summarize --host cursor`; assert exact stdout, exit status, and
  persistence outcomes.
- Run the B-014 sentinel suite through capture success, forced database-open
  spill, replay, parse error, adapter request, and LLM fake-provider paths.
- Run B-015 malformed, exact-limit, encoded-over-limit, and
  decoded-expansion-over-limit cases before classifier invocation.

## Rollback

Runtime-only change behind the `--host cursor` value; nothing writes it until
#824 ships. Cursor summarize also stays unwired until #825 ships. Rollback is
reverting the PR; no data migration is involved
(B-011 values only appear once #824 installs hooks).
