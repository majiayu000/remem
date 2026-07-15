# Cursor Hook I/O Protocol Product Spec

Status: Draft, needs human approval before implementation
Date: 2026-07-15

Tracking:
- Spec/tracking issue: #823
- Epic: #821
- Blocking prerequisite: #822 (Cursor hooks contract PoC; open questions below are gated on it)
- Related runtime surfaces: `remem context`, `remem observe`, `remem summarize`, host identity
- Review round: 3, the final autonomous spec-fix round; any later finding
  requires an explicit human decision before another change.

## Problem

remem's hook subcommands currently speak two host protocols: Claude Code and
Codex. Cursor documents a similar hook system (`hooks.json`, JSON on stdin,
JSON on stdout) with different field and event names. The exact payloads,
model-visible context behavior, background-agent behavior, and size limits are
not treated as implementation facts until the real-host PoC in #822 records
them. In particular, a hook printing `additional_context` successfully does
not by itself prove that a real Cursor agent receives that context.

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

1. B-001 `cursor` is recognized by the shared hook-host parser's exact closed
   set (`claude-code`, `codex-cli`, and `cursor`) and is supported by `context`,
   `observe`, and `summarize`. `session-init --host cursor` is an explicit
   unsupported combination: dispatch returns non-zero before any prompt write,
   context stdout, adapter call, or other side effect. Aliases, misspellings,
   empty strings, and arbitrary values fail at every hook command and
   persistence boundary before output or write, with an error that enumerates
   the full closed set. Internal host auto-detection must not make `unknown` a
   persistable or explicitly accepted hook host.
2. B-002 When invoked with `--host cursor` and a Cursor `sessionStart` stdin
   payload, `remem context` requires a non-empty `session_id` and a
   `workspace_roots` array satisfying both `len == 1` and
   `trim(workspace_roots[0])` is non-empty. Before canonical path, git-root, or
   project derivation, it applies a platform-aware normalization backed by a
   sanitized #822 fixture. Windows forms such as `/c:/...` are not converted
   by guesswork: an unverified path shape fails closed and the raw string is
   never persisted as project identity. It maps only the verified normalized
   root to the remem context cwd/project boundary and parses the base field
   `transcript_path` (string | null). A null or absent `transcript_path` is
   valid input. Every other array shape fails closed, including `[]`, `[""]`,
   mixed blank/non-blank arrays, and multiple non-empty roots. The hook process
   cwd and an undeclared `CURSOR_PROJECT_DIR` fallback must not select a project.
3. B-003 The Cursor parser requires and strictly validates the common
   `hook_event_name` discriminator. Only exact
   `hook_event_name: "sessionStart"` on the `context` command can select the
   Cursor injection renderer. Unknown event names or event/command mismatches
   fail non-zero with no plain-text or Claude-shaped fallback. When that exact
   event succeeds under `--host cursor`, stdout is a single JSON object whose
   `additional_context` field carries the ANSI-stripped context body. No other
   host's output shape changes.
4. B-004 The Cursor `additional_context` payload must not contain
   prompt-level control instructions (no "render exactly one status line",
   no first-response workarounds, no hidden directives). It may contain the
   context body and compact metadata only.
5. B-005 When context generation fails or produces empty output under
   `--host cursor`, stdout is empty and the failure is logged at error level.
   A broken, half-rendered, or fallback context JSON object must never be
   emitted.
6. B-006 No injection-capable Cursor equivalent for `session-init` is assumed.
   Cursor documentation describes `beforeSubmitPrompt` as permit/block only;
   #822 must verify the real host behavior. Unless a later human-approved spec
   changes this decision, `session-init --host cursor` fails explicitly at the
   dispatch entry with a non-zero exit before any prompt-event write, context
   stdout, enqueue, spill, or database side effect. `remem doctor` reports the
   unsupported combination; it must not silently pretend the Claude behavior
   exists.
