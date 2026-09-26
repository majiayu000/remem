# Shared agent session parsing

Status: Current contract (implementation in progress)
Date: 2026-09-25

## Goal

Use agent-sessions 0.2 for Claude Code / Codex JSONL framing, native message and tool projection, and file discovery while preserving Remem's lossless archive and memory-quality policies.

## Acceptance

1. Raw physical rows, occurrence identity, empty messages, metadata/control messages, timestamp precedence and user/assistant roles retain existing meaning.
2. Captured boundaries reject short files before delivering the pending final row; appends beyond the boundary are ignored. Invalid UTF-8 remains an error. Only one pending row is retained.
3. Default scanning honors CLAUDE_CONFIG_DIR and CODEX_HOME. Empty overrides return an error. Nonempty overrides are required sources: missing, non-directory or unreadable projects/sessions directories fail the scan; only inferred default directories may be absent. Explicit HOST:LABEL=PATH roots remain supported; missing required roots fail and subagent descendants remain excluded.
4. Malformed candidate Codex tool rows, string-only tool arguments/output, success markers and resolved Git SHA checks keep their current contract.
5. Source classification uses shared precedence: subagent evidence wins, native exec stays unattended even when the originator says Codex Desktop, and IDE is interactive. Unattended is an existing mode label, not proof that no person initiated the session.
6. Existing saved classifications upgrade once from the legacy policy to the shared policy only when native evidence confirms the legacy stored mode. An additive classifier-version column distinguishes legacy rows from new-policy rows; it does not change raw identity, archived rows, capture activation, extraction or memory policy. Missing mode evidence defers upgrading a known legacy mode. Host conflicts and actual known-mode conflicts remain errors, and failed batches cannot partially upgrade classifications.
7. Isolated regression tests, full runtime suite, formatting/check/clippy, release metadata and first-run smoke pass before delivery.

## Distribution

Use the registry dependency. Local validation may use an external Cargo patch while agent-sessions is unpublished. Publication and registry lockfile verification are separate delivery gates. The specification records user-approved ecosystem work; issue/spec/implementation PR links are prepared by the integration owner before remote submission.
