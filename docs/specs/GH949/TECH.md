# GH949 — SQLite Runtime Tuning Technical Contract

Status: Current contract
Refs: #949, #942

## Ownership

`src/db/pragma.rs` owns parsing and applying SQLite connection policy.
`src/db/core.rs` owns opening database handles and configuring SQLCipher. All
three configured open helpers must construct one `ConnectionPragmas` value
before opening a handle, configure the cipher, and apply the typed pragmas.

No other runtime module should duplicate this pragma set.

## Initialization Order

For every configured connection:

1. Read `REMEM_SQLITE_CACHE_KIB` and `REMEM_SQLITE_SYNCHRONOUS`.
2. Reject invalid or non-Unicode values.
3. Open the requested database handle.
4. Configure SQLCipher.
5. For read-write handles, set and verify `journal_mode=WAL`.
6. Enable foreign keys and set the 5,000 ms busy timeout.
7. Apply the negative cache-size value so SQLite interprets it as KiB.
8. For read-write handles, apply `FULL` or the explicit `NORMAL` override.
9. Force `temp_store=MEMORY`.

Parsing before step 3 prevents an invalid override from creating an empty
database as a side effect.

## Typed Values

The cache limit is parsed into a bounded `i64`; it is never interpolated from
untrusted text. Synchronous policy is a closed enum with only `Full` and
`Normal`. Pragmas use rusqlite's typed update helpers rather than a
configuration-derived SQL batch.

The WAL update uses `pragma_update_and_check` and rejects any response other
than `wal`. This ensures the later synchronous policy is applied only after
the required journal mode is active.

## Read-Only Behavior

Read-only connections do not attempt to change journal mode or synchronous
policy. They still apply connection-local foreign-key, busy-timeout, cache, and
temporary-storage settings.

## Verification Design

Unit tests cover pure parsing boundaries and closed-enum behavior. Runtime
tests open real create, existing read-write, and read-only connections and
query the effective pragma values. Environment-mutating tests hold the shared
runtime configuration test lock and restore previous values.

On Unix, a non-Unicode environment fixture proves configuration is rejected
before the database path exists.

`tests/search_latency_benchmark.rs` owns the opt-in encrypted release A/B:

- 2,000 KiB versus 65,536 KiB cache targets on the same seeded corpus;
- `FULL` default versus explicit `NORMAL` write latency;
- abrupt process exit recovery for committed and uncommitted WAL transactions.

The harness is ignored by default because it is intentionally large and
timing-sensitive. Its measurements are engineering evidence, not a universal
latency claim.

## Packaging Hygiene

`.remem/` is a runtime data directory. It belongs in `.gitignore` and the Cargo
package exclusion list. GH949 removes the accidentally committed log and lock
files; existing repository history remains intact.

## Deferred Work

`PRAGMA optimize` needs a long-lived maintenance owner and an explicit cadence.
Short-lived hook and worker-loop connections are not suitable owners, so this
contract does not add it.
