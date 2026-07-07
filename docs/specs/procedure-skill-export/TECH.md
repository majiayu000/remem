# Procedure Skill Export Technical Spec

Status: Current contract
Date: 2026-07-02

Tracking:
- Spec/tracking issue: #680

## Existing Implementation Facts

- Promoted procedures persist as `memory_type='procedure'` rows with project
  scope, keeping branch, evidence event ids, files touched, command, reuse
  condition, and confidence derived from verified run count
  (`docs/procedural-memory.md`).
- Promotion gates: ≥2 successful verified runs, raw source event ids per run,
  verification freshness window, matching project/branch/workflow key/command.
- GH680 Phase 1 adds `remem procedures list`; export remains unimplemented
  until the later phases below.
- Memory lifecycle marks superseded/wrong rows `stale`
  (`docs/memory-lifecycle.md`); procedures carry no TTL by default.

## Design

### 1. CLI surface

New `remem procedures` namespace:

- `list [--project <p>] [--json]`: promoted procedures with maturity columns
  (verified runs, last verification epoch, branch, files touched count,
  confidence). Reads existing rows; no schema change needed for v1 listing.
- `export <memory_id> --format claude-skill|codex-prompt|runbook-md
  [--out <dir>]`: renders one procedure to one draft file. Refuses (exit
  non-zero, actionable message) unless the source row has
  `status = 'active'`, `memory_type = 'procedure'`, the trace-promotion
  evidence required by `docs/procedural-memory.md` (at least two successful
  verified runs and source evidence event ids), and is inside the verification
  freshness window. Any non-active status (`stale`, `rejected`, `deleted`,
  `archived`, `superseded`, or future inactive statuses) is ineligible.
  Export eligibility reuses the promotion-gate freshness predicate rather
  than defining a second one.

MCP: read-only `procedures` listing/status tooling may follow; export stays
CLI-only in v1. MCP must not render or write draft artifacts because writing
files from MCP expands the attack surface without a driving use case.

### 2. Render templates

One template module, three output profiles, rendering stored fields verbatim:

- `claude-skill`: `SKILL.md`-shaped file — frontmatter name derived from the
  workflow key, frontmatter description derived from the workflow key plus a
  bounded sanitized summary of the stored reuse condition/preconditions, body
  sections for when-to-use (reuse condition), steps (command), preconditions
  (project, branch), files touched.
- `codex-prompt`: prompt Markdown with the same sections in Codex prompt
  conventions.
- `runbook-md`: plain runbook with a verification-evidence section.

Every profile emits:

- header marker: `<!-- remem-draft: procedure export, review before commit -->`
  plus a human-visible "Draft — review before committing" line;
- provenance footer: source memory id, evidence event ids, verified run
  count, generation date, remem version.

For `claude-skill`, `SKILL.md` YAML frontmatter is first-line content. The
draft marker and human-visible warning are emitted immediately after the
closing `---` delimiter so the file remains loadable by Claude skill tooling.
The `description` field is the skill activation hint, so it must include the
bounded reuse-condition summary and must be pinned by snapshot tests to avoid
regressing to workflow-key-only descriptions.
For `codex-prompt` and `runbook-md`, the draft marker may be the first
nonblank content.

Before any template renders or writer opens a target path, the exporter
re-scans every field that will be rendered into the draft: workflow key,
command/content, reuse condition, project/branch preconditions, files
touched, evidence ids, generated title/name/description, and provenance
strings. Secret detection uses the concrete capture redaction contract in
`src/adapter/redaction.rs`, including the `redact_hook_payload_preview`
option-argument, URL-userinfo, cookie, authorization, and sensitive-key tests.
Instruction-pattern detection uses the `memory-poisoning-defense` pattern set.
A positive match aborts export with an explicit error and leaves no partial
file or registry row; v1 does not silently redact and continue because a
reviewed draft must not hide that its source procedure is unsafe to publish.

Snapshot tests pin all three renderings for a fixture procedure.

### 3. Write-path guard

The export writer lives in a `procedures::export` module whose only caller is
the CLI action. Enforcement is layered:

- module visibility: the writer is not `pub` beyond the CLI action path;
- runtime guard: the writer asserts it is running in a CLI invocation context
  (not worker/dream/hook entrypoints) and returns an error otherwise;
- negative test: a compile-fail or runtime test proves worker/dream/hook
  paths cannot invoke the writer.

Default `--out` is `./remem-drafts/` (created on demand). The writer refuses
paths that resolve into high-context locations (`.claude/`, `.codex/`,
`AGENTS.md`, `CLAUDE.md`, repo-local `skills/`, repo-local `.agents/skills/`,
configured skill roots, discovered skill roots, and nested variants) even
when passed explicitly, regardless of export format or final file name. Moving
a reviewed draft there is deliberately a human `mv`/`git` action (SEC-13
surface; path check is by canonicalized prefix/basename, SEC-07).

The writer never silently replaces an existing draft path:

- absent target: write the draft and create a registry row;
- existing target with no registry match or a content digest different from
  the recorded export digest: refuse with an explicit error because the draft
  may have been reviewed or user-edited;
- existing unmodified remem-generated target: overwrite only when the user
  passes an explicit `--overwrite-generated` flag.

The error message points to `--out <new-dir>` or a renamed target as the safe
path for a new draft. There is no flag that overwrites a reviewed/user-edited
draft in place.

### 4. Export registry and doctor back-link

New table `procedure_exports` (migration): memory_id, format, relative output
path, content digest at export, source procedure digest/version at export,
source `updated_at` at export, exported_at_epoch, remem version. On each doctor
run:

- flag rows whose source memory is now any non-active status or expired
  (`export drifted: source procedure inactive`);
- flag rows whose source verification freshness lapsed;
- flag rows whose source memory is still active but whose source digest,
  version, or `updated_at` no longer matches the exported source snapshot
  (`export drifted: source procedure changed after export`);
- report count of exports per project.

Doctor does not read or hash the exported files themselves (they may have
been legitimately edited after review); drift is defined against the source
procedure state and the source snapshot captured at export time only.

### 5. Interaction with #678

Pack export and procedure export share the deterministic-rendering utilities
(stable ordering, LF endings, no run-timestamps in body except the labeled
provenance footer) but remain separate commands and formats: packs are
re-importable data, procedure drafts are human-facing artifacts and are never
re-imported.

## Phases and Verification

Phase 1: `procedures list` + fixture (`cargo test procedures`). Shipped by
GH680 Phase 1 with read-only listing and maturity metadata; export remains
future work.
Phase 2a: export eligibility core. The export source loader reuses the same
fresh procedure verification evidence as `procedures list` and rejects
non-procedure, inactive, expired, suppressed, superseded, or insufficiently
verified rows before any render/write path can be added.
Phase 2b: export command, templates, snapshot tests, write-path guard negative
test.
Phase 3: `procedure_exports` migration + doctor probe; docs update replacing
the procedural-memory.md non-goal paragraph with this contract.

Verify per phase: `cargo fmt --check && cargo check && cargo test`, plus
`remem doctor` smoke on a fixture store with one drifted export.
