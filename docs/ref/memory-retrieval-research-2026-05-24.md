# Memory Retrieval Research Notes

Date: 2026-05-24

Scope: this note records the broader search requested after PR #180. The immediate design constraint is to avoid making dense vector embeddings, vector databases, or neural rerankers the next implementation step. Those options are documented as later-stage possibilities only when a source discusses them.

Coverage note: this is a curated research note, not an exhaustive index of every recent AI memory blog. The sources below were selected for design relevance to remem. A quick follow-up sweep on 2026-05-24 found additional recent blog posts and papers worth tracking, listed in "Recent Watchlist".

## Current remem Baseline

remem already has a strong local-first foundation:

- Automatic capture through hooks, instead of relying on the agent to remember to call `save_memory`.
- SQLite storage with FTS5 trigram search, LIKE fallback for short tokens, entity search, temporal search, and RRF fusion.
- Project-scoped context injection with a global preference overlay.
- Session summaries, workstreams, raw archive, observations, and typed memories.

PR #180 addressed the first non-vector quality issue found in this round:

- `eval-local` now separates true project leaks from intentional `scope='global'` overlay hits.
- Entity search now ranks project-local memories before global overlay memories, with stable recency/id ordering.

The remaining question is not "add a vector DB"; it is how to improve retrieval, memory lifecycle, and evaluation while preserving local-first behavior and auditability.

## Research Threads

### Recent Watchlist Not Fully Reviewed

These sources were found after the first document pass. They are not yet fully incorporated into the roadmap.

