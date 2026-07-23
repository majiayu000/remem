# Cursor Hook I/O Protocol Product Spec

Status: Draft, needs human approval before implementation
Date: 2026-07-23

Tracking:
- Spec/tracking issue: #823
- Epic: #821
- Blocking prerequisite: #822 evidence PR #914 exact head
  `c4ab9b84788bc349b9674525b4c2bf5400f6606f`, still awaiting human adoption
- Related runtime surfaces: `remem context`, `remem observe`, `remem summarize`, host identity
- This packet incorporates the observed Cursor 3.12.17 contract from PR #914,
  but remains blocked on human evidence/spec approval and the explicitly
  unresolved limits/platform cases. No implementation task may start from this
  draft alone.

## Problem

remem's hook subcommands currently speak two host protocols: Claude Code and
Codex. Cursor provides a similar hook system (`hooks.json`, JSON on stdin,
JSON on stdout) with different field and event names. PR #914 recorded the
actual Cursor 3.12.17 payload shapes and demonstrated that context behavior is
event- and version-specific: short `postToolUse.additional_context` was
model-visible, while short `sessionStart.additional_context` was ignored even
though the hook fired successfully. Background/cloud behavior, platform path
forms, and numeric size limits remain unproved.

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
- N2. No Cursor transcript file format parsing; that belongs to #825. Cursor
  `stop` summarization is therefore blocked on #825 and must not reuse the
  Claude/Codex raw transcript reader in the interim.
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
   `sessionStart`, `postToolUse`, and `stop` must share one human-approved
   canonical session identity. PR #914 observed both `session_id` and
   `conversation_id` on every captured event and found them equal. The parser
   therefore requires exact equality whenever both fields are present; a
   missing, blank, wrong-typed, or mismatched
   identity fails closed before output, capture, enqueue, spill, or persistence.
   `sessionStart.transcript_path` is valid `null`; the first prompt/tool events
   and inner subagent tool events may also carry `null`. Later parent events
   and Stop used one stable string path. A null child path must never be
   replaced with the parent path.
