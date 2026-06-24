# Public Memory Benchmark Technical Spec

Status: Current contract
Tracking:
- Spec: #629
- M6 public proof umbrella: #384
- Coding-agent outcome benchmark: #385
- Implementation: #630, #631, #632, #633, #634, #635, #636, #637, #638

## Current Implementation Truth

The repository already has deterministic memory eval surfaces:

- `remem eval` over `eval/golden.json`;
- `remem eval-extraction`;
- `remem eval-gates`;
- `remem eval-e2e`;
- `remem eval-governance`;
- `remem eval-weight-grid`;
- `docs/specs/issue385-coding-agent-ab/` for coding-agent A/B.

The public benchmark must build beside those surfaces. It should reuse current
runtime paths and schemas instead of introducing a second memory store, second
retrieval stack, or private benchmark database.

## Proposed Layout

```text
eval/public/
  README.md
  lockfiles/
    models.lock.json
    docker-images.lock.json
    prompts.lock.json
  schemas/
    benchmark-manifest.schema.json
    memory-run.schema.json
    coding-run.schema.json
    report.schema.json
  memory/
    suites/
      longmemeval/
      locomo/
      minteval/
      longmemeval-v2/
      remem-code-memory/
      adversarial-policy/
    manifests/
      remem-code-memory-v1.json
      adversarial-policy-v1.json
    runners/
      run_memory_suite.py
      adapters/
        remem/
        bm25/
        vector/
        hybrid_rag/
        summary/
        oracle/
    reports/
      memory-report-v1.json
      memory-report-v1.md
    artifacts/
      <run_id>/
        remem.db.snapshot.tar.zst
        reader_input.txt
        retrieved_evidence.json
        answer.json
        score.json
        diagnosis.json
  coding/
    suites/
      issue385-v1/
        tasks/
        histories/
        repos/
        oracles/
        curated_files/
    runners/
      run_coding_bench.py
      run_one_task.py
      score_patch.py
    reports/
      coding-report-v1.json
      coding-report-v1.md
    artifacts/
      <run_id>/
        remem.db.snapshot.tar.zst
        injected_context.txt
        tool_log.jsonl
        patch.diff
        test.log
        score.json
```

The first implementation may use Rust modules under `src/eval/` instead of
Python scripts when that matches repo patterns. The artifact paths and schemas
remain the public contract either way.

## CLI Contract

Preferred public command group:

```bash
remem bench verify --root eval/public --json-out /tmp/remem-bench-verify.json
remem bench memory --suite remem-code-memory --condition remem_default --json-out /tmp/remem-memory.json
remem bench coding --suite issue385-v1 --runs-per-condition 3 --dry-run --json-out /tmp/remem-coding-dry-run.json
remem bench report --root eval/public --json-out eval/public/reports/baseline.json --markdown-out eval/public/reports/baseline.md
```

Compatibility rule: if implementation keeps the existing #385 command name
`remem eval-coding-bench`, the public report must document the mapping from
`bench coding` to `eval-coding-bench`. Do not create two independent coding
benchmark runners.

Required behavior:

- all commands that write reports require explicit output paths;
- dry-run validates schema, matrix, paths, and isolation without invoking an
  agent or external model;
- command arguments that execute subprocesses are passed as arrays, not shell
  strings;
- provider keys come only from environment or normal provider configuration;
- no secrets, API keys, or private prompts are written to committed reports.

## Artifact Schemas

### Benchmark Manifest

```json
{
  "schema_version": 1,
  "benchmark_id": "issue385-v1",
  "layer": "coding_agent_outcome",
  "version": "v1",
  "created_at_epoch": 1760000000,
  "source_policy": {
    "private_user_memory_allowed": false,
    "requires_temp_remem_data_dir": true,
    "external_dataset_revision": null
  },
  "conditions": ["no_memory", "remem", "curated_file"],
  "reports": ["eval/public/coding/reports/coding-report-v1.json"]
}
```

`layer` is one of:

- `memory_system_capability`;
- `coding_agent_outcome`.

### Memory Task Manifest

```json
{
  "task_id": "state-key-stale-api-001",
  "suite": "remem-code-memory-v1",
  "category": "stale_memory_avoidance",
  "reference_time_epoch": 1760000000,
  "history_episodes": [
    {
      "episode_id": "prior-001",
      "events_path": "histories/prior-001.jsonl",
      "expected_memory_facts": ["fact:api_v2_current"]
    }
  ],
  "question": {
    "prompt_path": "tasks/state-key-stale-api-001/question.md",
    "expected_answer_path": "tasks/state-key-stale-api-001/answer.md",
    "abstention_allowed": false
  },
  "gold_memory": {
    "required_facts": ["fact:api_v2_current"],
    "forbidden_facts": ["fact:api_v1_current"],
    "supporting_event_ids": ["prior-001:e17", "prior-001:e22"]
  }
}
```