- Mem0, "Introducing The Token-Efficient Memory Algorithm" (April 2026): https://mem0.ai/blog/mem0-the-token-efficient-memory-algorithm
- Mem0, "Long-Term Memory for AI Agents" (February 2026): https://mem0.ai/blog/long-term-memory-ai-agents
- Redis, "Long-Term Memory Architectures for AI Agents" (April 2026): https://redis.io/blog/long-term-memory-architectures-ai-agents/
- Letta, "Benchmarking AI Agent Memory: Is a Filesystem All You Need?" (August 2025): https://www.letta.com/blog/benchmarking-ai-agent-memory
- Taskade, "Memory Reanimation Protocol: AI Agent Memory" (April 2026): https://www.taskade.com/blog/memory-reanimation-protocol
- AgentMarketCap, "Agent Memory in Production 2026" (April 2026): https://agentmarketcap.ai/blog/2026/04/11/agent-memory-architecture-production-2026
- Agents' Codex, "Agent Memory: Hybrid Episodic-Semantic Systems for Production" (March 2026): https://agentscodex.com/posts/2026-03-10-agent-memory-architectures-hybrid-episodic-semantic/
- VentureBeat, "Observational memory cuts AI agent costs 10x..." (February 2026): https://venturebeat.com/data/observational-memory-cuts-ai-agent-costs-10x-and-outscores-rag-on-long/
- SurePrompts, "Agent Memory Architectures Compared (2026)" (May 2026): https://sureprompts.com/blog/agent-memory-architectures-compared-2026
- MemMachine paper (April 2026): https://arxiv.org/abs/2604.04853
- "Beyond RAG for Agent Memory" (February 2026): https://arxiv.org/abs/2602.02007
- "Memory for Autonomous LLM Agents: Mechanisms, Evaluation, and Emerging Frontiers" (March 2026): https://arxiv.org/abs/2603.07670
- SAGE graph-memory paper (May 2026): https://arxiv.org/abs/2605.12061
- "Memory in the Age of AI Agents: A Survey" (May 2026): https://openreview.net/forum?id=180d26775b5edf368b1aea4bcf724855acc29c14
- "Memory OS of AI Agent" (2025 EMNLP): https://aclanthology.org/2025.emnlp-main.1318/
- ICLR 2026 MemAgents workshop: https://iclr.cc/virtual/2026/workshop/10000792
- MCML recap of MemAgents workshop: https://mcml.ai/news/2026-05-06-mem-agents-workshop-at-iclr-2026/
- MemAgent ICLR 2026 poster: https://iclr.cc/virtual/2026/poster/10007825
- Memanto paper (April 2026): https://arxiv.org/abs/2604.22085
- Memanto research page: https://memanto.ai/research
- Mastra Observational Memory: https://mastra.ai/research/observational-memory
- Wire, "Tool-based agent memory" (April 2026): https://usewire.io/blog/memory-as-tools-2026-agent-memory-pattern/
- MemCP portable agent memory: https://mcp.altinors.com/
- AgenticMem / ai-memory MCP: https://agentic-mem.com/
- Memstate blog index: https://memstate.ai/blog
- HEBBS blog: https://blog.hebbs.ai/
- Chanl, "Your Agent Remembers Everything Except What Matters": https://www.channel.tel/blog/ai-agent-memory-episodic-semantic-iclr-2026
- Memori Labs trace-native memory announcement: https://www.prweb.com/releases/memori-labs-releases-new-agent-native-memory-infrastructure-automatically-creating-structured-memory-from-agent-trace-302765715.html
- LifeBench: https://arxiv.org/abs/2603.03781
- Chronos temporal-aware memory: https://arxiv.org/abs/2603.16862
- Synthius-Mem: https://arxiv.org/abs/2604.11563
- Beyond the Context Window: fact-based memory vs long-context LLMs: https://arxiv.org/abs/2603.04814
- Recalibrate: metacognitive monitoring for persistent agents: https://papers.ssrn.com/sol3/papers.cfm?abstract_id=6411378
- AgenticMemory binary graph format: https://papers.ssrn.com/sol3/Delivery.cfm/6265798.pdf?abstractid=6265798&mirid=1
- Persistent Memory for AI Agents 2026 implementation roundup: https://byteiota.com/persistent-memory-for-ai-agents-2026-implementation/
- AgentMarketCap, "The Agent Memory Protocol Gap": https://agentmarketcap.ai/blog/2026/04/06/agent-memory-protocol-gap-mcp-letta-memgpt-zep-open-memory-layer
- AgentMemory for coding agents: https://www.agentmemory.tech/
- MemNexus persistent coding-tool memory comparison: https://memnexus.ai/blog/2026-03-19-ai-coding-tools-persistent-memory-2026
- Shodh MCP memory guide: https://www.shodh-memory.com/blog/mcp-memory-server-guide
- Zylos AI memory architecture overview: https://zylos.ai/en/research/2026-04-05-ai-agent-memory-architectures-persistent-knowledge
- Feedback-Normalized Developer Memory for RL coding agents: https://arxiv.org/abs/2605.01567

Initial interpretation:

- Recent blog coverage is converging on the same failure modes: memory is not just query-time RAG, stale/conflicting facts dominate long-running systems, and scoped memory is more important than a single global store.
- The new algorithm/paper stream reinforces remem's non-vector priorities: better lifecycle operations, evidence preservation, temporal invalidation, query routing, and evaluation buckets.
- Many posts are product or marketing oriented. They should be used as idea discovery, not as implementation evidence, unless backed by a paper, benchmark, or reproducible docs.
- The broader product landscape is moving toward MCP-native memory and coding-agent-specific memory. This validates remem's hook/MCP direction, but the claims around benchmark scores and "universal memory" need careful reproduction before they influence design.

Action: before implementing a larger retrieval/lifecycle redesign, run a focused review of this watchlist and promote only sources that provide concrete algorithms, schemas, metrics, or failure cases.

### 1. Lexical Search, RRF, and Query Expansion

Useful sources:

