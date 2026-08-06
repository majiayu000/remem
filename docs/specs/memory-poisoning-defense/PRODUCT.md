# Memory Poisoning Defense Product Spec

Status: Current contract
Date: 2026-07-02

Tracking:
- Spec/tracking issue: #672
- Related: #377 (injection accountability, closed), #383 (usage feedback),
  #969 (Dream generated-output boundary), #991 (production-path security eval)

## Problem

remem injects memory content into shell-capable coding agents on every
session. Memories are extracted from captured events that include tool output,
fetched web content, and file contents — surfaces an attacker can influence. A
memory whose content contains instruction-like text is persistent prompt
injection: it re-enters the agent's context every session until someone
notices.

This threat class is measured and named: MINJA (arXiv 2503.03704) reports
high injection success against memory-based agents through query-only
interactions, and OWASP's Agentic AI Top 10 (2026) lists memory poisoning as
ASI06. remem today screens secrets (redaction) but never screens memory
content for instructions, and treats all capture sources as equally
trustworthy at promotion time.

## Goals

- Stop instruction-like payloads at write time: quarantine, never promote.
- Record a source trust class on every memory so promotion can require
  trustworthy provenance.
- Add defense-in-depth at injection time: flagged content never reaches the
  rendered context block, and drops are loud, not silent.
- Keep the review inbox the single escape hatch for false positives.
- Apply the same generated-surface boundary to Dream consolidation before it
  can create a replacement memory or supersede source memories.

## Non-Goals

- Semantic or embedding-based intent scoring of every capture (a heavier
  future layer; this spec is pattern + provenance).
- Network calls or LLM calls in the injection path.
- A general secrets scanner (redaction already exists and stays unchanged).
- Retroactive classification of the entire existing memory store. The scoped
  Dream stock backfill is an explicit operator command, not automatic migration
  work; unrelated historical memories remain outside this contract.

## Behavior Invariants

1. Every new memory candidate carries a source trust class derived from its
   captured provenance: `user_prompt` > `repo_file` > `local_tool_output` >
   `external_content`.
2. Candidate content matching an instruction-injection pattern is inserted as
   `quarantined`, never `pending_review` or auto-promoted, and the matched
   pattern is stored with the candidate.
3. Auto-promotion requires trust class at or above a configurable floor
   (default: `local_tool_output`); `external_content` never auto-promotes.
4. The context renderer re-scans items at injection time and drops any match,
   logging at error level with memory id and matched pattern; the drop is
   visible in doctor. No silent degradation.
5. A quarantined candidate can be approved only through explicit review; the
   approval records that the matched pattern was acknowledged.
6. Pattern-scan behavior is deterministic and versioned: the same content and
   pattern-set version always yields the same verdict.
7. Dream scans generated topic key, type, title, content, no-merge reason, and
   conflict reason before any persistence. It also scans the canonical
   title/content render so a pattern split across fields cannot bypass the
   boundary. A hit atomically creates or reuses a quarantined external-content
   candidate, binds it to the exact source-memory cluster, and records a
   no-merge decision; it never creates or supersedes an active memory or writes
   poisoned conflict metadata.
8. Dream approval is bound to the current candidate/artifact versions and a
   cryptographic snapshot of each source memory's version and canonical
   payload. A stale token fails closed, promotion must supersede exactly the
   reviewed current set, and a newer semantic decision invalidates the prior
   review without erasing its artifact. No-merge and conflict reasons are
   dismiss-only diagnostics and can never be promoted as memories.
9. Every Dream write rechecks current TTL, state-key ownership, suppression,
   and the exact source snapshot inside one immediate transaction. Clean model
   output remains `external_content`, and a merge can reuse only a reviewed
   cluster member—not an unrelated state-key or semantic-dedup target.
10. Any public artifact that reports poisoning-defense policy counts identifies
    its verification path and measurement source. The `remem_default`
    adversarial-policy condition runs capture -> observation extraction ->
    candidate governance -> promotion and reads counts from the resulting
    database. Direct-memory fixture insertion remains a named retrieval
    baseline and cannot be presented as production-path evidence.

## Acceptance Criteria

