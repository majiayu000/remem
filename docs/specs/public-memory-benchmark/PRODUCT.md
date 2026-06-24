# Public Memory Benchmark Product Spec

Status: Current contract
Tracking:
- Spec: #629
- M6 public proof umbrella: #384
- Coding-agent outcome benchmark: #385
- Implementation: #630, #631, #632, #633, #634, #635, #636, #637, #638

## Problem

remem needs public proof for two different claims that are easy to confuse:

1. The memory system is reliable at writing, compressing, retrieving, updating,
   abstaining, labeling staleness, and citing evidence across long histories.
2. Coding agents complete real engineering tasks better when remem is available
   than when they run with no memory or a maintained context file.

The existing `issue385-coding-agent-ab` spec owns the second claim. It compares
`no_memory`, `remem`, and `curated_file` on real coding tasks and records
resolution, tokens, turns, wall time, artifacts, and failure reasons.

The missing layer is a public memory-system benchmark that can explain why a
memory run succeeds or fails before the agent outcome benchmark is considered.
Without that layer, a failed coding task cannot be attributed to write-side
evidence loss, retrieval miss, reader failure, policy abstention, stale memory,
or ordinary coding failure. Without the #385 layer, deterministic memory quality
does not prove that coding agents solve more engineering tasks.

## Goal

Define a reproducible public benchmark program with two independent but
chainable evidence layers:

1. **Memory-system capability evidence**: long-term memory QA, retrieval,
   citation, temporal update, conflict, abstention, staleness, privacy, and
   non-retention gates.
2. **Coding-agent outcome evidence**: the #385 no-memory/remem/curated-file
   A/B benchmark on memory-dependent engineering tasks.

The public report must say which layer a result supports. Memory capability
results may support claims about memory-system quality. Only the coding-agent
outcome layer may support claims that remem improves engineering task
completion.

## Non-Goals

- Do not replace `docs/specs/issue385-coding-agent-ab/`.
- Do not claim that public QA/retrieval benchmarks prove coding-agent outcome
  improvement.
- Do not benchmark against the user's real `~/.remem` database or private
  project memory.
- Do not use LoCoMo or any external benchmark as a headline gate without
  committed methodology, dataset revision, model, and report artifacts.
- Do not publish "SOTA", "best", or "beats MEMORY.md" wording from
  deterministic evals alone.
- Do not tune fixtures, prompts, curated files, or baselines after seeing
  condition results without recording a new benchmark version.

## Product Contract

### Evidence Layers

#### Layer 1: Memory-System Capability

This layer proves that remem's memory machinery works as a memory system:

- capture and write path preserves durable evidence;
- compression and promotion do not discard answer-critical facts;
- retrieval recalls evidence under fixed budgets;
- temporal updates and stale facts are handled as-of a reference time;
- conflicts are detected instead of flattened into arbitrary current state;
- unsupported questions abstain;
- citations identify supporting memory IDs and source anchors;
- source-anchor staleness is labeled as `tracked`, `verify-before-trust`,
  `untracked`, or `error`;
- non-retention policy blocks secrets, credentials, unsafe user claims,
  unsupported assistant claims, and unapproved external-source claims.

Layer 1 failures must be decomposed. A report that only says `accuracy=82%` is
not sufficient. The report must answer whether the failure came from:

- write-side evidence loss;
- retrieval-side miss;
- reader/scorer failure;
- correct policy abstention;
- stale/conflicting evidence;
- policy leak or unsafe retention.

#### Layer 2: Coding-Agent Outcome

This layer is the existing #385 benchmark. It runs real engineering tasks under
the required conditions:

- `no_memory`;
- `remem`;
- `curated_file`.

The #385 layer remains the authority for outcome claims such as "remem helps
coding agents solve more tasks", "remem saves tokens", or "remem outperforms a
maintained context file". This spec extends the artifact and claim framework
around #385, but does not duplicate or replace it.

### Public Benchmark Suites

The memory-system layer uses three suite families.

#### External Long-Term Memory Anchors

External suites are included so other systems can reproduce and compare
results. The first adapter targets are:

- LongMemEval: information extraction, multi-session reasoning, temporal
  reasoning, knowledge updates, and abstention over 500 curated questions.
- LoCoMo: very long multi-session conversations, useful for conversational
  memory comparison but informational until methodology is revalidated for
  remem's coding-memory claims.
- MINTEval: long-horizon memory with multi-target interference, updates,
  distractors, and aggregation.
- LongMemEval-V2: agent environment experience questions covering static state,
  dynamic state tracking, workflow knowledge, gotchas, and premise awareness.

External suite adapters must pin dataset revisions, license notes, preprocessing
steps, reader model, scoring prompts, and excluded cases. If licensing or access
prevents committing raw data, the repo must commit manifests, checksums,
download instructions, and a smoke subset.

#### remem-Native Coding Memory Suite

This suite tests coding memory correctness without asking an agent to edit code.
Fixtures come from public repository history or synthetic but realistic session
streams. Each sample has history episodes and questions such as:

- "As of this reference time, which API is current?"
- "What was the root cause of this bug?"
- "Which old constraint was invalidated by a later commit or decision?"
- "Should this old memory be treated as tracked, verify-before-trust, or
  no-current?"
- "Which workstream identity should be continued after a rename?"
- "Which user preference is relevant to this task, and which preference should
  not be injected?"

Initial categories:

- prior decision dependency;
- prior bug root cause;
- stale memory avoidance;
- negative project constraint;
- workstream continuity;
- multi-hop project context;
- user-context relevance;
- conflict and ambiguity handling;
- source-anchor staleness;
- citation support.

#### Adversarial Memory Policy Suite

This suite proves that remem does not over-retain or over-inject. It must cover:

- secrets, API keys, credentials, account/payment data;
- third-party personal details without explicit framing;
- negation, jokes, roleplay, and sarcasm;
- assistant-authored unsupported claims;
- unapproved external-source claims;
- cross-sentence splicing;
- same-name repositories and branch divergence;
- same session with multiple unrelated tasks;
- workstream rename drift;
- stale file anchors;
- conflicting memories.

The required default for non-retainable evidence is no active memory claim, no
active user-context claim, no candidate, and no profile-summary input unless a
fixture explicitly models user approval.

### Benchmark Conditions

Memory suites must support these conditions where meaningful:

| Condition | Purpose |
|---|---|
| `no_memory` | Reader receives only the question. Lower bound. |
| `full_context` | Reader receives complete history where budget permits. Long-context upper reference. |
| `truncated_full_context` | Reader receives budget-limited history. Separates long-context budget loss. |
| `oracle_evidence` | Reader receives gold supporting evidence only. Reader upper bound. |
| `complete_stored_memory` | Reader receives remem's stored memory corpus without retrieval filtering. Write/compression check. |
| `retrieved_memory` | Reader receives real top-k retrieved memory. Actual memory usability check. |
| `bm25_baseline` | FTS/BM25 baseline over the same allowed evidence. |
| `vector_baseline` | Embedding retrieval baseline over the same allowed evidence. |
| `hybrid_rag_baseline` | BM25 plus vector plus reciprocal-rank fusion baseline. |
| `summary_baseline` | Per-session or rolling summary baseline. |
| `remem_default` | Current production configuration. |
| `remem_ablation_*` | Module ablations such as no temporal, no staleness, no entity, no usage, no query expansion. |

The diagnostic interpretation follows the same shape as the WhenLoss protocol:
compare truncated full context, oracle evidence, complete stored memory, and
retrieved memory to separate write-side from retrieval-side bottlenecks.

Coding-agent outcome suites keep the #385 required conditions:

| Condition | Purpose |
|---|---|
| `no_memory` | Agent runs without remem hooks, MCP, native memory, or curated context. |
| `remem` | Agent runs with remem through a supported runtime path and temporary data dir. |
| `curated_file` | Agent receives a realistic hand-maintained context file derived from the same source evidence. |

Optional coding-agent extensions are allowed only after the base three pass:

- `oracle_notes`;
- `remem_search_only`;
- `remem_no_staleness`;
- `remem_no_temporal`.

### Metrics

#### Memory-System Metrics

Retrieval:

- Recall@1/5/10/20;
- MRR;
- nDCG;
- support coverage;
- irrelevant memory rate;
- abstention precision and recall.

Answer quality:

- exact match or rubric score;
- temporal-as-of accuracy;
- knowledge-update accuracy;
- multi-hop accuracy;
- conflict detection accuracy;
- no-answer correctness.

Evidence:

- citation precision;
- citation recall;
- supporting memory IDs;
- source-anchor correctness;
- staleness label correctness.

Cost and performance:

- ingest tokens;
- query tokens;
- reader tokens;
- p50/p95 retrieval latency;
- p50/p95 end-to-end latency;
- database size;
- rows written per session.

Governance:

- non-retention leak rate;
- false block rate;
- suppression obeyed rate;
- deleted, sensitive, restricted, and suppressed default exclusion rate.

Stability:

- same-seed repeatability;
- parallel test determinism;
- config isolation;
- proof that real `~/.remem` was not accessed.

#### Coding-Agent Outcome Metrics

The #385 metrics remain required:

- `resolved`;
- `tokens_input`, `tokens_output`, `tokens_total`;
- `turns`;
- `wall_time_ms`;
- `commands_run`;
- final head SHA or patch artifact;
- `failure_reason`.

The public artifact layer adds:

- `memory_tokens`;
- `tool_calls`;
- patch diff path;
- test log path;
- injected context path;
- `injected_memory_ids`;
- `used_memory_ids`;
- citation precision and recall;
- stale memory used count;
- irrelevant injection count;
- missing relevant memory count;
- `memory_helped`;
- `memory_hurt`.

`failure_reason` must use a fixed enum, not free text:

- `test_failure`;
- `timeout`;
- `compile_failure`;
- `wrong_file_modified`;
- `ignored_memory`;
- `missing_memory`;
- `stale_memory_followed`;
- `irrelevant_memory_distracted`;
- `over_context_budget`;
- `agent_hallucinated_memory`;
- `oracle_inconclusive`.