- SQLite FTS5 documentation: https://www.sqlite.org/fts5.html
- Elasticsearch RRF reference: https://www.elastic.co/guide/en/elasticsearch/reference/current/rrf.html/
- Qdrant Hybrid Queries docs: https://qdrant.tech/documentation/search/hybrid-queries/
- Anthropic Contextual Retrieval: https://www.anthropic.com/research/contextual-retrieval
- Query2doc: https://arxiv.org/abs/2303.07678
- SPLADE v2: https://arxiv.org/abs/2109.10086
- "From BM25 to Corrective RAG" benchmark: https://arxiv.org/abs/2604.01733

Relevant ideas:

- FTS5 `bm25()` is already a good fit for code memory because file paths, command names, error text, PR numbers, model names, and dates are often exact lexical signals.
- RRF is a pragmatic way to merge independent retrieval channels without pretending their scores are calibrated.
- Contextual Retrieval's non-vector lesson is important: add concise context to the indexed text so BM25 has enough surrounding words to match underspecified queries.
- Query rewriting can help short or conversational questions, but must be logged and fused with the original query rather than silently replacing it.
- Sparse expansion such as SPLADE can be explored later without a dense vector DB, but it still adds model dependency and tokenization risk.

remem application:

- Add `search_text` or `search_context` as an indexed derived field, built from memory content plus project, branch, files, entities, symptoms, root cause, verification commands, and outcome.
- Keep original memory content immutable; derived search text must be rebuildable.
- Extend RRF channels: original query, exact phrase query, expanded core tokens, entity channel, temporal channel, raw archive fallback, and later an optional query rewrite channel.
- Add debug/explain output for search: channel hits, rank contribution, query rewrite, and final RRF score.

Risks:

- Over-expanded search text can pollute exact matching.
- Query rewriting can hallucinate terms, especially for paths, error codes, dates, and model names.
- More channels can improve recall while hiding precision regressions unless evaluation is split by query type.

### 2. Memory Lifecycle and Conflict Handling

Useful sources:

- Mem0 paper: https://arxiv.org/abs/2504.19413
- Mem0 forgetting discussion: https://mem0.ai/blog/memory-eviction-and-forgetting-in-ai-agents
- Letta memory overview: https://docs.letta.com/guides/agents/memory
- Letta archival memory: https://docs.letta.com/guides/ade/archival-memory
- MemGPT paper: https://arxiv.org/abs/2310.08560
- LangMem conceptual guide: https://langchain-ai.github.io/langmem/concepts/conceptual_guide/
- A-MEM: https://arxiv.org/abs/2502.12110
- MemReader: https://arxiv.org/abs/2604.07877

Relevant ideas:

- Strong memory systems treat memory as a lifecycle, not append-only notes.
- Mem0-style operations are useful: `ADD`, `UPDATE`, `DELETE`, `NOOP`. For remem, `DELETE` should normally mean soft invalidation, not physical deletion.
- Letta/MemGPT's split between always-visible core memory and searchable archival memory maps well to remem's SessionStart context versus raw archive/long-term storage.
- LangMem's semantic/episodic/procedural split fits coding agents better than a single "summary" type.
- Extractors need `NOOP` and `DEFER`; forcing every event into a memory creates pollution.

remem application:

- Add an explicit candidate lifecycle: `raw_event -> candidate -> add/update/invalidate/noop/defer -> active/stale/superseded memory`.
- Store provenance for every derived memory: source event ids, observation ids, session id, host, project, branch, and confidence.
- Introduce a small pinned layer for stable, high-confidence preferences and operational constraints; keep most facts archival.
- Treat procedural memories separately from facts. A procedure should require repeated success or manual review before it becomes pinned or skill-like.

Risks:

- LLM update/delete decisions can damage good memories.
- Pinned memory pollution is worse than archival pollution because it affects every turn.
- Defer queues can rot unless they have visible aging metrics and cleanup rules.

### 3. Temporal Graph and Relationship Memory

Useful sources:

