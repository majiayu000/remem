# Cursor Hook I/O Protocol Product Spec

Status: Draft, needs human approval before implementation
Date: 2026-07-14

Tracking:
- Spec/tracking issue: #823
- Epic: #821
- Blocking prerequisite: #822 (Cursor hooks contract PoC; open questions below are gated on it)
- Related runtime surfaces: `remem context`, `remem observe`, `remem summarize`, host identity

## Problem

remem's hook subcommands currently speak two host protocols: Claude Code and
Codex. Cursor (1.7+) exposes an isomorphic hook system (`hooks.json`, JSON on
stdin, JSON on stdout) but with different field names, different event names
(`sessionStart`, `postToolUse`, `stop`), and a different context-injection
contract: hooks return `additional_context` at the top level instead of
Claude's `hookSpecificOutput.additionalContext`. Cursor's `beforeSubmitPrompt`
can only permit or block a prompt; it cannot inject content, so remem's
`session-init` path has no full equivalent.

Without a Cursor protocol, remem cannot inject memory context or capture
sessions in Cursor at all.

This spec covers the runtime protocol only: host identity, stdin payload
parsing, and stdout rendering. Writing `hooks.json` / `mcp.json` during
install is `cursor-install-host/` (#824). Transcript file parsing is
`cursor-transcript-capture/` (#825).

## Decision

Add `cursor` as a first-class host identity and teach the context/observe/
summarize invocation layer to parse Cursor event payloads and render Cursor
hook JSON output, reusing the existing host-profile mechanism
(`--host <value>` flag, `HostKind` profile dispatch).

## Non-Goals

- N1. No install-side changes (`hooks.json`, `mcp.json`, `InstallTarget`,
  `HookStrategy`); those belong to #824.
- N2. No Cursor transcript file format parsing; that belongs to #825.
- N3. No project-level `.cursor/hooks.json` support.
- N4. No re-design of what context is injected; the rendered context body is
  host-independent and unchanged.
- N5. No workaround instructions asking the assistant to re-render status
  lines (the GH668 failure class must not be reintroduced on a new host).

## Behavior Invariants

1. B-001 `--host cursor` is accepted wherever `--host claude-code` and
   `--host codex-cli` are accepted today (context, observe, summarize,
   session-scoped identity). Any other unknown host value keeps failing with
   an error that enumerates the full closed set of valid hosts, now including
   `cursor`.
2. B-002 When invoked with `--host cursor` and a Cursor `sessionStart` stdin
   payload, `remem context` parses at minimum `session_id` and the base field
   `transcript_path` (string | null). A null or absent `transcript_path` is
   valid input, not an error.
3. B-003 When context injection succeeds under `--host cursor` for a
   `sessionStart` event, stdout is a single JSON object whose
   `additional_context` field carries the ANSI-stripped context body. No other
   host's output shape changes.
4. B-004 The Cursor `additional_context` payload must not contain
   prompt-level control instructions (no "render exactly one status line",
   no first-response workarounds, no hidden directives). It may contain the
   context body and compact metadata only.
5. B-005 When context generation fails or produces empty output under
   `--host cursor`, stdout is empty or a well-formed no-op and the failure is
   logged at error level. A broken or half-rendered JSON object must never be
   emitted.
6. B-006 `session-init` has no injection-capable Cursor event. The Cursor
   protocol must define its behavior explicitly as one of: (a) not wired on
   Cursor, or (b) wired to a non-injecting event for side effects only. The
   chosen behavior is recorded in this spec before implementation and is
   visible in `remem doctor` host diagnostics — it must not silently pretend
   the Claude behavior exists.
7. B-007 When invoked with a Cursor `postToolUse` payload, `remem observe`
   parses `tool_name`, `tool_input`, and `tool_output` (JSON-stringified per
   Cursor docs) into the existing observe event model. An unrecognized
   `tool_name` value is recorded as-is or explicitly skipped with a
   diagnostic; it must never be silently rewritten to a different tool class
   (the #817 failure class).
8. B-008 When invoked with a Cursor `stop` payload, `remem summarize` runs the
   same summarize entry as a Claude `Stop` hook. `status` values `aborted` and
   `error` must not abort capture of what was already observed; whether they
   suppress the LLM summary call is an explicit decision recorded in the tech
   spec, not an accident.
9. B-009 A stdin payload that is not valid JSON, or that is missing fields
   required by the event contract, fails closed: error-level log with the
   event name and parse failure, no partial context emission, exit code
   consistent with existing hook failure behavior.
10. B-010 Claude Code and Codex protocol behavior is byte-identical before and
    after this change for identical inputs (zero regression).
11. B-011 Host identity recorded in the database for Cursor-origin sessions
    uses one canonical value (`cursor`); it must never be stored as
    `claude-code`, `codex-cli`, or an empty string.

## Boundary Checklist

| Category | Verdict |
|---|---|
| Empty / missing input | covered: B-002 (null transcript_path), B-009 (missing fields) |
| Error and failure paths | covered: B-005, B-009 |
| Authorization / permission | N/A — hook stdin comes from the local Cursor process the user already runs; no new trust boundary beyond existing hosts |
| Concurrency / race / ordering | N/A — each hook invocation is an independent short-lived process; shared-state concurrency is owned by the existing capture/db layer, unchanged here |
| Retry / repetition / idempotency | covered: B-010 (re-invocation parity); duplicate-injection gating is host-independent and unchanged |
| Illegal state transitions | covered: B-006 (session-init cannot silently claim Claude semantics) |
| Compatibility / migration | covered: B-001 (closed-set host values), B-011 (DB host value) |
| Degradation / fallback | covered: B-005, B-006, B-007 (no silent rewrite), B-008 |
| Evidence and audit integrity | covered: B-011 (host provenance recorded truthfully) |
| Cancellation / interruption / partial completion | covered: B-008 (aborted/error stop payloads) |

## Open Questions (gated on #822)

- Q1. Exact `sessionStart` payload field set (`composer_mode`,
  `is_background_agent`, workspace roots) and whether background-agent
  sessions should receive injection at all.
- Q2. The closed set of Cursor `tool_name` values and their mapping onto the
  observe matcher (`Write` / `Edit` / `Bash` equivalents).
- Q3. Whether `preCompact` exists with usable semantics for a mid-session
  summarize trigger.
- Q4. Injection size limits or truncation behavior for `additional_context`.

## Acceptance Criteria

- A-1. All B-001..B-011 have automated verification per the tech spec mapping.
- A-2. `cargo test` passes with zero changes to existing Claude/Codex
  protocol tests.
- A-3. The four open questions are answered (or explicitly parked with a
  documented downgrade) before implementation starts.
