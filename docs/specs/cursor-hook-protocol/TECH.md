# Cursor Hook I/O Protocol Technical Spec

Status: Draft, needs human approval before implementation
Date: 2026-07-14

Tracking:
- Spec/tracking issue: #823
- Product spec: docs/specs/cursor-hook-protocol/PRODUCT.md
- Epic: #821 · Prerequisite PoC: #822

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
Cursor documentation and review evidence describe JSON hook payloads with
`transcript_path`, `sessionStart`, `postToolUse`, and `stop` fields; review
evidence additionally reports `conversation_id` on `stop`. #822 must record the
exact payloads emitted by the installed Cursor version and prove whether
top-level `additional_context` reaches a real agent before this design leaves
its blocked state.

## Proposed Design

### 1. Host identity (B-001, B-011)

- `src/identity.rs`: add `InstallHost::Cursor` with db/env value `"cursor"`;
  extend `parse()` and its error message's valid-value list.
- Introduce one exact hook-host parser shared by the `context`, `session-init`,
  `observe`, and `summarize` CLI boundaries and by hook persistence entrypoints.
  Its accepted values are exactly `claude-code`, `codex-cli`, and `cursor`.
  `runtime_config::normalize_host()` and `HostKind::Unknown` are not validation
  boundaries; aliases and arbitrary strings must fail before rendering,
  adapter dispatch, config creation, enqueueing, or database writes.
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
- Cursor `sessionStart` requires a non-empty `session_id` and exactly one
  non-empty `workspace_roots` entry. Map `workspace_roots[0]` to the existing
  invocation `cwd`, then derive project identity from that value. Missing or
  empty arrays fail closed. Arrays with multiple non-empty roots remain blocked
  pending #822 plus a human identity decision (B-013); do not silently select a
  root. Do not use the hook process cwd or an unverified
  `CURSOR_PROJECT_DIR`. The base `transcript_path` remains null-tolerant.
- `src/observe/hook.rs`: accept the Cursor `postToolUse` shape. `tool_output`
  arrives JSON-stringified — decode once, and on decode failure store the raw
  string rather than dropping the event. Tool-name mapping table lives in one
  place; unknown names pass through verbatim (B-007), never remapped.
- `src/summarize`: accept the Cursor `stop` shape only after #822 verifies its
  exact fields. Map required non-empty `conversation_id` to
  `SummarizeInput.session_id` before the existing missing-session early return,
  enqueue, spill identity, or persistence paths. Decision (B-008):
  `aborted`/`error` still preserve already captured events; the LLM summary call
  runs for `completed` and `aborted`, and is skipped with an error-level log for
  `error`. This decision remains subject to the real payload evidence.
- Apply the same explicit error result to Cursor `postToolUse` and `stop`:
  malformed or contract-incomplete input returns non-zero, emits no stdout, and
  performs no partial adapter dispatch, enqueue, spill, or database write.

### 3. Stdout rendering (B-003, B-004, B-005)

- `src/context/render.rs`: alongside `is_codex_session_start_hook()`, add the
  Cursor branch in `context_stdout_for_invocation()`: when
  `HostKind::Cursor` + sessionStart source, emit
  `{"additional_context": "<ansi-stripped body>"}` and nothing else. Empty
  context body → empty stdout (existing early return already does this).
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
`beforeSubmitPrompt` contract. Running session-init purely for side effects
would duplicate the proposed `sessionStart` capture path. `remem doctor` host
diagnostics (surface added in #824) reports
"session-init: not supported on cursor" so the gap is visible, not silent.

## Product-to-Test Mapping

| Behavior invariant | Implementation area | Verification |
|---|---|---|
| B-001 `cursor` accepted; every hook command rejects aliases/unknown/empty before side effects | shared hook-host parser + `src/cli/dispatch.rs` + context/observe/summarize entrypoints | unit tests for all four commands and persistence entrypoints: exact three values accepted; `curser`, `unknown`, aliases, and empty rejected with the same closed-set error and no write |
| B-002 sessionStart maps one workspace root; null transcript_path valid | `src/context/invocation.rs` | Cursor fixture with exactly one root maps it to cwd/project; null/absent transcript path succeeds; missing/empty roots fail without current-cwd fallback |
| B-003 additional_context JSON on stdout, other hosts unchanged | `src/context/render.rs` | new render test: Cursor sessionStart → top-level `additional_context`; existing Codex/Claude render tests untouched and green |
| B-004 no control instructions in payload | `src/context/render.rs` | regression test asserting absence of GH668 marker strings in Cursor output |
| B-005 failure → empty stdout + error log, never broken JSON | context entrypoint + `src/context/render.rs` | tests: empty body and generation failure emit no stdout; serialization is atomic |
| B-006 session-init not wired, doctor-visible | design decision + #824 doctor surface | assert no UserPromptSubmit-equivalent entry in #824's generated hooks.json fixture; doctor line test in #824 |
| B-007 observe parses postToolUse; unknown tool_name never rewritten | `src/observe/hook.rs` | unit test: Cursor payload with `tool_name: "SomethingNew"` recorded verbatim; JSON-stringified `tool_output` decoded |
| B-008 stop maps conversation identity; statuses preserve prior capture | `src/summarize` | fixtures require `conversation_id`, assert it becomes persisted/enqueued `session_id`, and cover `completed` / `aborted` / `error` decisions |
| B-009 malformed stdin fails closed | context/observe/summarize command entrypoints | subprocess tests for invalid JSON and missing required fields assert non-zero exit, empty stdout, error log, and zero writes/enqueues/spills |
| B-010 Claude/Codex zero regression | whole crate | `cargo test` full suite; no existing test modified |
| B-011 DB host value is `cursor` | shared host parser + capture/enqueue/persistence boundaries | `as_db_value()` unit test plus DB integration tests proving only canonical `cursor` reaches each hook-origin host column |
| B-012 real-agent marker gate | #822 PoC evidence | unique synthetic marker appears in a real Cursor agent's model-visible context; stdout-only marker is failure and blocks injection |
| B-013 multi-root remains fail-closed | `src/context/invocation.rs` + #822/human gate | two-root fixture returns non-zero with no stdout/write; implementation cannot enable multi-root until a recorded human decision |

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
  another project's memory. B-002/B-013 require one root and block ambiguity;
  neither current cwd nor an undeclared environment variable is a fallback.

## Verification Plan

- `cargo fmt --check && cargo check`
- `cargo test` (full suite; new tests listed in the mapping)
- #822 real-host PoC, recording the Cursor version and sanitized raw evidence:
  - capture exact event names and payload field names/types for `sessionStart`,
    `postToolUse`, `stop`, and any observed `preCompact`;
  - invoke real Cursor tools and record their exact `tool_name` values;
  - compare foreground and background-agent sessions and record whether hooks
    fire and whether context becomes model-visible;
  - emit a unique synthetic marker from the hook and verify that a real Cursor
    agent receives it; a marker visible only in stdout/logs blocks injection;
  - probe context sizes around the largest accepted payload and record
    truncation, rejection, and agent-visible behavior;
  - exercise zero, one, and multiple `workspace_roots`; keep multi-root blocked
    until a human approves an identity policy.
- After the PoC gate passes, pipe its sanitized payload fixtures through
  `remem context --host cursor`, `remem observe --host cursor`, and
  `remem summarize --host cursor`; assert exact stdout, exit status, and
  persistence outcomes.

## Rollback

Runtime-only change behind the `--host cursor` value; nothing writes it until
#824 ships. Rollback is reverting the PR; no data migration is involved
(B-011 values only appear once #824 installs hooks).
