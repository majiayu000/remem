# Hook and Projection Redaction Product Spec

Status: Current contract
Date: 2026-09-13

Tracking:
- Spec/tracking issue: #1083
- Implementation PR: #1084

## Problem

Hook capture, preference/import classification, and MCP/CLI workstream
projections share redaction helpers. Projection needs aggressive option and
inline-assignment scrubbing so credentials do not leave through listings, but
the shared capture path must stay fail-closed without rewriting benign prose,
documentation tokens such as `-username`, or canonical project path
separators.

## Goals

- Keep capture/import `redact_sensitive_text` free of sensitive-option and
  attached short-option heuristics.
- Apply projection-only heuristics (`redact_projected_sensitive_text`,
  hook payload preview) for inline assignments, space-separated options, and
  attached short options such as `-ualice:pw` and `-u"alice:correct horse"`.
- Preserve original whitespace separators when rebuilding redacted text so
  projected project identifiers remain correlatable.
- Limit filesystem-path exemptions to projected project identifiers.

## Non-Goals

- A second redaction crate or host-specific scrubbers.
- Changing workstream identity matching (canonical values stay unredacted
  until output projection).

## Behavior

| Surface | Entry | Sensitive options / attached `-u` | Inline short assignments | Path exemption |
|---|---|---|---|---|
| Capture / preference / import body | `redact_sensitive_text` | off | off | off |
| Hook payload preview | `redact_hook_payload_preview` | on | on | off |
| MCP/CLI workstream text fields | `redact_projected_sensitive_text` | on | on | off |
| MCP/CLI project identifiers | `redact_projected_project_text` | on | on | on |

## Done when

- Attached quoted credentials such as `curl -u"alice:correct horse"` redact
  without leaking the remainder.
- Shared `redact_sensitive_text` leaves `-username` and similar benign tokens
  unchanged.
- Projected project paths keep original internal whitespace.
- `python3 scripts/ci/check_file_size.py` passes with each redaction source
  file under the 800-line guard.
- Focused adapter redaction regression tests pass.