### Claim Levels

#### Level 1: Reproducible Local Memory Benchmark

Allowed claim: "remem outperforms baseline Y on benchmark X under the published
memory-system harness."

Required evidence:

- external or remem-native memory suite can be reproduced from a clean checkout;
- reports include full context, oracle, RAG, summary, remem default, and at
  least one relevant ablation;
- remem is materially better than at least one baseline on a defined memory
  capability combination;
- artifacts pass the verifier.

Forbidden claim: coding-agent task improvement.

#### Level 2: Coding-Agent Outcome Improvement

Allowed claim: "remem improved coding-agent outcome on fixture X under the
published #385 harness."

Required evidence:

- `no_memory`, `remem`, and `curated_file` all run on the same task set;
- at least three runs per condition;
- remem has positive resolution delta versus no memory;
- token, turn, and wall-time regressions are reported and justified;
- remem is not materially worse than curated file, or the report explains the
  tradeoff in maintenance cost and context noise.

Forbidden claim: public SOTA.

#### Level 3: Public SOTA Claim

Allowed claim: "remem is SOTA for benchmark X under harness Y and budget Z."

Required evidence:

- comparison uses a public benchmark, public baseline, same model, same budget,
  same harness, and published artifacts;
- report includes reproducible raw outputs, logs, model locks, prompt hashes,
  and environment metadata;
- claim wording names the benchmark and condition. It must not generalize to all
  long-term memory or all coding agents.

### Coding Outcome Stop-Loss Gate

The stop-loss gate applies to README, release, marketing, or roadmap claims
that remem improves coding-agent outcomes, beats a maintained context file, or
is broadly superior for coding workflows. It does not gate scoped
memory-system capability claims. Level 1 memory-system claims and Level 3
memory-benchmark claims may pass on their own reproducible memory evidence,
artifact verifier result, and claim-level requirements without waiting for #385
coding deltas. If a Level 3 claim is about coding-agent outcomes rather than a
memory benchmark, this coding outcome gate still applies.

For coding-outcome superiority claims, `remem-public-coding-claim` passes only
if all are true:

1. remem beats `no_memory` on coding-agent resolved rate by at least 10
   percentage points, or by a statistically credible positive bootstrap
   interval.
2. remem is not worse than `curated_file` by more than 3 percentage points.
3. remem total token cost is at most `curated_file + 20%`, unless a higher
   resolved rate justifies the cost.
4. stale-memory-caused failures are under 2% of runs.
5. privacy and non-retention leak rate is 0 on the adversarial suite.
6. All artifacts reproduce from a clean checkout.

If `curated_file` ties or beats remem with lower cost and no material usability
downside, the roadmap must record a stop-loss signal. The next slice should
focus on ergonomics, export/import, human-maintained memory workflows, and
context-file integration rather than more retrieval machinery.

## Implementation Roadmap

### Phase 1: Artifact Schema And Verifier

Issue: #630

Add the artifact schema, sample fixtures, report verifier, and local smoke
commands. This phase does not invoke agents or external datasets.

### Phase 2: Memory Capability Suites

Issues: #631, #632, #633

Add remem-native coding memory QA, adversarial policy fixtures, diagnostic
conditions, and baseline adapters. Start with remem-native and adversarial
suites because they are closest to current code and user-context policy.

### Phase 3: Coding-Agent A/B MVP

Issues: #634, #635, #636

Implement the #385 runner, task fixture pack, and memory attribution/failure
taxonomy. The first fixture set should contain 12-20 tasks with objective
oracles and at least three runs per condition before publication.

### Phase 4: Baseline Report And Claim Gate

Issues: #637, #638

Publish the directional report and enforce public claim policy. README and
release coding-outcome or broad superiority wording must remain conservative
until artifacts pass the coding outcome stop-loss gate. Scoped memory-system
benchmark wording may use the relevant memory claim level once its memory
artifacts pass verification.

### Phase 5: External Reproduction Package

Add external suite adapters, dataset manifests, Docker image digests, model
lockfiles, prompt hashes, raw logs, DB snapshots, patch artifacts, and artifact
verification commands. The smoke subset must be cheap enough for a clean clone;
the full benchmark may require manual budget approval.

## Acceptance Criteria

- This spec is indexed in `docs/specs/README.md` as a current contract.
- The public benchmark report separates memory-system capability results from
  coding-agent outcome results.
- The #385 benchmark remains the source of truth for coding-agent outcome.
- Artifacts include enough metadata to reproduce results without private memory.
- Every public claim links to the report and claim level it satisfies.
- Unsupported SOTA, broad superiority, or coding-task superiority wording is
  forbidden in README or release docs.

## Open Product Decisions

- Which model/provider is the first public memory-suite reader.
- Whether the first external adapter should be LongMemEval or LongMemEval-V2.
- Whether LoCoMo remains informational only or receives a revalidated public
  methodology.
- Whether `remem bench` is a new command group or a compatibility wrapper over
  existing `eval-*` commands for the first implementation slice.
- How large the first public coding task pack should be before moving beyond
  directional evidence.
