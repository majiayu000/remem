# Migration Discipline

A remem SQLite migration has one concern.

Schema evolution (new tables, columns, indexes, triggers) may backfill the
table it just changed. Historical business-data corrections belong in their
own migration or a traceable operator command. Do not mix those in one file.

`scripts/ci/check_migration_concerns.py` rejects a new migration that both
changes schema and runs upgrade-time `UPDATE`/`DELETE` against a table it did
not create or alter. Trigger bodies are treated as schema, not upgrade DML.

## Allowed

- `ALTER TABLE memories ADD COLUMN ...` plus `UPDATE memories SET` that column
- A rebuild that `CREATE`s `foo_new`, copies rows, drops `foo`, and renames
- A data-only file such as `v011_reprice_ai_usage_events.sql`

## Not allowed

v083 is the example: it added retrieval-enrichment state and also rewrote
`ai_usage_events` GPT-5.6 pricing. The pricing rewrite should have been a
separate file, like v011.

## Historical allowlist

A few shipped files still cross a second table. They stay listed in
`HISTORICAL_CROSS_TABLE_REWRITES` and must not gain more rewrite targets.
When a listed extra rewrite disappears, remove it from the allowlist.