- Zep temporal KG paper: https://arxiv.org/abs/2501.13956
- Zep Facts docs: https://help.getzep.com/facts
- Zep concepts: https://help.getzep.com/v2/concepts
- Graphiti repository: https://github.com/getzep/graphiti
- Graphiti search docs: https://help.getzep.com/graphiti/working-with-data/searching
- Microsoft GraphRAG paper: https://arxiv.org/abs/2404.16130
- GraphRAG query modes: https://microsoft.github.io/graphrag/query/overview/
- Microsoft DRIFT Search: https://www.microsoft.com/en-us/research/blog/introducing-drift-search-combining-global-and-local-search-methods-to-improve-quality-and-efficiency/
- HippoRAG: https://arxiv.org/abs/2405.14831
- HippoRAG 2: https://arxiv.org/abs/2502.14802

Relevant ideas:

- Zep/Graphiti's most useful idea for remem is not "use a graph DB"; it is temporal facts with provenance and invalidation.
- Facts should have validity fields such as learned time, valid-from, valid-until, invalidated-at, and source evidence.
- GraphRAG separates local entity questions from global sensemaking questions. remem has both: "what was the hook path fix?" and "what is this project doing recently?"
- HippoRAG shows graph traversal can help multi-hop association, but raw passages remain important because triples lose context.

remem application:

- Start with a lightweight temporal fact table in SQLite, not a full graph database.
- Use a minimal ontology first: `Project`, `File`, `Command`, `Issue`, `PR`, `Model`, `Decision`, `Bugfix`, `Preference`, and `Procedure`.
- Add typed edges only where they help coding workflows: `fixed_by`, `verified_by`, `blocked_by`, `supersedes`, `uses_file`, `uses_command`, `affects_project`.
- Implement `as_of(time)`, current facts, historical facts, and superseded/conflicting facts before any complex PageRank-style traversal.
- Build project/topic community summaries later for global questions such as "what changed this week?" or "what is the current risk?"

Risks:

- Entity merging errors are amplified by graph traversal.
- Temporal extraction is fragile; explicit event time and inferred validity time must be stored separately.
- Community summaries go stale and must never become the source of truth.

### 4. Evaluation and Benchmarks

Useful sources:

- LoCoMo: https://snap-research.github.io/locomo/
- LoCoMo paper: https://arxiv.org/abs/2402.17753
- LongMemEval: https://arxiv.org/abs/2410.10813
- LongMemEval GitHub: https://github.com/xiaowu0162/LongMemEval
- LongMemEval-V2: https://arxiv.org/abs/2605.12493
- RAGAS metrics: https://docs.ragas.io/en/stable/concepts/metrics/available_metrics/
- BEIR GitHub: https://github.com/beir-cellar/beir
- BEIR paper: https://arxiv.org/abs/2104.08663
- SQuAD 2.0: https://rajpurkar.github.io/SQuAD-explorer/
- UAEval4RAG: https://aclanthology.org/2025.acl-long.415.pdf
- AbstentionBench: https://arxiv.org/abs/2506.09038

Relevant ideas:

- LoCoMo is informational-only for remem. It is useful research context, but
  not a CI or release gate.
- `self_retrieval` alone is too optimistic. It tests whether a recent title can find itself, not whether a user question gets the right evidence.
- LongMemEval's buckets are useful for remem: information extraction, multi-session reasoning, temporal reasoning, knowledge update, and abstention.
- RAGAS separates context quality from answer faithfulness. remem should do the same: first measure retrieved evidence, then measure generated answer quality if answer generation is introduced.
- BEIR-style metrics are useful and deterministic: Hit@k, MRR, Recall@k, Precision@k, MAP, and nDCG.
- No-answer and false-premise cases need their own metrics; they should not be treated as ordinary retrieval misses.

remem application:

