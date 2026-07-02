# Memory Poisoning Defense Product Spec

Status: Current contract
Date: 2026-07-02

Tracking:
- Spec/tracking issue: #672
- Related: #377 (injection accountability, closed), #383 (usage feedback)

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

## Non-Goals

- Semantic or embedding-based intent scoring of every capture (a heavier
  future layer; this spec is pattern + provenance).
- Network calls or LLM calls in the injection path.
- A general secrets scanner (redaction already exists and stays unchanged).
- Retroactive classification of the entire existing memory store in the first
  implementation (a backfill command may follow separately).

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

## Edge Cases

- Legitimate memories that quote attack strings (for example a lesson about
  prompt injection itself): quarantine fires, review approves; the approval
  acknowledgement exists exactly for this case.
- Mixed provenance (one candidate supported by both a user prompt and web
  content): the lowest trust class among supporting evidence wins.
- Existing memories created before this feature have no trust class; they are
  treated as `local_tool_output` by default and are not retro-quarantined.
- Pattern-set updates: raising the pattern-set version re-scans only at
  injection time; it does not bulk-rewrite stored rows.

## Rollout Notes

Ship scan-and-quarantine on by default (it only affects new candidates), with
the injection-time re-scan behind a config flag for one release to measure the
false-positive rate before defaulting it on.
