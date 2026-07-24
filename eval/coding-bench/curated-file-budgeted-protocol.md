# `curated_file_budgeted` Curator Protocol

Primary human baseline for issue #931. Unlike the diagnostic
`curated_file_expert` condition (unbudgeted, built from gold evidence), this
condition measures what a realistic, target-blind human curator produces under
a fixed maintenance budget.

## Rules

1. **Chronological, target-blind.** The curator sees history episodes strictly
   in chronological order and never sees the target task, its prompt, its
   tests, or its gold memory facts. The `MEMORY.md` is frozen before the
   target task is revealed.
2. **Per-session time budget.** Each history episode grants at most
   `budget.minutes_per_session` minutes of maintenance time. Unused time does
   not roll over.
3. **Character cap.** `MEMORY.md` must never exceed `budget.max_chars`
   characters, measured after every session and at freeze time.
4. **Edit accounting.** After each session the curator records: minutes spent,
   edit count, deletion count, conflict-resolution count, and the file size in
   characters. These feed the `human maintenance minutes / 100 sessions`
   metric.
5. **Freeze and hash.** At freeze time the artifact records the final
   character count and the SHA-256 of the final `MEMORY.md`. The run harness
   must verify the file it injects matches this hash.
6. **Run-time surface.** During the target task run, the final `MEMORY.md` is
   the only memory surface: no remem hooks, no MCP, no host-native memory.

## Artifact

Every `curated_file_budgeted` run must attach a curator log conforming to
`eval/coding-bench/schemas/curator-log.schema.json`. Example:
`eval/coding-bench/examples/curator-log.example.json`.

Beyond the schema, the validator (`eval/coding-bench/validate_schemas.py`)
enforces cross-field rules:

- every `sessions[].minutes_spent <= budget.minutes_per_session`;
- every `sessions[].chars_after <= budget.max_chars`;
- `final_char_count <= budget.max_chars`;
- `totals.maintenance_minutes` equals the sum of `sessions[].minutes_spent`;
- `totals.update_count`, `totals.deletion_count`, and
  `totals.conflict_resolution_count` equal the corresponding session sums.

## Default v1 budget

- `minutes_per_session`: 3 minutes;
- `max_chars`: 4000 characters.

The budget is part of the pre-registered benchmark configuration and must be
locked before the first official run (see the claim gate in
`eval/claims/registry.json`).
