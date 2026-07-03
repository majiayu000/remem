# Product Spec

## Linked Issue

GH-668

## User Problem

Codex App can render `SessionStart` hook context in the thread UI. remem also
previously carried a first-response workaround that told the assistant to print
a compact `Remem context:` status line. When both surfaces are visible, users
see duplicate startup status and may see internal hook instructions that were
intended only for model coordination.

## Goals

- Preserve automatic Codex `SessionStart` memory injection.
- Use the supported Codex hook context contract for model-visible memory.
- Ensure remem does not ask the assistant to render a second startup status
  line after Codex has already run the hook.
- Keep duplicate-injection gating silent for repeated same-session
  `SessionStart` events.
- Document the current visibility boundary accurately.

## Non-Goals

- Do not remove automatic `SessionStart` context injection.
- Do not solve the broader Codex plugin GUI work tracked by GH-390.
- Do not invent an Apps SDK app or connector id.
- Do not claim Codex hook context is hidden from the UI while current Codex
  builds still render completed-hook context blocks.

## Behavior Invariants

1. Codex hook invocations that emit memory context carry it via
   `hookSpecificOutput.additionalContext`.
2. Direct terminal `remem context --host codex-cli` output remains plain text.
3. Suppressed duplicate `SessionStart` output remains empty stdout.
4. The emitted context must not contain a first-response workaround instruction
   or require the assistant to print `Remem context:`.
5. ANSI display styling must not leak into model-visible structured hook
   context.

## Acceptance Criteria

- [ ] Simulated Codex `SessionStart` invocation emits valid JSON with
      `hookSpecificOutput.hookEventName = "SessionStart"` and full context in
      `additionalContext`.
- [ ] Direct CLI context output remains plain text for manual use.
- [ ] Duplicate/suppressed hook output remains silent.
- [ ] Regression tests prove the structured hook output does not include the
      first-response workaround or `Remem context:` line.
- [ ] README and plugin docs describe the current Codex visibility contract and
      do not promise hidden UI behavior.

## Rollout Notes

This is a hook-output contract change with no database migration. Existing
users keep the same hook command; the binary chooses structured JSON only when
the invocation looks like a Codex `SessionStart` hook.
