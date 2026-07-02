# Project Memory Pack Technical Spec

Status: Current contract
Date: 2026-07-02

Tracking:
- Spec/tracking issue: #678

## Existing Implementation Facts

- `memories` rows already carry the fields a pack must preserve: scope,
  memory_type, state_key (+confidence/reason), confidence, provenance/source
  columns, `valid_from_epoch`/`expires_at_epoch`, and status
  (`active`/`stale`).
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
- Exported fields per memory: content, memory_type, scope (`project` only),
  state_key + confidence/reason, confidence, created/valid epochs, source
  trust class, content_hash. Local row ids, session ids, and raw evidence
  event ids are NOT exported (machine-local identifiers; provenance is
  summarized as an origin string instead).
- `format_version` starts at 1; import rejects unknown major versions with an
  actionable error.

## Export

- Query: active, project-scoped memories for the resolved project id;
  `stale`, expired, suppressed, and user-scoped rows are excluded.
- Redaction gate: every content field is re-scanned with the capture-time
  redaction patterns; any hit aborts the export listing the offending memory
  ids (exit non-zero, U-29 — no silent skip-and-continue).
- Determinism test: export twice on an unchanged store, assert byte equality;
  property covered in CI with a fixture store.

## Import

Merge algorithm per pack row, inside one transaction:

1. Identity: match by `state_key` when present, else by
   `(memory_type, content_hash)`.
2. Skip (report `dedup`) when an identical active local row exists.
3. Skip (report `suppressed`/`invalidated`) when the identity matches a
   locally suppressed or stale/invalidated memory — packs never resurrect
   local decisions (#381 semantics).
4. Conflict (same state_key, different content): the local row wins; the pack
   row is recorded as a `pending_review` candidate with
   `source_kind='pack'` so the user can adopt it explicitly. No silent
   overwrite in either direction.
5. Otherwise insert as an active memory with source trust class `pack` and an
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
(`cargo test export`).
Phase 2: import + merge/dry-run + round-trip and resurrection-safety tests
(`cargo test import`).
Phase 3: trust-class wiring (#672 dependency), doctor probe, README
walkthrough; `remem doctor` smoke on a store with an imported pack.

Phase 3 depends on the #672 trust-class schema landing first; Phases 1–2 can
ship with the origin string recorded and the trust class defaulting to the
most restrictive treatment.