### Coding Task Manifest

```json
{
  "task_id": "state-key-stale-api-001",
  "suite": "issue385-v1",
  "repo": {
    "url": "https://example.com/org/repo.git",
    "base_commit": "0000000000000000000000000000000000000000",
    "language": "rust"
  },
  "history_episodes": [
    {
      "episode_id": "prior-001",
      "reference_time_epoch": 1760000000,
      "events_path": "histories/prior-001.jsonl",
      "expected_memory_facts": ["fact:api_v2_current"]
    }
  ],
  "target": {
    "prompt_path": "tasks/state-key-stale-api-001/prompt.md",
    "allowed_paths": ["src/**", "tests/**"],
    "forbidden_paths": ["Cargo.lock"],
    "oracle": {
      "commands": [["cargo", "test", "-q", "stale_api_contract"]],
      "required_patch_patterns": [],
      "forbidden_patch_patterns": ["old_api("]
    }
  },
  "gold_memory": {
    "required_facts": ["fact:api_v2_current"],
    "forbidden_facts": ["fact:api_v1_current"],
    "supporting_event_ids": ["prior-001:e17", "prior-001:e22"]
  }
}
```

### Memory Run Artifact

`schemas/memory-run.schema.json` validates memory-system capability runs. It
must not require coding-agent fields such as `resolved`, patch/test logs, or
`repo_base_commit`.

```json
{
  "schema_version": 1,
  "benchmark_version": "remem-code-memory-v1",
  "layer": "memory_system_capability",
  "suite": "remem-code-memory",
  "condition": "remem_default",
  "task_id": "state-key-stale-api-001",
  "run_index": 2,
  "reference_time_epoch": 1760000000,
  "reader_model": {
    "provider": "openai",
    "model": "gpt-5.2",
    "temperature": 0,
    "prompt_hash": "sha256:..."
  },
  "environment": {
    "os": "linux",
    "arch": "x86_64",
    "docker_image_digest": "sha256:...",
    "remem_commit": "...",
    "fixture_revision": "remem-code-memory-v1"
  },
  "answer": {
    "text": "The v2 API is current as of the reference time.",
    "abstained": false,
    "score": 1.0,
    "score_method": "rubric",
    "temporal_as_of_correct": true,
    "no_answer_correct": null
  },
  "retrieval": {
    "retrieved_memory_ids": [101, 104],
    "retrieved_supporting_evidence_ids": ["prior-001:e17"],
    "gold_supporting_event_ids": ["prior-001:e17", "prior-001:e22"],
    "support_coverage": 0.5,
    "missing_supporting_evidence_ids": ["prior-001:e22"],
    "irrelevant_memory_count": 1
  },
  "evidence": {
    "cited_memory_ids": [101],
    "cited_event_ids": ["prior-001:e17"],
    "citation_precision": 1.0,
    "citation_recall": 0.5,
    "source_anchor_staleness": {
      "prior-001:e17": "tracked"
    }
  },
  "metrics": {
    "ingest_tokens": 12345,
    "query_tokens": 1200,
    "reader_tokens": 2400,
    "retrieval_latency_ms": 82,
    "end_to_end_latency_ms": 2100,
    "rows_written": 14
  },
  "diagnosis": {
    "write_side_gap": false,
    "retrieval_side_gap": true,
    "reader_gap": false,
    "policy_abstention": false,
    "notes": ["one required evidence ID was not retrieved"]
  },
  "artifacts": {
    "reader_input": "artifacts/.../reader_input.txt",
    "retrieved_evidence": "artifacts/.../retrieved_evidence.json",
    "answer": "artifacts/.../answer.json",
    "score": "artifacts/.../score.json",
    "diagnosis": "artifacts/.../diagnosis.json",
    "remem_db_snapshot": "artifacts/.../remem.db.snapshot.tar.zst"
  }
}
```

### Coding Run Artifact

`schemas/coding-run.schema.json` validates #385 coding-agent outcome runs and
retains coding-specific oracle, patch, test, and repository fields.

