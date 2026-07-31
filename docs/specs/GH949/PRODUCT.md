# GH949 — SQLite Runtime Tuning Product Contract

Status: Current contract
Refs: #949, #942

## Problem

Every remem hook, CLI command, worker iteration, MCP request, and API operation
opens one or more SQLite connections. Before GH949 those entry points repeated
part of the connection setup and left SQLite's small default page cache in
place. SQLCipher disables memory-mapped I/O, so cache misses on a large store
require both file I/O and page decryption.

The optimization must not make successfully acknowledged memories less durable
without an explicit operator choice. It also must not introduce configuration
that silently falls back when malformed.

## User Outcomes

- Every configured connection receives the same applicable SQLite policy.
- The default cache target is 65,536 KiB per connection.
- Read-write connections use WAL and retain `synchronous=FULL` by default.
- Operators may explicitly choose `synchronous=NORMAL` when they accept that a
  system crash or power loss can lose recent acknowledged commits.
- Temporary SQLite data remains in memory so SQLCipher users do not get
  unencrypted temporary files.
- Invalid configuration prevents the connection from opening and names the
  offending environment variable.
- The README documents defaults, units, accepted values, memory multiplication,
  and the durability tradeoff.

## Configuration Contract

`REMEM_SQLITE_CACHE_KIB` is optional. When present and non-empty it must be a
base-10 integer from `1` through `1048576`, inclusive. The value is a
per-connection KiB cache target, not a process-wide allocation.

`REMEM_SQLITE_SYNCHRONOUS` is optional. It accepts only `full` or `normal`,
case-insensitively. Empty or absent selects `full`.

Non-Unicode values are invalid on platforms that permit them. Invalid values
must fail before a new database file is created.

## Durability and Security

`FULL` is the default because remem reports hook and API writes as successful
after SQLite commits them. A later power loss must not silently discard those
acknowledged memories merely to reduce write latency.

`NORMAL` is an opt-in WAL policy. It preserves database consistency and
application-process crash recovery, but recent committed transactions can be
lost after a system crash or power loss.

`temp_store=MEMORY` is not configurable in this contract. SQLCipher does not
guarantee encryption for file-backed temporary storage.

## Scope

In scope:

- configured create, existing read-write, and read-only connection paths;
- cache, synchronous, temp-store, WAL, foreign-key, and busy-timeout policy;
- strict environment parsing;
- user documentation and reproducible latency evidence.

Out of scope:

- changing SQLCipher key handling;
- `PRAGMA optimize` lifecycle scheduling;
- changing database schema or migration behavior;
- automatic selection of `NORMAL`.

## Acceptance

- Focused tests read back effective pragmas from real connections.
- The default read-write path reports SQLite synchronous value `2` (`FULL`).
- Explicit `normal` reports value `1`.
- Invalid, out-of-range, and non-Unicode overrides fail closed.
- Read-only connections apply connection-local pragmas without requiring
  database writes.
- An ignored encrypted release-mode A/B harness compares the old 2,000 KiB
  cache target with the new default and compares `FULL` with opt-in `NORMAL`.
- `.remem` runtime files are ignored and excluded from crate packages.
