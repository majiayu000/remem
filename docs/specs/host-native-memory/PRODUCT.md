# Host-Native Memory Data Sources — Product Contract

Refs #852 / Refs #849. Source spec packet: `specs/GH852/`. Evidence:
`docs/research/gh852-host-native-memory-poc.md`.

## Shipped behavior

- `remem import codex-memories` imports Codex CLI rollout-summary memories
  one-way and read-only. Records are untrusted external content: they enter
  the candidate review queue only (`pending_review` / `quarantined`), are
  never auto-promoted, and never reach active memories from this path. The
  Codex source tree is never modified.
- Dry-run and apply share one planning function. Apply requires
  `--expect-plan-digest` from a prior dry-run and refuses to commit when the
  frozen plan no longer matches. A batch with any secret-like, malformed,
  unknown, or unstable file fails entirely; there is no partial import.
- Idempotency is content-based (`sha256(format || version || content ||
  route)`), so re-runs and host file renames do not create duplicates.
- Records with verifiable workspace evidence (record `cwd` resolving to an
  existing local directory) route to that project; everything else lands in
  the Codex tool-owned `search_only` review queue. The import command's own
  cwd never influences routing.
- `remem doctor` reports Codex native memory source state
  (`not_configured` / `ready` / `unreadable` / `unsupported_format`) without
  printing memory bodies, and reports the Claude native-bridge state.
- Claude native topic files (`~/.claude/projects/<slug>/memory/*.md`) are
  ingested as external-content review candidates, never directly into active
  memories; remem's own `remem_sessions.md` delivery file is excluded
  (no self-ingestion); ingestion failures are error-level and propagate to
  the hook exit status.

## Explicit non-behavior (fail-closed defaults)

- The Claude `autoMemoryDirectory` delivery bridge is **no-go / `hook_only`**
  pending isolated real-host PoC evidence (SP852-T1) and a recorded per-host
  human go decision. remem does not write `autoMemoryDirectory`, take over
  any directory, or emit a `MEMORY.md` delivery block.
- Codex `hooks.json` capture is unchanged; GH-852 produced an audit only.
- Only the PoC-fingerprinted `codex-rollout-summary/v1` format is accepted;
  unknown host versions fail visibly instead of being loosely parsed.