```json
{
  "schema_version": 1,
  "benchmark_version": "issue385-v1",
  "layer": "coding_agent_outcome",
  "condition": "remem",
  "task_id": "state-key-stale-api-001",
  "run_index": 2,
  "model": {
    "agent": "codex-cli",
    "model": "gpt-5.2",
    "temperature": 0,
    "reasoning": "medium"
  },
  "environment": {
    "os": "linux",
    "arch": "x86_64",
    "docker_image_digest": "sha256:...",
    "remem_commit": "...",
    "repo_base_commit": "..."
  },
  "resolved": true,
  "failure_reason": null,
  "metrics": {
    "tokens_input": 123456,
    "tokens_output": 7890,
    "tokens_total": 131346,
    "memory_tokens": 3210,
    "turns": 8,
    "wall_time_ms": 420000,
    "tool_calls": 31,
    "commands_run": 12
  },
  "memory_contract": {
    "injected_memory_ids": [101, 104],
    "used_memory_ids": [101],
    "citation_precision": 1.0,
    "citation_recall": 0.5,
    "stale_used_count": 0,
    "missing_relevant_memory_count": 1,
    "irrelevant_injection_count": 1,
    "memory_helped": true,
    "memory_hurt": false
  },
  "artifacts": {
    "patch": "artifacts/.../patch.diff",
    "tool_log": "artifacts/.../tool_log.jsonl",
    "test_log": "artifacts/.../test.log",
    "injected_context": "artifacts/.../injected_context.txt",
    "remem_db_snapshot": "artifacts/.../remem.db.snapshot.tar.zst"
  }
}
```

## Runner Architecture

### Memory Suite Runner

For each memory task and condition:

1. Create a temporary `REMEM_DATA_DIR`.
2. Load history episodes through the configured write path:
   - supported CLI/MCP/API path for `remem_default`;
   - fixture loader only when the condition explicitly tests stored-memory
     representation and documents that it bypasses ingestion;
   - no write path for `no_memory` and `oracle_evidence`.
3. Build the reader input according to the condition.
4. Run the reader/scorer with fixed model, prompt hash, seed where supported,
   and token budget.
5. Score answer, retrieval, evidence, citation, staleness, and governance
   metrics.
6. Write one run artifact per task/condition/run index.

The reader must be fixed across compared conditions. Changing the reader creates
a new benchmark version.

### Coding-Agent Runner

The coding runner follows #385 and adds public artifact requirements:

1. Create a fresh workdir from the pinned repository and base commit.
2. Create a fresh temporary `REMEM_DATA_DIR`.
3. Apply the selected condition:
   - `no_memory`: disable remem hooks, MCP, native memory, and curated files;
   - `remem`: seed remem through supported ingestion and enable the supported
     runtime path;
   - `curated_file`: provide only the approved curated context file.
4. Start the coding agent with the same model, budget, and target prompt.
5. Capture tool log, injected context, token usage, command count, and wall
   time.
6. Run objective oracle commands in the task manifest.
7. Check allowed and forbidden paths and patch patterns.
8. Produce patch artifact, score artifact, and run JSON.
9. Clean up unless `--keep-workdirs` is set.

Condition order should be randomized per task/run seed to reduce ordering bias.
No condition may read artifacts or temp data from another condition.

## Isolation Requirements

All benchmark runs must prove:

- `REMEM_DATA_DIR` points to a temp path created for that run;
- real `~/.remem` was not opened;
- no host hook config was mutated;
- no real project memory was imported;
- workdir is clean or intentionally preserved after failure;
- Docker image digest or equivalent environment lock is recorded;
- fixture revision is recorded for remem-native suites;
- external dataset revision and checksum are recorded only when
  `source_policy.external_dataset_revision` is non-null.

The verifier must fail on:

- absolute paths under a real user home when a relative artifact path is
  expected;
- missing temp-data evidence for remem conditions;
- missing patch/test logs for coding runs;
- missing supporting evidence IDs for memory runs;
- unknown failure reason enums;
- reports that mix memory capability and coding outcome metrics in one metric
  table, aggregate, or artifact list even if they include a `layer` field.

Reports must be either single-layer, with one top-level `layer` and only that
layer's schemas and metrics, or multi-section with separate per-layer sections.
Each per-layer section must carry its own `layer` tag, schema references, run
artifact list, aggregate metrics, claim level, and verifier result. A top-level
summary may link the sections but must not merge memory-system and
coding-outcome metrics into one claimed result.

## External Dataset Adapters

Adapters must be conservative:

- pin dataset source URL, revision, checksum, license, and preprocessing script;
- keep source-specific answer formats outside generic scoring code;
- commit only redistributable data or a smoke subset when raw data cannot be
  committed;
- record skipped cases and reasons;
- keep LoCoMo informational until a methodology update explains scoring,
  adversarial skips, judge prompts, and comparability.

Initial source references:

- LongMemEval: https://arxiv.org/abs/2410.10813
- LoCoMo: https://arxiv.org/abs/2402.17753
- MINTEval: https://arxiv.org/abs/2605.18565
- LongMemEval-V2: https://arxiv.org/abs/2605.12493
- WhenLoss diagnostics: https://arxiv.org/abs/2605.24579
- SWE-bench harness pattern: https://github.com/swe-bench/SWE-bench
- SWE-bench Verified: https://www.swebench.com/verified.html
- Mem0 comparison context: https://arxiv.org/abs/2504.19413

