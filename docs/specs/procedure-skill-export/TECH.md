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
- No `remem procedures` CLI namespace exists; procedures are reachable only
  through generic search/timeline surfaces.
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
  non-zero, actionable message) when the procedure is stale, expired, or
  outside the verification freshness window — export eligibility reuses the
  promotion-gate freshness predicate rather than defining a second one.

MCP: read-only `procedures` listing/status tooling may follow; export stays
CLI-only in v1. MCP must not render or write draft artifacts because writing
files from MCP expands the attack surface without a driving use case.

### 2. Render templates

One template module, three output profiles, rendering stored fields verbatim:

- `claude-skill`: `SKILL.md`-shaped file — frontmatter name/description
  derived from the workflow key, body sections for when-to-use (reuse
  condition), steps (command), preconditions (project, branch), files
  touched.
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
For `codex-prompt` and `runbook-md`, the draft marker may be the first
nonblank content.

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
`AGENTS.md`, `CLAUDE.md` and nested variants) even when passed explicitly —
moving a reviewed draft there is deliberately a human `mv`/`git` action
(SEC-13 surface; path check is by canonicalized prefix/basename, SEC-07).

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

- flag rows whose source memory is now `stale`/expired
  (`export drifted: source procedure invalidated`);
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

Phase 1: `procedures list` + fixture (`cargo test procedures`).
Phase 2: export command, templates, snapshot tests, write-path guard negative
test.
Phase 3: `procedure_exports` migration + doctor probe; docs update replacing
the procedural-memory.md non-goal paragraph with this contract.

Verify per phase: `cargo fmt --check && cargo check && cargo test`, plus
`remem doctor` smoke on a fixture store with one drifted export.