7. B-007 When invoked with a verified Cursor `postToolUse` payload, `remem
   observe` maps required non-empty `conversation_id` to canonical `session_id`
   and applies B-002's exact single-root validation and platform-aware
   normalization to map the workspace root to cwd/project before any capture.
   Missing or wrong-typed identity fields and zero-, multi-, or mixed-empty-root
   arrays fail non-zero with no adapter dispatch or write. #822 must confirm the
   real field types and a sanitized Windows fixture before implementation. A
   valid payload also parses `tool_name`, `tool_input`, and `tool_output`
   (JSON-stringified per Cursor docs) into the existing observe event model. An
   unrecognized `tool_name` is recorded as-is or explicitly skipped with a
   diagnostic; it must never be silently rewritten to a different tool class
   (the #817 failure class).
8. B-008 When invoked with a Cursor `stop` payload, `remem summarize` maps the
   required non-empty Cursor `conversation_id` to remem's canonical
   `session_id` before enqueueing or persistence. #822 must also identify the
   exact project-root field and type emitted by a real Cursor `stop`. Until
   that evidence exists, or when that verified field is missing, blank,
   multi-root, or otherwise ambiguous, summarize fails with a non-zero exit and
   performs no write, enqueue, or spill. After verification, only one validated
   root/cwd is mapped to the remem project; the hook process cwd and
   `CURSOR_PROJECT_DIR` are never fallbacks. A valid event runs the same
   summarize entry as a Claude `Stop` hook. `status` values `aborted` and
   `error` must not discard capture that was already persisted; whether they
   suppress the LLM summary call is an explicit decision recorded in the tech
   spec, not an accident.
9. B-009 A stdin payload that is not valid JSON, that is missing fields
   required by the Cursor event contract, or whose `hook_event_name` is unknown
   or mismatched with the invoked command, fails closed: error-level log with
   the event name and a redacted parse failure, non-zero exit, no stdout, no
   fallback to CLI/current cwd, and no partial persistence.
10. B-010 Claude Code and Codex protocol behavior is byte-identical before and
    after this change for valid canonical-host inputs (zero regression).
    B-001's rejection of aliases and arbitrary explicit host values is the only
    intentional boundary tightening.
11. B-011 Host identity recorded in the database for Cursor-origin sessions
    uses one canonical value (`cursor`); it must never be stored as
    `claude-code`, `codex-cli`, `unknown`, an alias, an arbitrary value, or an
    empty string. All hook-origin tables receive host identity only after the
    B-001 closed-set validation succeeds.
12. B-012 Cursor context injection remains blocked until #822 proves, with a
    unique synthetic marker, that a real Cursor agent receives hook-provided
    context. Inspecting hook stdout, logs, or a payload fixture alone is not
    sufficient. If the marker is absent, the injection capability is parked as
    blocked and #823/#824 must not advertise or install it.
13. B-013 Multi-root Cursor workspaces remain unresolved until #822 and human
    review select an identity policy. Across `sessionStart`, `postToolUse`, and
    `stop`, workspace-root arrays are valid only when their total length is
    exactly one and that sole string is non-empty after trimming. `[]`, `[""]`,
    `["", "/repo"]`, `["/repo", ""]`, and arrays containing two non-empty
    roots all fail closed before output, capture, enqueue, or spill.
    Implementation must not discard blank elements and then pick the remaining
    root, silently pick the first root, merge projects, or use the hook process
    cwd.

## Boundary Checklist

| Category | Verdict |
|---|---|
| Empty / missing input | covered: B-002 (null transcript_path; workspace root required), B-009 (missing fields) |
| Error and failure paths | covered: B-005, B-009 |
| Authorization / permission | N/A — hook stdin comes from the local Cursor process the user already runs; no new trust boundary beyond existing hosts |
| Concurrency / race / ordering | N/A — each hook invocation is an independent short-lived process; shared-state concurrency is owned by the existing capture/db layer, unchanged here |
| Retry / repetition / idempotency | covered: B-010 (re-invocation parity); duplicate-injection gating is host-independent and unchanged |
| Illegal state transitions | covered: B-006 (session-init cannot silently claim Claude semantics) |
| Compatibility / migration | covered: B-001 (closed-set host values), B-011 (DB host value), B-013 (multi-root blocked) |
| Degradation / fallback | covered: B-005, B-006, B-007 (no silent rewrite), B-008 |
| Evidence and audit integrity | covered: B-011 (host provenance recorded truthfully) |
| Cancellation / interruption / partial completion | covered: B-008 (aborted/error stop payloads) |

## Open Questions (gated on #822)

- Q1. Exact `sessionStart` payload field names and types, including
  `composer_mode`, `is_background_agent`, and `workspace_roots`; whether
  background-agent sessions should receive injection; and the human-approved
   policy for payloads containing multiple workspace roots. Also identify the
   exact conversation/project-root fields and types on real `postToolUse` and
   `stop` payloads, plus sanitized Windows root forms.
- Q2. The observed closed set of Cursor `tool_name` values and their mapping
  onto the observe matcher (`Write` / `Edit` / `Bash` equivalents), using real
  tool invocations rather than documentation alone.
- Q3. Whether `preCompact` is emitted in the tested Cursor version, its exact
  payload and ordering, and whether it has usable mid-session summarize
  semantics.
- Q4. The model-visible behavior and practical size/truncation limit of
  `additional_context`, including a boundary test around the largest accepted
  synthetic context.

## Acceptance Criteria

- A-1. All B-001..B-013 have automated verification per the tech spec mapping.
- A-2. `cargo test` passes with zero changes to existing Claude/Codex
  protocol tests.
- A-3. #822 records the real Cursor version, exact event payloads, the
  `postToolUse` and `stop` conversation/project-root fields and types, sanitized
  Windows root fixtures, observed tool names, background-agent behavior,
  `preCompact` behavior, and context size behavior. The four open questions are
  answered or explicitly parked behind a human-approved fail-closed downgrade
  before implementation starts.
- A-4. #822 proves a unique synthetic marker is visible to a real Cursor agent.
  If it does not, Cursor injection is recorded as blocked and no implementation
  or installation path claims injection support.
