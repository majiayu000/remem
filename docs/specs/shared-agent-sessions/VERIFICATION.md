# Migration verification

## Private snapshot parity

2026-09-25, original projector from remem 7f4e144f versus shared-library adapter
(core a88ed07). Deterministically selected every eighth sorted Codex JSONL file
and all Claude JSONL files from the pre-existing frozen local snapshot.

- 952 files, 290,572 physical records.
- 33,898 projected user/assistant messages.
- Zero malformed JSON records; zero role/text/timestamp differences.
- Temporary comparison harness removed after execution. No transcript contents
  or original paths were added to Git. Comparison ran read-only, without a DB.
- This is sampled projection parity, not a claim about every Codex file or
  new source-classification labels. Synthetic tests separately cover captured
  byte boundaries, ordinal/empty-row preservation, UTF-8 errors and discovery.

## Expected classification changes

Native `source` now participates in Codex mode projection. `exec` plus Desktop
maps to the existing unattended label; native subagent evidence wins over
originator, and IDE maps to interactive. This label does not establish whether
a human initiated a run. First session ID/cwd/branch and raw identity stay local.

## Delivery gates

Runtime full preflight is in progress. Focused tests passed: 8 raw projection/
framing, 42 ingestion/identity, 23 Git evidence, 28 raw archive, 25 reconciliation,
and 2 isolated CLI root tests (128 total). The CLI tests check both valid native
root overrides, both empty override errors, and no database creation on invalid
configuration.

The consumer compiled against registry agent-sessions 0.2.0 with checksum
`741368addca6a3758a911dc5b0871df029863c67d2d2008788662586ed4748c3`.
No agent-sessions path patch remains. Registry resolution is verified locally;
remote CI, merge and the Remem release are separate gates.

## Validation environment

The repository public-surface guard requires Rust 1.97.0; it was installed and
the prior unset rustup default was retained. The historical security benchmark
producer commit `7b3c13c7795eafc4ebc55402848552986ed13ca9` was fetched to resolve
its source-tree attestation. An ignored target/doc link exposes the shared Cargo
rustdoc output to the existing guard. The unchanged REST declaration scan took
about nine minutes on this machine.

The spec-only commit dbf45024 full local preflight passed every gate except the
production test run (4000 passed, 2 failed, 1 ignored). The runner had imposed one
shared REMEM_CONFIG across per-test data directories, allowing an enabled rule
setting to reach two disabled-config cases. A controlled reproduction failed
with that shared setting and passed both cases when REMEM_CONFIG was unset.
The runner now clears inherited REMEM settings and isolates HOME and data while
letting each fixture own its configuration. The original full exit 1 is retained
as evidence; it is not reported as a green run. Fresh remote CI verifies the
spec branch; the complete local suite will run on the implementation branch.