- Create a golden query file with stable references, not only volatile local memory ids.
- Suggested schema fields: `id`, `query`, `project`, `category`, `answerable`, `expected_answer`, `evidence_refs`, `relevant_topic_keys`, `grade`, and `rationale`.
- Categories: `single_session`, `multi_session`, `temporal`, `knowledge_update`, `preference`, `project_scope`, `procedure`, `abstention`, and `false_premise`.
- Report per-category metrics, not just one overall score.
- Add `evidence_recall@k`, `nDCG@10`, `rank_histogram`, and missing examples.
- Use LLM-as-judge only for diagnostic answer quality, not as the primary CI gate.

Risks:

- Public memory benchmarks can be gamed by long context windows and prompt tuning.
- Golden sets become stale unless evidence refs are stable and reviewed.
- Graded relevance improves nDCG but increases annotation cost.

### 5. Agent and Coding-Workflow Memory

Useful sources:

- OpenAI AGENTS.md guide: https://developers.openai.com/codex/guides/agents-md
- Claude Code memory docs: https://docs.claude.com/en/docs/claude-code/memory
- Claude Code hooks: https://docs.claude.com/en/docs/claude-code/hooks
- Claude Code skills: https://docs.claude.com/en/docs/claude-code/skills
- OpenAI Agents SDK tracing: https://openai.github.io/openai-agents-python/tracing/
- Letta context repositories: https://www.letta.com/blog/context-repositories
- Letta Code memory docs: https://docs.letta.com/letta-code/memory/
- Git Context Controller: https://arxiv.org/abs/2508.00031
- Agentic coding manifests study: https://arxiv.org/abs/2509.14744

Relevant ideas:

- Coding agents need scoped memory: organization/user/project/directory/task memory should not collapse into one flat pool.
- Hook payloads and traces are not memory, but they are the raw evidence memory should be derived from.
- Git-backed context repositories are useful because memory changes become reviewable, mergeable, and revertible.
- Procedural memory is different from factual memory. A repeated verified workflow can become a runbook or skill; a one-off event should not.

remem application:

- Continue prioritizing automatic capture and background extraction. Do not rely on agent self-discipline to save important memories.
- Normalize traces into event/span-like records: workflow id, session id, parent span, tool name, timestamps, project, cwd, branch, metadata, and redaction status.
- Add a reviewable export path later: project digest, memory delta, or markdown runbook that can be committed deliberately.
- Promote procedural memories only after repeated success, explicit verification commands, and stable project scope.

Risks:

- Raw tool traces can include secrets, private code, or irrelevant output.
- Auto-writing project files can pollute repositories; SessionStart injection is safer by default.
- Procedural memories become harmful if they encode outdated deployment or review practices.

## Recommended Non-Vector Roadmap

## Design Pattern Comparison

This section compares the main memory system designs found in the recent sweep. The goal is to decide what remem can borrow without losing its local-first, hook-driven, auditable design.

### Pattern A: Passive Context Injection

Examples: current remem SessionStart context, classic RAG memory, many MCP memory servers.

Design:

- Search runs before the agent reasons.
- Top memories are injected into the prompt.
- The agent consumes whatever retrieval selected.

Strengths:

- Simple, fast, deterministic, and easy to test.
- Works well for startup context, user preferences, project facts, and recent decisions.
- Good match for Codex/Claude session-start hooks.

Weaknesses:

- Retrieval runs "blind" before the agent knows what it actually needs.
- No chance for the agent to navigate adjacent evidence or update stale memory during reasoning.
- Bad retrieval pollutes the prompt every turn.

Borrow for remem:

- Keep this as the default startup path.
- Make it smaller and more explainable: project-local first, global preferences separately, and show why each memory was injected.
- Do not rely on this path alone for complex questions.

### Pattern B: Tool-Based Memory

Examples: Wire's tool-based memory discussion, many MCP-native memory systems, Mem0-style tool operations.

Design:

- Memory operations are tools: retrieve, navigate, store, update, discard.
- The agent decides when to call memory inside the reasoning loop.
- Tool results return source metadata and adjacent navigation handles.

Strengths:

