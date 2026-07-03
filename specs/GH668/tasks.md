# Task Plan

## Linked Issue

GH-668

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`

## Implementation Tasks

- [ ] `SP668-T1` Owner: agent; Dependencies: none; Done when: Codex `SessionStart` hook output is serialized as `hookSpecificOutput.additionalContext` while manual context output stays plain text; Verify: focused render tests.
- [ ] `SP668-T2` Owner: agent; Dependencies: `SP668-T1`; Done when: duplicate suppression still emits empty stdout and structured hook context strips ANSI styling before model injection; Verify: focused render tests.
- [ ] `SP668-T3` Owner: agent; Dependencies: `SP668-T1`; Done when: docs describe current Codex hook visibility and no assistant-rendered `Remem context:` fallback remains documented; Verify: docs diff.
- [ ] `SP668-T4` Owner: agent; Dependencies: `SP668-T1` `SP668-T2` `SP668-T3`; Done when: local checks pass and the PR references GH-668; Verify: commands below.

## Verification

- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH668`
- `cargo fmt --check`
- `cargo check`
- Focused render tests listed in `tech.md`
- `cargo test` before merge readiness

## Handoff Notes

Use `Refs #668` for partial implementation evidence. Close GH-668 only after
maintainer review confirms the Codex App thread no longer shows both a hook
context block and an assistant-rendered `Remem context:` line.
