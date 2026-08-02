# Host-Native Memory Data Sources — Technical Contract

Refs #852 / Refs #849. Source spec packet: `specs/GH852/`.

## Modules

| Area | Files |
| --- | --- |
| Import CLI | `src/cli/archive_types.rs` (`ImportAction::CodexMemories`), `src/cli/actions/codex_memory_import.rs` + `discovery.rs` / `parser.rs` / `plan.rs` / `apply.rs` / `tests.rs` |
| Shared external-candidate persistence | `src/memory_candidate/route.rs` (`insert_external_candidate`, `external_candidate_exists`), `src/db/capture.rs` (`ensure_project_row`) |
| Claude native input closure | `src/observe/native.rs`, `src/observe/hook.rs` |
| Bridge status (read-only) | `src/context/claude_memory/ownership.rs` |
| Doctor | `src/doctor/codex_native_memory.rs`, `src/doctor/native_memory.rs` |

## Pipeline

```text
source dir (read-only, default ~/.codex/memories/rollout_summaries)
  -> discovery: no symlinks/subdirs/unknown files; per-file 1 MiB,
     total 64 MiB, 10k file caps (proposal defaults); stable read
     (metadata compared before/after)
  -> parser: codex-rollout-summary/v1 only (header thread_id/updated_at/
     rollout_path/cwd [+ git_branch], RFC3339, absolute paths, non-empty body)
  -> secret boundary: redact_sensitive_text delta on any record blocks the
     whole batch before any hash/candidate persistence
  -> plan: identity sha256(format \0 version \0 body \0 route-key);
     in-batch + route-scoped DB dedup;
     scan_instruction_pattern -> quarantined; else pending_review
  -> dry-run JSON report with plan digest
     or apply (--expect-plan-digest match) -> single transaction ->
        memory_candidates rows with source_trust_class='external_content',
        auto_promote_block_reason set, evidence_event_ids='[]'
```

Since schema v076, cross-process idempotency is owned by the immutable
`external_candidate_identities` ledger rather than mutable candidate text or
topic fields. Its length-prefixed SHA-256 identity binds source kind, memory
type, an optional semantic discriminator, source project, owner scope/key, the
null-tagged target project, topic key, and text. The ledger stores only the text
digest, atomically claims one candidate id, counts later native-import
occurrences without reopening terminal review rows, and retains the original
route identity even if an approved candidate is edited. Digest collisions or
field mismatches fail loudly. Pre-v076 exact rows are adopted deterministically
without deletion; identities carrying a semantic discriminator never adopt an
unverifiable legacy row.
Claude native ingestion uses the same helper with
`source_kind='claude_native'`.

## Invariant mapping (tested)

- B-005/B-006: unknown/subdir/symlink/malformed/concurrent-change fixtures
  fail the batch; source tree byte-identical before/after
  (`codex_memory_import/tests.rs`).
- B-007/B-010: dry-run writes nothing; apply is one transaction; stale plan
  digest rejected at the CLI boundary.
- B-008: second apply and renamed-file re-runs classify as dedup; two
  repositories retain distinct candidates for identical content, while a retry
  in either repository only advances the immutable identity occurrence count.
- B-009/B-018: quarantine and secret fixtures leave zero active memories;
  secret batches persist no candidates or hashes.
- B-011: doctor four-state tests; missing dir exits zero with
  `not_configured`.
- B-017: verified-cwd fixture routes `repo/startup_core`; unverifiable cwd
  routes `tool:codex-cli` / `search_only`.
- B-019: `remem_sessions.md` exclusion, candidate-only ingestion, and error
  propagation tests in `src/observe/native.rs`.

## Deliberate limits

- The format set is closed at `codex-rollout-summary/v1` (fingerprinted on
  codex-cli 0.145.0). New host versions require new PoC evidence and a spec
  update before a new detector version is added.
- `autoMemoryDirectory` takeover machinery (settings writes, receipts,
  delivery block, SessionStart manifest exclusion) is intentionally absent;
  shipping it requires SP852-T1 PoC evidence and fresh human approval per
  `specs/GH852/tasks.md`.