- Better for ambiguous questions, multi-hop recall, and stale/conflicting facts.
- Lets the agent ask follow-up memory queries instead of accepting a fixed top-k.
- Fits MCP naturally.

Weaknesses:

- Requires agent discipline and good tool descriptions.
- Can add latency and tool-call cost.
- If update/delete tools are too permissive, the agent can damage memory.

Borrow for remem:

- Keep automatic capture as the write backbone; do not depend on the agent to save important facts.
- Add read-side tools such as `search`, `show`, `timeline`, `neighbors`, and later `explain`.
- Add write/update tools only behind review or soft-invalidation gates.

### Pattern C: Observational Memory

Examples: Mastra Observational Memory, older observer/reflector designs, remem session summaries.

Design:

- Background observers convert raw conversations into dense observations.
- The agent sees a stable observation log instead of dynamically retrieved context.
- The main conversation can drop old raw messages once they are represented as observations.

Strengths:

- Stable, cacheable prompt.
- Good for long conversation continuity.
- Does not require per-turn retrieval.

Weaknesses:

- Observation quality decides everything.
- If an observation is wrong or incomplete, later turns inherit that compression error.
- Less useful for open-ended search outside what was observed.

Borrow for remem:

- This is close to remem's strongest direction: Stop/context-focused automatic capture and background summarization.
- Improve observation quality with provenance, confidence, lifecycle operations, and better eval.
- Keep raw archive as source of truth so observations can be rebuilt.

### Pattern D: Typed Semantic Memory

Examples: Memanto, LangMem semantic/episodic/procedural split, Redis taxonomy.

Design:

- Memories are not a flat bag of notes.
- They are typed: fact, preference, decision, procedure, event, artifact, error, relationship, etc.
- Retrieval and update behavior depends on type.

Strengths:

- Easier to route and score.
- Makes conflict handling more specific.
- Helps evaluation by category.

Weaknesses:

- Bad type classification creates hidden routing errors.
- Too many types become hard to maintain and test.

Borrow for remem:

- Keep existing decision/bugfix/discovery/preference types, but add clearer semantics for `episode`, `fact`, `procedure`, and `artifact`.
- Use type-specific indexing and rendering.
- Add type-specific evaluation buckets.

### Pattern E: Lifecycle and Conflict Management

Examples: Mem0 `ADD/UPDATE/DELETE/NOOP`, Zep invalidated facts, memory forgetting papers.

Design:

- New evidence is compared with existing memory.
- The system chooses add, update, invalidate, noop, or defer.
- Old facts are superseded rather than silently overwritten.

Strengths:

- Directly attacks stale and contradictory memory.
- Prevents append-only memory bloat.
- Makes memory auditable.

Weaknesses:

- LLM judgments can be wrong.
- Automatic deletion is dangerous.
- Requires good provenance and rollback.

Borrow for remem:

- Use `invalidate` and `supersede`, not hard delete.
- Add `noop` and `defer` to extraction outputs.
- Build visible metrics: write rate, noop rate, defer age, superseded count, conflict count.

### Pattern F: Temporal Fact Graph

Examples: Zep/Graphiti, GraphRAG, HippoRAG, Chronos.

Design:

- Facts are edges between entities.
- Facts carry time and provenance.
- Retrieval can filter by current, historical, or as-of time.

Strengths:

- Strong for "what changed?", "what was true then?", and multi-hop relationships.
- Fits software work where PRs, issues, branches, commands, and decisions supersede each other.

Weaknesses:

- Entity resolution errors are amplified.
- Full GraphRAG is expensive and easy to overbuild.
- Community summaries can become stale.

Borrow for remem:

- Do not start with a full graph DB.
- Add lightweight SQLite temporal facts first: subject, predicate, object, valid_from, valid_to, source ids, confidence.
- Start with coding-specific edges: `fixed_by`, `verified_by`, `supersedes`, `blocked_by`, `uses_file`, `uses_command`.

