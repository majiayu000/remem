# Tech Spec

## Linked Issue

GH-680

## Product Spec

Link to `product.md`.

## Accepted Contract

The authoritative technical contract is
`docs/specs/procedure-skill-export/TECH.md`.

This SpecRail packet reflects the existing #680 contract and keeps
implementation behind the normal SpecRail readiness and spec-approval gates.

## Codebase Context

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| Procedural memory | `docs/procedural-memory.md`, `src/memory/procedure/` | Promotion gates create procedure memories after repeated verified runs. | Export eligibility reuses these gates. |
| CLI | `src/cli/` | No `remem procedures` namespace exists. | New list/export commands live here. |
| Memory lifecycle | `src/memory/lifecycle/` | Source procedure can become stale, archived, or superseded. | Export registry must detect drift. |
| Redaction/security | `src/adapter/redaction.rs`, GH-672 pattern set | Sensitive or instruction-like procedure text must not be written into drafts. | Export scans every rendered field before file write. |
| Doctor | `src/doctor/` | Reports health and actionable maintenance. | Doctor flags drifted procedure exports. |
| High-context files | `AGENTS.md`, `CLAUDE.md`, skill roots | Auto-loaded instruction surfaces are high risk. | Export writer must refuse these paths. |

## Design Rules

- Export is CLI-only in v1; MCP may list but cannot write draft artifacts.
- No background job may write procedure drafts.
- The writer refuses high-context output paths even when explicitly provided.
- Existing user-edited draft paths are never silently overwritten.
- Rendered fields are scanned before any target file is opened.

## Proposed Design

1. Add `remem procedures list` to show active procedure memories with maturity
   metadata.
2. Add `remem procedures export <memory_id>` with formats
   `claude-skill`, `codex-prompt`, and `runbook-md`.
3. Implement one template module with three output profiles and snapshot
   tests.
4. Keep Claude skill YAML frontmatter as first-line content; emit the draft
   marker after the closing delimiter.
5. Add scan-before-write checks using redaction and the GH-672 instruction
   pattern set.
6. Add layered writer protection: module visibility, CLI invocation guard, and
   high-context path rejection.
7. Add `procedure_exports` registry with source digest/version snapshot and
   doctor drift reporting.
8. Update `docs/procedural-memory.md` to replace the old export non-goal with
   the review-gated contract.

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| Mature procedure listing | CLI query | list fixture and JSON tests |
| Three export formats | template module | snapshot tests |
| Claude skill loadability | claude-skill template | frontmatter-first snapshot |
| Eligibility checks | export action | non-active/non-procedure/insufficient evidence tests |
| Safe writes | writer guard | high-context path and overwrite tests |
| No background writes | module/guard | worker/dream/hook negative tests |
| Drift reporting | registry + doctor | source changed/inactive/stale tests |

## Data Flow

Procedure memory row -> eligibility and freshness checks -> scan all rendered
fields -> template render -> safe writer -> `procedure_exports` registry.
Doctor compares registry source snapshot to current source procedure state and
reports drift without hashing user-edited output files.

## Risks

- Security: procedure text can become high-context agent instructions; scan and
  path refusal must fail closed.
- Compatibility: draft format snapshots become user-facing and should change
  only intentionally.
- UX: refusal to write directly into skill roots is deliberate but must produce
  actionable output.

## Test Plan

- [x] CLI parsing and list output tests.
- [x] Snapshot tests for all three export formats.
- [x] Eligibility, scan, overwrite, and high-context path rejection tests.
- [x] Negative worker/dream/hook reachability test.
- [x] `procedure_exports` migration and doctor drift tests.
- [x] Documentation update verification.
- [x] `cargo fmt --check`, `cargo check`, focused tests, and `cargo test`
      before merge readiness.

## Rollback Plan

Disable the export command and doctor probe. Registry rows can remain as audit
history. Draft files are user-controlled artifacts and are not mutated by
rollback.