3. B-003 The Cursor parser requires and strictly validates the common
   `hook_event_name` discriminator. Only exact
   `hook_event_name: "sessionStart"` on the `context` command can select the
   Cursor injection renderer. Unknown event names or event/command mismatches
   fail non-zero with no plain-text or Claude-shaped fallback. When that exact
   event succeeds under `--host cursor`, stdout is a single JSON object whose
   `additional_context` field carries the ANSI-stripped context body. No other
   host's output shape changes. This serialization contract does not imply
   capability support: PR #914 proved the short marker was not model-visible
   on `sessionStart` in Cursor 3.12.17, so installation/advertising of this
   injection path remains blocked for that version. In contrast, a short
   `postToolUse.additional_context` marker was model-visible; any use of that
   capability requires a separate human-approved output/ownership contract and
   cannot inherit the session-start limit. For any capability enabled after its
   smallest bounded marker works, #822 and human approval must freeze a numeric
   `CURSOR_ADDITIONAL_CONTEXT_MAX_BYTES` and exact UTF-8 measurement point.
   A body exactly at that limit succeeds. For a body one byte over, the
   approved policy must be either fail-closed with empty stdout or deterministic
   UTF-8-safe truncation with a model-visible truncation marker; implementation
   remains blocked until the numeric limit, measurement point, and one-byte-over
   policy are all approved.
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
   PR #914 observed default fail-open behavior for non-zero/timeout hooks but no
   injection capability. Unless a later human-approved spec
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
   valid payload also parses `tool_name`, `tool_input`, and `tool_output` into
   the existing observe event model. PR #914 observed exact generic names
   `Read`, `Shell`, `Task`, and `MCP:browser_tabs`; generic `tool_input` was an
   object and successful `tool_output` was a string. These fields must not be
   decoded as the old draft's guessed pair of JSON strings. Variant-specific
   fields are validated under the #822-approved byte limit before any tool
   classification, filtering, capture, or adapter dispatch; malformed or
   over-limit fields fail closed with zero writes. An
   unrecognized `tool_name` follows the existing generic-capture contract: it
   is recorded verbatim with the decoded generic input/output and is never
   skipped, silently rewritten, or forced through a known-tool classifier (the
   #817 failure class). PR #914 observed one failed `Read` as
   `postToolUseFailure` with string `error_message`,
   `failure_type: "error"`, numeric `duration`,
   `is_interrupt: false`, and the same `tool_use_id` as the matching pre-tool
   event. Human review may approve `tool_use_id` as the per-call key for this
   observed path. Write/Edit/Delete and a failed Shell remain unobserved; human
   review must either
   freeze a canonical event/upsert key and failure-precedence rule that safely
   correlates dual-event delivery and map the verified failure event into the
   observe model with
   an explicit canonical failure outcome/discriminator preserved through
   capture, spill, and database persistence (and consumed downstream where
   relevant), or keep Cursor observe explicitly incomplete and prevent #824
   from installing or advertising capture. If the real payloads expose no safe
   shared call identity, correlation or content-derived deduplication must not
   be guessed and the disabled/incomplete branch is mandatory. A success-only
   hook path must not be described as complete automatic capture.
8. B-008 When invoked with a Cursor `stop` payload, `remem summarize` maps the
   required non-empty Cursor `conversation_id` to remem's canonical
   `session_id` before enqueueing or persistence. #822 must also identify the
   exact project-root field and type emitted by a real Cursor `stop`. PR #914
   observed `workspace_roots`, equal `session_id`/`conversation_id`, and a
   string transcript path on Stop. When the verified root is missing, blank,
   multi-root, or otherwise ambiguous, summarize fails with a non-zero exit and
   performs no write, enqueue, or spill. After verification, only one validated
   root/cwd is mapped to the remem project; the hook process cwd and
   `CURSOR_PROJECT_DIR` are never fallbacks. Even after #822 verifies the stop
   identity fields, Cursor summarize remains unavailable until #825 lands a
   verified Cursor transcript reader. Before that prerequisite, the
   `transcript_path` field is stripped/deferred at the Cursor boundary and must
   never reach the existing Claude/Codex raw transcript parser, enqueue, spill,
   or LLM summarization path. `status` is a required string in the
   human-approved observed set. PR #914 observed exact `completed` and
   `aborted`; `error` was not observed and cannot enter the accepted set by
   analogy. Both observed statuses carried numeric `loop_count: 0`; completed
   included token-count fields while aborted omitted them. The canonical Stop
   key is proposed as `(session_id, generation_id, loop_count)` after B-002
   equality validation; replay/conflict behavior and nonzero/missing/null loop
   handling require exact-head human approval. A missing, blank, wrong-typed,
   unapproved status, or unapproved loop shape fails non-zero before transcript
   reading, enqueue, spill, persistence, or an LLM call. After #825, `aborted`
   and any later-approved `error` must not
   discard capture that was already persisted; whether they suppress the LLM
   summary call is an explicit decision recorded in the tech spec, not an
   accident.
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
12. B-012 Cursor context injection is capability-specific. PR #914's
    real-agent marker makes short `postToolUse.additional_context` proven for
    Cursor 3.12.17, but makes `sessionStart.additional_context` blocked for that
    same version. Inspecting hook stdout, logs, or a payload fixture alone is
    never sufficient. #823/#824 must report and gate these capabilities
    separately; a proven post-tool path must not promote session-start
    injection, another Cursor version, background mode, or an unmeasured size.
13. B-013 Multi-root Cursor workspaces remain unresolved until #822 and human
    review select an identity policy. Across `sessionStart`, `postToolUse`, and
    `stop`, workspace-root arrays are valid only when their total length is
    exactly one and that sole string is non-empty after trimming. `[]`, `[""]`,
    `["", "/repo"]`, `["/repo", ""]`, and arrays containing two non-empty
    roots all fail closed before output, capture, enqueue, or spill.
    Implementation must not discard blank elements and then pick the remaining
   root, silently pick the first root, merge projects, or use the hook process
   cwd.
14. B-014 Cursor identity metadata that is not part of remem's canonical event
    identity, including `user_email`, is removed or irreversibly redacted at
    the outer Cursor payload boundary. It must not appear in canonical capture
    events, database rows, spill files, logs/error previews, adapter requests,
    LLM prompts, or model output. Tests use a unique email sentinel and inspect
    every one of those sinks, including the database-open-failure spill path.
15. B-015 Cursor generic `postToolUse.tool_input` is a JSON object and
    `tool_output` is a string in PR #914; MCP-specific `tool_input` and
    `result_json` are strings. Each event variant validates its observed field
    types before tool-name mapping, known-tool classification, generic capture,
    or filtering. String-encoded MCP fields are decoded exactly once. Human
    approval freezes a numeric `CURSOR_TOOL_FIELD_MAX_BYTES` applying to
    encoded strings and canonical decoded/object representations. Invalid nested JSON, encoded input
    above the limit, or decoded expansion above the limit fails non-zero with
    no capture, enqueue, spill, adapter call, or diagnostic containing raw
    payload data. Boundary tests cover exactly-at-limit and one-byte-over data.
16. B-016 PR #914 invoked the read-only
    `cursor-ide-browser.browser_tabs` MCP tool and observed generic
    `preToolUse`/`postToolUse` under `MCP:browser_tabs` plus specific
    `beforeMCPExecution`/`afterMCPExecution`. The specific payload used string
    `tool_input`/`result_json`, `mcp_server_name: "cursor-ide-browser"`, and
    `tool_name: "browser_tabs"`. Human approval must select one canonical
    capture/upsert path and prevent double capture across generic and
    MCP-specific delivery. Documentation names or guessed mappings are not
    evidence; no MCP mapping may ship before that decision.

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
| Evidence and audit integrity | covered: B-011 (host provenance recorded truthfully), B-014 (PII sentinel absent), B-016 (real MCP probe) |
| Cancellation / interruption / partial completion | covered: B-008 (observed aborted and any later-approved error Stop payloads) |
| Resource exhaustion / payload expansion | covered: B-003 (additional_context limit and exact/one-byte-over behavior), B-015 (encoded and decoded tool-field limits) |
| Failed tool execution | covered: B-007 (real failure-event probe; preserve failure evidence or keep observe uninstalled/incomplete) |

## PR #914 Evidence Resolution and Remaining Gates

- Q1 partially resolved: every observed event carried equal string
  `session_id`/`conversation_id`, a `workspace_roots` array, and a
  context-dependent string/null `transcript_path`; foreground
  `sessionStart.is_background_agent` was `false`. Windows/UNC, multi-root,
  true background/cloud, and `sessionEnd` remain unobserved.
- Q2 partially resolved: observed generic names are `Read`, `Shell`, `Task`,
  and `MCP:browser_tabs`; one failed Read used `postToolUseFailure` and a shared
  `tool_use_id`. Write/Edit/Delete, failed Shell, and dual-event precedence
  remain unobserved.
- Q3 resolved for manual compaction: `/summarize` emitted `preCompact` with
  `trigger: "manual"` and numeric context/window/message fields. Its remem
  mid-session action remains a separate human product decision.
- Q4 resolved only for small markers: session-start injection failed while
  post-tool injection succeeded on 3.12.17. No numeric limit can be selected
  until the chosen capability works at the smallest bounded size.
- Q5 delivery resolved: generic and MCP-specific hooks both fired for the
  read-only browser call. Canonical single-capture ownership remains a human
  decision.
- Stop evidence: `completed` and `aborted` plus numeric `loop_count: 0` were
  observed; `error`, nonzero, missing, null, and replay stability remain
  unobserved.

## Acceptance Criteria

- A-1. All B-001..B-016 have automated verification per the tech spec mapping.
- A-2. `cargo test` passes with zero changes to existing Claude/Codex
  protocol tests.
- A-3. PR #914 exact-head evidence is human-adopted for the observed Cursor
  3.12.17 payloads, identity equality, null-path behavior, tool names, failed
  Read, MCP delivery, manual `preCompact`, completed/aborted and `loop_count:0`.
  Follow-up evidence or an explicit fail-closed human decision covers sanitized
  Windows root fixtures, background-agent behavior, the numeric
  additional-context limit/measurement/one-byte-over policy, the bounded
  nested-tool-field limit, failed-tool hook behavior, shared invocation identity,
  canonical deduplication/precedence and preservation policy, and the real MCP
  hook behavior and canonical single-capture path in B-016. The remaining gates are
  answered or explicitly parked behind a human-approved fail-closed downgrade
  before implementation starts.
- A-4. Capability evidence stays split: Cursor 3.12.17 post-tool injection may
  be approved from the visible marker, while session-start injection remains
  blocked from the absent marker. No implementation or installation path
  merges those states or claims an unproved size/version/mode.
- A-5. A unique `user_email` sentinel is absent from capture, database, spill,
  log/error, adapter, LLM-request, and model-output fixtures, and Cursor `stop`
  cannot call any Claude/Codex transcript parser before #825 is merged.