- [ ] Poisoned-event fixture: seeded captured events containing override
      phrases, "run the following" imperatives, and authority claims produce
      zero rendered context items; each block is logged with pattern and
      provenance.
- [ ] Schema carries source trust class; the auto-promote gate consumes it,
      covered by unit tests for each trust class boundary.
- [ ] Doctor shows quarantine count, pattern-set version, and last injection
      drop; quarantined items are listable and reviewable.
- [ ] False-positive path: approving a quarantined memory works and is
      recorded; the approved memory then renders normally.
- [ ] Poisoned Dream fixtures, including split title/content and poisoned
      no-merge/conflict reasons, produce review candidates and cluster-bound
      audit artifacts while every source memory remains active and no generated
      payload reaches active search/context or conflict metadata.
- [ ] Dream review requires a current provenance token; stale/mismatched tokens
      and any out-of-cluster supersede target roll back without lifecycle,
      review, audit, or idempotency-ledger mutation.
- [ ] Clean Dream decisions reject expired, non-current, suppressed, changed,
      or cluster-external targets with zero generated memory, operation, edge,
      candidate, or raw diagnostic leakage; successful output is external trust.
- [ ] Candidate API and CLI output redact secrets and neutralize terminal
      controls across every Dream-generated field without weakening the
      internal digest or exact-promotion checks.
- [ ] Adversarial-policy run artifacts identify the production pipeline,
      production source-scanner configuration, generated-surface verdict, and
      database-measured active/reviewable/summary-input counts. A regression
      that replaces those measurements with fixture-derived constants fails.

## Edge Cases

- Legitimate memories that quote attack strings (for example a lesson about
  prompt injection itself): quarantine fires, review approves; the approval
  acknowledgement exists exactly for this case.
- Mixed provenance (one candidate supported by both a user prompt and web
  content): the lowest trust class among supporting evidence wins.
- Existing memories created before this feature have no trust class; they are
  treated as `local_tool_output` by default and are not retro-quarantined by
  the migration. The explicit GH-990 command is the scoped exception for
  identifiable pre-v076 Dream-merged stock.
- Pattern-set updates: raising the pattern-set version re-scans only at
  injection time; it does not bulk-rewrite stored rows.

## Rollout Notes

Ship scan-and-quarantine on by default (it only affects new candidates), with
the injection-time re-scan behind a config flag for one release to measure the
false-positive rate before defaulting it on.

### v076 forward boundary + #990 stock backfill

The `v076_dream_poisoning_quarantine` migration is pure DDL. It creates
`dream_quarantine_artifacts`, `external_candidate_identities`, and
`external_candidate_recurrences` with their triggers and indexes, and does not
issue any `UPDATE` or `INSERT ... SELECT` against existing rows.

The pre-v076 stock — memories Dream merged before the upgrade, identifiable by
`memories.session_id = 'dream' AND status = 'active' AND source_trust_class =
'local_tool_output'` — is handled by the explicit `remem dream-backfill`
command (#990), not by a migration:

- Dry-run by default; `--apply` is required to write.
- Every stock row is re-scanned with the same generated-surface scanner and
  calling convention as the forward path.
- A hit is retired (`status='archived'`), stamped `external_content`, and
  bound to a quarantine artifact plus review candidate through the existing
  ledger (v077 `backfill_memory_id`); approving the candidate restores that
  exact memory in place, rejecting leaves it retired.
- A clean row only has its trust class backfilled to `external_content`,
  matching what the forward path stamps on new merges; recency signals are
  left untouched.

The dry-run report exposes a deterministic `plan_digest`. An apply can require
that digest with `--expect-plan-digest`; regardless of that optional operator
check, the write path re-plans inside one immediate transaction and compares
the complete ordered plan plus exact row snapshots before any artifact or
trust-class write. Drift aborts atomically. A completed command produces no
second stock plan, and the backfill binding has a unique partial index so one
retired memory cannot acquire duplicate irreversible artifacts.

With #990 landed, the Dream poisoning defense is closed for both new writes
and the pre-v076 stock. Re-running the backfill is idempotent: quarantined
rows are archived and backfilled rows change trust class, so a second run
finds no stock.