These references are design inputs. They are not remem result claims.

## Scoring

### Memory Scoring

Memory scoring produces:

- retrieval metrics;
- answer/rubric metrics;
- evidence and citation metrics;
- governance metrics;
- latency and token metrics;
- diagnostic failure attribution.

Diagnostic fields:

```json
{
  "diagnosis": {
    "write_side_gap": true,
    "retrieval_side_gap": false,
    "reader_gap": false,
    "policy_abstention": false,
    "notes": ["required evidence missing from complete_stored_memory"]
  }
}
```

### Coding Scoring

Coding scoring produces:

- oracle command status;
- unauthorized path changes;
- required/forbidden patch pattern results;
- patch artifact hash;
- failure enum;
- memory attribution.

Oracle commands are arrays:

```json
[["cargo", "test", "-q", "stale_api_contract"]]
```

Shell-concatenated command strings are not accepted in manifests.

## Report Generation

Reports are generated from run artifacts, not from ad hoc summaries. The
Markdown report must include:

- benchmark version;
- layer;
- conditions;
- model/provider;
- dataset or fixture revisions;
- environment locks;
- aggregate metrics;
- per-category memory table;
- per-task coding table;
- variance and run count;
- failure decomposition;
- claim level satisfied or not satisfied;
- reproduction commands;
- artifact verifier result.

Reports must use cautious wording:

- "capability benchmark" for Layer 1;
- "coding-agent outcome benchmark" for #385;
- "directional evidence" until stop-loss and claim gates pass;
- no "SOTA" unless Level 3 evidence is present.

## Implementation Plan And Ownership

| Issue | Scope | Depends on | PR type |
|---|---|---|---|
| #629 | This spec and index entry | #384, #385 | Spec only |
| #630 | Artifact schema and verifier | #629 | Implementation |
| #631 | remem-native coding memory QA suite | #630 | Implementation |
| #632 | adversarial memory policy suite | #630 | Implementation |
| #633 | write-vs-retrieval diagnostics and baselines | #630, #631 | Implementation |
| #634 | #385 runner with required conditions | #629, #630, #609 | Implementation |
| #635 | coding task fixture pack v1 | #634 | Implementation |
| #636 | memory attribution and failure taxonomy | #634, #635, #609 | Implementation |
| #637 | baseline directional report | #631, #632, #633, #634, #635, #636 | Implementation/docs |
| #638 | public claim policy and stop-loss gate | #637 | Documentation or implementation |

Parallelism:

- #631 and #632 may run in parallel after #630 if fixtures are disjoint.
- #634 can start after #630 and #609 handoff gates are clear.
- #635 and #636 are serial with #634 because they depend on runner schema.
- #637 waits for both evidence layers.

## Verification

Spec-only PR:

```bash
cargo fmt --check
cargo check
```

Initial artifact/schema slice:

```bash
cargo test bench_artifact --lib
cargo run -- bench verify --root eval/public --json-out /tmp/remem-bench-verify.json
```

Memory suite slices:

```bash
cargo test memory_bench --lib
cargo run -- bench memory --suite remem-code-memory --condition remem_default --json-out /tmp/remem-code-memory.json
cargo run -- bench memory --suite adversarial-policy --json-out /tmp/remem-adversarial-policy.json
```

Coding benchmark slices:

```bash
cargo test coding_bench --lib
cargo run -- bench coding --suite issue385-v1 --runs-per-condition 3 --dry-run --json-out /tmp/remem-coding-dry-run.json
```

Before publishing or changing README claims:

```bash
cargo run -- eval-gates --json-out /tmp/remem-eval-gates.json
cargo run -- bench verify --root eval/public --json-out /tmp/remem-public-bench-verify.json
rg -n "SOTA|best|beats|outperforms|state-of-the-art" README.md docs
```

Full submission for runtime code changes still follows repo rules:

```bash
cargo fmt --check
cargo check
cargo test
```

## Failure Handling

Benchmark failures are valid data when:

- runner isolation succeeded;
- scoring oracle completed;
- artifact schema is valid;
- failure reason is structured;
- logs are present.

Process failures include:

- fixture checkout failure;
- invalid oracle command;
- missing temp data isolation;
- real user memory access;
- missing required metrics;
- malformed artifact path;
- unknown failure enum;
- report claim level unsupported by artifacts.

Process failures fail the benchmark run and must not be counted as task
outcomes.

## Open Technical Decisions

- Whether to implement `remem bench` in Rust first or ship a thin script layer
  around current `eval-*` commands.
- Which reader model and provider are allowed for the first public memory
  report.
- How to collect token counts consistently across Codex CLI, Claude Code, and
  future hosts.
- Whether Docker is required for all coding tasks or only for public full runs.
- How large DB snapshots may be before artifacts move to release assets instead
  of git.
