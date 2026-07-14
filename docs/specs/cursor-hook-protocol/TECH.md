# Cursor Hook I/O Protocol Technical Spec

Status: Draft, needs human approval before implementation
Date: 2026-07-14

Tracking:
- Spec/tracking issue: #823
- Product spec: docs/specs/cursor-hook-protocol/PRODUCT.md
- Epic: #821 · Prerequisite PoC: #822

## Existing Implementation Facts

Verified against `origin/main` (`b191e3f6`), 2026-07-14:

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
- `src/summarize/input.rs:9` carries `transcript_path: Option<String>` into
  summarize; the transcript reader itself is
  `src/memory/raw_transcript.rs:13 read_transcript_content()` (owned by #825).
- Install-side hook command lines pass the host explicitly:
  `hook_command()` renders `<bin> <subcommand> --host <runtime_host>`
  (`src/install/config.rs:51-58`), with `runtime_host()` returning
  `"claude-code"` / `"codex-cli"` (`src/install/config.rs:43-48`). The install
  side is #824's surface; listed here because the `--host cursor` value this
  spec introduces is what #824 will write into `hooks.json`.

External contract (to be re-verified by #822): Cursor hooks receive JSON on
stdin with base fields including `transcript_path` (string | null);
`sessionStart` provides `session_id`, `is_background_agent`, optional
`composer_mode`, and accepts `additional_context` (top-level string) in the
hook's stdout JSON; `postToolUse` provides `tool_name`, `tool_input`,
`tool_output` (JSON-stringified), `tool_use_id`, `cwd`, `duration` and accepts
`additional_context`; `stop` provides `status`
("completed" | "aborted" | "error") and `loop_count`. Source:
https://cursor.com/docs/hooks (fetched 2026-07-14).

## Proposed Design

### 1. Host identity (B-001, B-011)

- `src/identity.rs`: add `InstallHost::Cursor` with db/env value `"cursor"`;
  extend `parse()` and its error message's valid-value list.
- `src/context/host.rs`: add `HostKind::Cursor` and a
  `CursorContextProfile`; `resolve_profile()` dispatches it. Profile policy
  (gating, budget) starts identical to `ClaudeCodeContextProfile` unless #822
  shows a reason to differ.

### 2. Stdin parsing (B-002, B-007, B-008, B-009)

- `src/context/invocation.rs`: extend `parse_hook_input()` to accept the
  Cursor `sessionStart` shape. Cursor event names map onto existing invocation
  sources: `sessionStart` → SessionStart/startup. Field mapping:
  `session_id` → session id, base `transcript_path` → `transcript_path`
  (null-tolerant, reuse `clean_optional`).
- `src/observe/hook.rs`: accept the Cursor `postToolUse` shape. `tool_output`
  arrives JSON-stringified — decode once, and on decode failure store the raw
  string rather than dropping the event. Tool-name mapping table lives in one
  place; unknown names pass through verbatim (B-007), never remapped.
- `src/summarize`: accept the Cursor `stop` shape (`status`, `loop_count`).
  Decision (B-008): `aborted`/`error` still run capture finalization; the LLM
  summary call runs for `completed` and `aborted`, and is skipped with an
  error-level log for `error` (the transcript may be truncated mid-tool-call).
  This decision is revisitable with #822 evidence.
- Malformed stdin: same failure path as today's malformed Claude payloads —
  error log with event context, no stdout emission (B-009).

### 3. Stdout rendering (B-003, B-004, B-005)

- `src/context/render.rs`: alongside `is_codex_session_start_hook()`, add the
  Cursor branch in `context_stdout_for_invocation()`: when
  `HostKind::Cursor` + sessionStart source, emit
  `{"additional_context": "<ansi-stripped body>"}` and nothing else. Empty
  context body → empty stdout (existing early return already does this).
- The rendered body is the existing host-independent context; no new
  instruction text is added (B-004). Add a regression test asserting the
  Cursor payload contains none of the GH668 instruction markers.

### 4. session-init (B-006)

Decision for the product spec's B-006: option (a) — `session-init` is NOT
wired on Cursor. `beforeSubmitPrompt` cannot inject, and running session-init
purely for side effects duplicates what `sessionStart` already does. `remem
doctor` host diagnostics (surface added in #824) reports
"session-init: not supported on cursor" so the gap is visible, not silent.

## Product-to-Test Mapping

| Behavior invariant | Implementation area | Verification |
|---|---|---|
| B-001 `cursor` accepted; unknown hosts enumerate closed set | `src/identity.rs` | unit tests beside `src/identity.rs:148-156` existing parse tests: `parse("cursor")` ok; `parse("vscode")` error message contains all three hosts |
| B-002 sessionStart parse, null transcript_path valid | `src/context/invocation.rs` | new fixture test beside `parses_codex_hook_stdin` (`src/context/invocation.rs:198`): Cursor payload with and without `transcript_path` |
| B-003 additional_context JSON on stdout, other hosts unchanged | `src/context/render.rs` | new render test: Cursor sessionStart → top-level `additional_context`; existing Codex/Claude render tests untouched and green |
| B-004 no control instructions in payload | `src/context/render.rs` | regression test asserting absence of GH668 marker strings in Cursor output |
| B-005 failure → empty stdout + error log, never broken JSON | `src/context/render.rs` | test: empty context body under Cursor host yields empty stdout |
| B-006 session-init not wired, doctor-visible | design decision + #824 doctor surface | assert no UserPromptSubmit-equivalent entry in #824's generated hooks.json fixture; doctor line test in #824 |
| B-007 observe parses postToolUse; unknown tool_name never rewritten | `src/observe/hook.rs` | unit test: Cursor payload with `tool_name: "SomethingNew"` recorded verbatim; JSON-stringified `tool_output` decoded |
| B-008 stop statuses: capture always, summary per decision table | `src/summarize` | unit tests for `completed` / `aborted` / `error` payloads |
| B-009 malformed stdin fails closed | `src/context/invocation.rs` | test: invalid JSON → parse error path, no partial invocation |
| B-010 Claude/Codex zero regression | whole crate | `cargo test` full suite; no existing test modified |
| B-011 DB host value is `cursor` | `src/identity.rs` | `as_db_value()` unit test |

## Risks

- R1. Cursor hook schema is younger than Claude's and may drift; every
  external-contract claim above is re-verified by #822 before implementation.
- R2. `tool_name` vocabulary mismatch could starve the observe matcher
  (`src/observe/hook.rs:190`); mitigated by B-007 (record verbatim) so data is
  not lost even before the mapping table is tuned.
- R3. `additional_context` may be rendered visibly in Cursor's UI (the GH668
  Codex lesson); B-004 keeps the payload free of instruction text so worst
  case is a visible context block, not a leaked directive.

## Verification Plan

- `cargo fmt --check && cargo check`
- `cargo test` (full suite; new tests listed in the mapping)
- Manual: pipe recorded #822 payload dumps through
  `remem context --host cursor` / `remem observe --host cursor` /
  `remem summarize --host cursor` and inspect stdout/log output.

## Rollback

Runtime-only change behind the `--host cursor` value; nothing writes it until
#824 ships. Rollback is reverting the PR; no data migration is involved
(B-011 values only appear once #824 installs hooks).
