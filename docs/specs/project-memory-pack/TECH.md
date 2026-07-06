# Project Memory Pack Technical Spec

Status: Current contract
Date: 2026-07-02

Tracking:
- Spec/tracking issue: #678

## Existing Implementation Facts

- `memories` rows already carry the fields a pack must preserve: title,
  content, scope, memory_type, state_key (+confidence/reason), confidence,
  provenance/source columns, `valid_from_epoch`/`expires_at_epoch`, owner
  routing fields, and status (`active`/`stale`/`archived` plus governance
  statuses in newer schemas).
- Suppression state lives in `memory_suppressions`; lifecycle semantics are
  invalidate-never-delete (`docs/memory-lifecycle.md`).
- Backup import is whole-DB restore, not selective merge (#145/#171
  hardening history).
- Markdown export for curated memories exists for the A/B curated-file
  condition (#383/#385); it is one-way and not import-parseable.
- Secret redaction runs at capture time (`src/adapter/redaction.rs`).
- #672 defines write-time instruction-pattern quarantine and a source trust
  class vocabulary; this spec adds the `pack` class to it.

## Pack Format

Directory layout (versioned, committed to the consumer repo):

```
<pack-dir>/
  pack.json        # manifest: format_version, project identity, exporter
                   # remem version, memory count, content digest
  memories.jsonl   # one active memory per line, canonical field order
  INDEX.md         # generated human-readable index (type-grouped)
```

- `memories.jsonl` is the source of truth; `INDEX.md` is derived and
  regenerated on export (never parsed on import).
- Canonical serialization: stable field order, sorted by
  `(memory_type, state_key | content_hash)`, LF line endings, no timestamps
  of the export run itself (determinism; same principle as #673). The
  manifest digest covers `memories.jsonl` bytes.
- Exported fields per memory: title, content, memory_type, scope (`project`
  only), state_key + confidence/reason, confidence, created/valid epochs,
  owner intent (`repo`), and content_hash. Local row ids, session ids, raw
  evidence event ids, and the row's local source trust class are NOT exported
  (machine-local identifiers; provenance is summarized as an origin string
  instead). Import assigns the `pack` trust class locally, so canonical bytes
  never contain the trust class that import rewrites.
- `format_version` starts at 1; import rejects unknown major versions with an
  actionable error.

## Export

- Query: active repo-owned startup memories for the resolved local project:
  `owner_scope = 'repo'`, owner key matching the current repository/project
  owner, target project matching the current project where applicable, and
  context class eligible for startup/retrieval context. Legacy rows without
  owner fields may be included only when the startup-context compatibility
  query would include them for the same project. `stale`, expired,
  suppressed, tool/domain/user/workstream/session-owned rows, and other
  non-startup rows are excluded.
- Redaction gate: every content field is re-scanned with the capture-time
  redaction patterns; any hit aborts the export listing the offending memory
  ids (exit non-zero, U-29 — no silent skip-and-continue).
- Determinism test: export twice on an unchanged store, assert byte equality;
  property covered in CI with a fixture store.

## Import

Merge algorithm per pack row, inside one transaction:

1. Remap project identity first: the manifest exporter project remains
   provenance only, while imported rows target the importing checkout's
   resolved local project and repo owner key. Round-trip export uses the
   local project identity in the new manifest but keeps identical
   `memories.jsonl` bytes.
2. Identity: match by
   `(target_owner_scope, target_owner_key, memory_type, state_key)` when a
   state key is present, else by
   `(target_owner_scope, target_owner_key, memory_type, content_hash)`.
3. Skip (report `dedup`) when an identical active local row exists.
4. Skip (report `suppressed`/`invalidated`) when the identity or any policy
   suppression predicate matches local policy: direct memory identity,
   topic/state key, entity, or substring/pattern suppression. Also skip when
   the local identity maps to any inactive local row (`stale`, `archived`,
   `rejected`, `deleted`, `superseded`, invalidated, or equivalent
   governance status) — packs never resurrect local decisions (#381
   semantics).
5. Conflict (same state_key, different content): the local row wins; the pack
   row is recorded as a `pending_review` candidate with
   `source_kind='pack'` so the user can adopt it explicitly. No silent
   overwrite in either direction.
6. Otherwise, insert as an active memory with source trust class `pack` and an
   origin string (`pack:<manifest digest prefix>`), running the #672
   instruction-pattern scan first; quarantined content goes to the review
   inbox, not the store.

`--dry-run` executes the same pipeline against a savepoint and rolls back,
printing the add/dedup/skip/conflict/quarantine report.

## Trust and Injection

- New source trust class `pack` slots below local user-typed and repo-owned
  classes in the #672 vocabulary. Auto-promote gates treat `pack` as
  non-promotable to higher-trust surfaces; injection includes pack memories
  normally but `remem why` and doctor expose the origin.
- Doctor probe: imported-pack count by origin digest, plus quarantine counts
  from pack imports.

## Round-Trip Property

Export from store A → import into empty store B → export from B must be
byte-identical to the first pack (asserted in a fixture test). This pins the
canonical serialization and guarantees no lossy field mapping.

## Interaction with #383 Markdown Export

The curated-file export for the A/B benchmark and this pack share the
rendering layer where practical (one exporter module, two output profiles).
They stay separate commands: the benchmark export optimizes for prompt
ergonomics, the pack for determinism and re-import.

## Phases and Verification

Phase 1: pack format + export + determinism/redaction tests
(`cargo test pack_export`) is implemented for `remem export --project <p>
[--pack <dir>]`; import paths remain disabled.
Phase 2A: import dry-run + merge planner validates `pack.json`/`memories.jsonl`
integrity and reports add/dedup/skip/conflict/quarantine categories without
writing active imported memories (`cargo test pack_import`). Phase 2B adds
active import for safe rows, suppression/invalidation no-resurrection tests,
`pack` trust-class writes, and conflict/quarantine review routing.
Phase 3: round-trip identity fixture, doctor probe, README walkthrough, and
`remem doctor` smoke on a store with an imported pack.

No pack row may enter active startup memory without the #672 scan and `pack`
trust class in place.
