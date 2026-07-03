# Tech Spec

## Linked Issue

GH-668

## Product Spec

Product: `product.md`

## Codebase Context

| Area | Files | Current behavior | Required change |
| --- | --- | --- | --- |
| Codex hook command | `src/install/config.rs`, `plugins/remem/scripts/remem-hook.js` | Runs `remem context --host codex-cli`. | Keep the command stable. |
| Hook input parsing | `src/context/invocation.rs` | Reads Codex stdin fields including `session_id`, `cwd`, `transcript_path`, and `source`. | Use existing `source` values to identify real Codex `SessionStart` hook calls. |
| Context rendering | `src/context/render.rs` | Produces plain text context and applies duplicate-injection gating. | Wrap emitted Codex `SessionStart` hook context as JSON `hookSpecificOutput.additionalContext`; keep empty output empty. |
| Styling | `src/context/style.rs` | Can render ANSI colored context for Codex. | Strip ANSI before putting text into model-visible structured JSON. |
| Docs | `README.md`, `plugins/remem/README.md`, `docs/spec-codex-context-injection-gating-2026-05-25.md` | Describe low-noise Codex context behavior, with some stale plain-stdout assumptions. | Document structured hook output and the visible completed-hook boundary. |

## Proposed Design

- Add a renderer helper that converts final context stdout for a
  `ContextInvocation`.
- If output is empty, return empty output unchanged so duplicate suppression
  remains silent.
- If the host is `codex-cli` and the hook source is one of Codex
  `SessionStart` sources (`startup`, `resume`, `clear`, `compact`), return:

```json
{
  "hookSpecificOutput": {
    "hookEventName": "SessionStart",
    "additionalContext": "<rendered remem context>"
  }
}
```

- Strip ANSI codes from the `additionalContext` payload.
- For all non-hook and non-Codex invocations, return the existing plain text.
- Do not emit `systemMessage`; it is a UI warning surface and would create
  another visible status path.

## Product-to-Test Mapping

| Product invariant | Verification |
| --- | --- |
| Structured Codex hook context | Unit test parses JSON and checks `hookEventName` plus `additionalContext`. |
| Direct CLI remains plain | Unit test with no hook source returns unchanged text. |
| Suppression remains silent | Unit test with empty output returns empty string. |
| No workaround leak | Unit test rejects `first assistant response` and `Remem context:` in structured stdout. |
| No ANSI in model context | Unit test wraps colored context and asserts stripped payload. |

## Risks

- If Codex adds a supported hidden hook context display flag later, remem should
  adopt it in a separate change after verifying current behavior.
- If Codex changes `SessionStart.source` names, remem may fall back to plain
  output for new source values until updated.
- Hook JSON serialization must not run for manual CLI use, or terminal users
  would lose the readable context block.

## Test Plan

- `cargo test context::tests::codex_hook_stdout --no-fail-fast`
- `cargo fmt --check`
- `cargo check`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH668`
