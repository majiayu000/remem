# High-Fidelity Episode Evidence Product Spec

Status: Current contract
Issue: #626
Related:
- M6 public proof umbrella: #384
- Coding-agent outcome benchmark: #385
- Current memory contracts: #381, #383, #384, #385, #390

## Problem

remem's default product path intentionally distills raw agent sessions into
curated memories, temporal facts, summaries, and searchable context. That is
the right default for local coding-agent use: it keeps context compact,
governable, and useful under hook latency and token budgets.

Public long-horizon memory comparison has a different need. When a benchmark
fails, reviewers must be able to tell whether remem lost source evidence,
failed to retrieve preserved evidence, applied a policy abstention, or simply
failed the downstream coding task. Default cleanup and compression can make
that distinction ambiguous if episode-level evidence is no longer available to
the verifier.

## Goal

Define an optional high-fidelity evidence mode that preserves bounded episode
and source slices for benchmark, debugging, and falsification workflows without
changing default local retention.

The mode must make failures diagnosable:

- `missing_source_evidence`: the relevant source slice was not preserved or was
  policy-blocked before the benchmark could inspect it.
- `stored_but_not_retrieved`: the source slice exists, but search or context
  retrieval missed it.
- `retrieved_but_not_used`: the evidence was delivered, but the reader or
  coding agent ignored it.
- `policy_abstained`: policy correctly blocked retention, promotion, retrieval,
  or injection.
- `ordinary_task_failure`: memory evidence was available and used, but the
  coding task still failed.

## Non-Goals

- Do not make high-fidelity retention the default.
- Do not bypass non-retention policy for secrets, credentials, restricted user
  claims, unsupported assistant claims, or unapproved external-source claims.
- Do not duplicate the #385 coding benchmark runner.
- Do not introduce a second durable memory truth beside existing current-memory
  contracts unless an implementation spec later proves the existing tables
  cannot hold the required links.
- Do not expose private raw transcripts in committed public benchmark artifacts.

## Product Contract

### Default Versus High-Fidelity Retention

Default local installs keep the existing retention behavior. Users get curated
memory, search, current-state resolution, raw recall where available, and normal
cleanup/compression semantics.

High-fidelity evidence mode is opt-in and scoped. It is enabled only for one of:

- an explicit benchmark run;
- an explicit project-level diagnostic session;
- an explicit session id chosen by a user or test fixture.

The mode must be visible in status and benchmark artifacts. A user or reviewer
must be able to see whether a run used default retention or high-fidelity
retention.

### Preserved Evidence Slices

The mode preserves source slices, not every byte forever. A preserved slice is
the minimal evidence range required to explain a memory or benchmark answer.

Allowed slice kinds:

- captured event ranges from `captured_events`;
- raw user/assistant turns from `raw_messages` when raw archive capture is the
  source of truth;
- compressed-source links from `compressed_observation_sources`;
- cited source event ids attached to observations, candidates, temporal facts,
  graph edges, user-context candidates, or current-state answers;
- benchmark fixture episodes and their reference times.

Each slice records:

- project, branch when known, host, session id, and reference time;
- source event ids or raw message ids;
- retention reason such as `benchmark_fixture`, `debug_trace`, or
  `source_anchor`;
- policy status: retained, redacted, suppressed, deleted, or blocked;
- purge eligibility and storage budget bucket.

### Scope And Activation

High-fidelity mode is disabled by default for ordinary local installs.

Activation must be explicit:

- CLI: a future implementation may add a flag such as
  `--high-fidelity-evidence` to benchmark/debug commands.
- Config: a future implementation may allow a bounded project/session policy,
  but it must not apply globally without an explicit user decision.
- Fixture: benchmark manifests may request high-fidelity mode for isolated
  temporary data directories.

Activation must never silently install hooks, change high-context config files,
or retain restricted user-context material.

### Links To Existing Memory Truth

High-fidelity slices are evidence for existing records, not replacement memory.
They link to:

- curated memories in `memories`;
- temporal facts in `memory_facts`;
- observations and memory candidates;
- graph edges and conflict edges;
- context injection decisions in `context_injections` and
  `context_injection_items`;
- citation and usage feedback in `memory_citation_events` and
  `memory_usage_events`;
- current-state answers and their history/why edges.

The product view must make provenance clear: a current answer can cite a
current memory, the current memory can cite temporal facts and source event ids,
and high-fidelity mode can expose the preserved source slice for audit.

### User Surfaces

`remem search_raw` and MCP `search_raw` remain recall-only evidence surfaces.
When high-fidelity mode is active, raw results may include a
`high_fidelity_source` marker showing whether the hit is preserved, redacted,
or policy-blocked.

MCP `get_observations`, `current_state`, `timeline`, and REST memory/detail
surfaces may expose compact source-slice metadata. They must not dump full raw
private text by default.

Benchmark artifacts must record:

- `high_fidelity_mode`: `off`, `benchmark`, `project`, or `session`;
- preserved source slice counts;
- missing supporting source ids;
- policy-blocked supporting source ids;
- retrieval-delivered source ids;
- source ids cited or used by the agent/reader.

### Privacy And Governance

High-fidelity mode must preserve existing suppression and deletion semantics.

Required behavior:

- secrets and credentials are redacted or blocked before preservation;
- restricted user-context claims are not retained against policy;
- policy-suppressed memories and source slices are hidden from default search,
  detail, context injection, and benchmark public artifacts;
- delete/reject/stale governance actions update or hide linked evidence slices
  consistently;
- public artifacts may include hashes, ids, redaction markers, and counts, but
  must not include private raw text unless the fixture is explicitly public and
  approved.

### Storage Bounds

The mode must be bounded by policy:

- per-run or per-session byte cap;
- per-project retained-slice cap;
- age-based purge policy;
- explicit cleanup command or benchmark artifact cleanup;
- status/doctor warning when preserved slices exceed budget or when purge fails.

If the budget is exceeded, the system must fail closed for benchmark runs:
report `missing_source_evidence` or `policy_abstained` rather than silently
pretending the evidence was available.

### Benchmark Interpretation

#385 coding benchmark runs must record whether high-fidelity mode was active.
The report must distinguish:

- source evidence was never available to remem;
- source evidence was preserved but not promoted or indexed;
- source evidence was promoted/indexed but not retrieved;
- source evidence was retrieved but not used by the agent;
- evidence was intentionally absent because policy blocked it.

This distinction is required before public reports claim that a memory failure
is a retrieval/ranking failure.

## Implementation Slices

1. Define storage metadata for preserved source slices and links to existing
   memory/current-state records.
2. Add opt-in activation for benchmark/debug commands with default-off behavior.
3. Expose compact slice metadata through raw search, detail/current-state, and
   benchmark artifacts.
4. Add governance and purge behavior for retained slices.
5. Add doctor/status checks for active mode, byte budgets, policy-blocked
   slices, and purge failures.
6. Add public benchmark verifier checks that fail when source evidence is
   missing, private paths leak, or mode metadata is absent.

## Validation Commands

Spec PR:

```bash
python3 scripts/ci/check_spec_lifecycle.py
cargo fmt --check
cargo check
```

Future implementation PRs:

```bash
cargo test high_fidelity --lib
cargo test coding_bench --lib
cargo run -- eval-coding-bench --fixture eval/coding-bench/fixtures/tasks.json --runs-per-condition 1 --dry-run
cargo run -- eval-gates --json-out /tmp/remem-eval-gates.json
```