### Pattern G: Trace-Native Developer Memory

Examples: Memori Labs trace-native memory, OpenAI Agents SDK tracing, Claude Code hooks, coding-agent memory systems.

Design:

- Tool calls, tool results, decisions, failures, retries, and outcomes become structured evidence.
- Memory is derived from execution trace, not only chat text.

Strengths:

- Best fit for coding agents.
- Captures what actually happened, not only what was said.
- Enables workflow/procedural memory.

Weaknesses:

- Tool traces can contain secrets or large noisy outputs.
- Direct trace retrieval is too noisy.
- Requires redaction and retention controls.

Borrow for remem:

- Treat trace as raw evidence, not as final memory.
- Normalize events with project, cwd, branch, session, tool, timestamps, result status, files touched, and redaction status.
- Promote repeated verified workflows into procedural memory only after successful runs.

## What remem Should Borrow First

The highest-leverage ideas are:

1. Golden query evaluation with stable evidence refs.
2. `search_context` as rebuildable FTS text, not dense vectors.
3. Search explain output showing channel, rank, and scope.
4. Typed memory buckets: fact, episode, decision, bugfix, preference, procedure, artifact.
5. Lifecycle operations: add, update, invalidate, noop, defer.
6. Soft supersession instead of deletion.
7. Lightweight temporal facts with provenance.
8. Read-side memory tools for navigation and evidence expansion.
9. Trace-derived procedural memory, gated by verification.

What to avoid for now:

- A full graph database.
- Mandatory embeddings or vector DB.
- Neural reranking in the hot path.
- Agent-autonomous hard delete.
- Auto-writing repo memory files.
- Treating marketing benchmark claims as design proof without reproduction.

### P0: Make Evaluation Trustworthy

- Keep PR #180's project/global split.
- Add a golden query dataset with stable evidence references.
- Add per-category metrics: `Hit@k`, `MRR@10`, `Recall@k`, `Precision@k`, `nDCG@10`, and `evidence_recall@k`.
- Add abstention and false-premise cases.
- Show missing examples and rank histograms.

Reason: without reliable metrics, search improvements will be guesswork.

### P1: Improve FTS/RRF Without Dense Vectors

- Add `search_context`/`search_text` as a derived FTS field.
- Add FTS column weights for title, content, files, concepts, facts, and derived search context.
- Add multi-channel RRF for original query, exact phrase query, expanded query, entity channel, temporal channel, and raw fallback.
- Add search explain/debug output.

Reason: this keeps the current SQLite/FTS architecture while addressing short-memory context loss and channel opacity.

### P2: Add Lifecycle and Temporal Facts

- Add candidate lifecycle operations: add/update/invalidate/noop/defer.
- Add soft invalidation and supersession, never hard-delete important derived memories by default.
- Add temporal fact fields with provenance.
- Add minimal typed relationship edges for coding workflows.

Reason: retrieval quality depends on storing fewer, better, current memories, not only ranking more memories.

### P3: Add Procedural Memory Carefully

- Track repeated successful workflows.
- Require verification commands before promoting a procedure.
- Export mature procedures as runbooks or skills only after review.

Reason: procedural memory can compound success, but also freezes bad habits if promoted too early.

## Explicit Non-Goals for the Next PR

- No dense vector database.
- No mandatory embedding model.
- No neural reranker.
- No full GraphRAG rewrite.
- No automatic writes to repository memory files.
- No hard deletion based on LLM judgment alone.

## Open Questions

- Should `search_context` be generated at memory promotion time, or as a background rebuildable job?
- What is the smallest stable evidence reference: `topic_key`, source event id, observation id, or a composite key?
- How many golden queries are enough for CI: 30 smoke tests, 100 core tests, or a larger nightly suite?
- Should global preferences be evaluated separately from project memories in every retrieval metric?
- What redaction policy should apply before raw traces are used as retrieval evidence?
