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

Runtime preflight and isolated smoke are in progress. Dependency validation uses
an external Cargo patch; registry publication and lockfile verification are
pending and must finish before claiming a distributable release.
