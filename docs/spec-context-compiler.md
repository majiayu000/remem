# Spec: Host-Aware Context Compiler

**Status**: Draft
**Date**: 2026-04-29
**Related**: `docs/context-budget-design-2026-04-29.md`

---

## 1. Background

`remem context` currently behaves like a renderer over one shared memory pool:

```text
load project memories
  -> dedupe / branch sort / self-diagnostic cap
  -> take(CONTEXT_MEMORY_LIMIT = 50)
  -> render preferences separately
  -> render core + index + workstreams + sessions
```

This caused a real Codex SessionStart case where the main index was dominated by:

```text
Preferences (46)
Decisions (2)
Discoveries (2)
```

The problem is not that `50` is too high or too low. The problem is that one flat limit is carrying unrelated context responsibilities:

- profile/preferences that should always apply;
- high-value core project memory;
- a compact non-preference memory index;
- recent session summaries;
- active workstreams;
- host-specific retrieval instructions.

Open-source memory systems converge on the same shape: small always-on profile/core context plus compact indexes, with details loaded on demand. remem already has the retrieval tools (`search`, `timeline`, `get_observations`), but SessionStart should become an entrypoint map rather than a flat memory dump.

## 2. Goals

- Introduce a host-aware `ContextCompiler` boundary.
- Make Claude Code and Codex context behavior explicit and configurable.
- Split context into stable sections with independent budgets.
- Treat preferences/profile as first-class always-on context, not ordinary index entries.
- Keep SessionStart compact and push full details to MCP/CLI retrieval.
- Preserve current behavior as much as possible during the first implementation slice.
- Make context limits testable via scoped Rust tests and live `remem context` smoke checks.

## 3. Non-Goals

- No vector database or graph database dependency.
- No rewrite of memory storage, promotion, dream, or lifecycle logic.
- No per-section plugin system in the first implementation.
- No change to MCP tool contracts.
- No broad `cargo test --workspace` requirement for this work; targeted package/module tests are enough.
- No automatic deletion of noisy memories as part of context compilation.

## 4. Design Principle

The long-term abstraction should be:

```text
ContextRequest
  -> Candidate Retrieval
  -> HostProfile.default_policy()
  -> ContextCompiler
  -> SectionPlan[]
  -> RenderedContext
```

The important distinction:

- **Host variation exists now**, so host profiles should be modeled now.
- **Section algorithms are still mostly shared**, so section behavior should start as enum + policy data, not a trait/plugin system.

## 5. Existing Boundaries

remem already has two host-related abstractions:

| Existing boundary | Responsibility |
| --- | --- |
| `InstallHost` | Mutate host config files and install/uninstall MCP/hooks. |
| `ToolAdapter` | Parse and normalize hook JSON from a host. |

This spec adds a third boundary:

| New boundary | Responsibility |
| --- | --- |
| `ContextHostProfile` | Choose default context policy, retrieval hints, and output style for a host. |

These boundaries should align on a shared `HostKind`, but they should not be collapsed into one giant trait.

## 6. Core Types

### 6.1 HostKind

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKind {
    ClaudeCode,
    CodexCli,
    Unknown,
}
```

Source of host selection:

1. `--host` CLI arg on `remem context` if present.
2. `REMEM_CONTEXT_HOST` env var.
3. Auto-detection fallback from environment/config.
4. `Unknown` default policy.

Install hooks should eventually set this explicitly:

```text
REMEM_CONTEXT_HOST=claude-code remem context
REMEM_CONTEXT_HOST=codex-cli remem context
```

### 6.2 ContextRequest

```rust
pub struct ContextRequest {
    pub cwd: String,
    pub project: String,
    pub session_id: Option<String>,
    pub current_branch: Option<String>,
    pub host: HostKind,
    pub use_colors: bool,
}
```

`generate_context()` should become a thin CLI entry that builds `ContextRequest` and delegates to the compiler.

### 6.3 ContextHostProfile

```rust
pub trait ContextHostProfile {
    fn host(&self) -> HostKind;
    fn capabilities(&self) -> HostCapabilities;
    fn default_policy(&self) -> ContextPolicy;
    fn retrieval_hints(&self) -> RetrievalHints;
}
```

Expected initial profiles:

| Profile | Notes |
| --- | --- |
| `ClaudeCodeContextProfile` | Claude has broader hooks and UserPromptSubmit. It can mention Claude-facing hook behavior and the same MCP retrieval tools. |
| `CodexCliContextProfile` | Codex currently observes mainly Bash through hooks. Its hints should emphasize `search` / `get_observations` and avoid implying full tool coverage. |
| `UnknownContextProfile` | Conservative defaults, no host-specific claims. |

### 6.4 HostCapabilities

```rust
pub struct HostCapabilities {
    pub has_mcp_tools: bool,
    pub has_session_start_hook: bool,
    pub has_user_prompt_submit_hook: bool,
    pub observes_native_file_edits: bool,
    pub observes_bash: bool,
    pub supports_context_colors: bool,
}
```

These are for context rendering and diagnostics. They do not replace `InstallHost` or `ToolAdapter`.

### 6.5 ContextPolicy

```rust
pub struct ContextPolicy {
    pub total_char_limit: usize,
    pub candidate_fetch_limit: usize,
    pub sections: Vec<SectionPolicy>,
}
```

The policy is declarative. It says what sections exist and what budget each section gets.

### 6.6 SectionPolicy

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionKind {
    Preferences,
    Core,
    Workstreams,
    MemoryIndex,
    Sessions,
    RetrievalHints,
}

pub struct SectionPolicy {
    pub kind: SectionKind,
    pub item_limit: usize,
    pub char_limit: usize,
    pub include_types: Vec<MemoryType>,
    pub exclude_types: Vec<MemoryType>,
    pub per_type_soft_limit: Vec<(MemoryType, usize)>,
}
```

Use enum-driven rendering first:

```rust
match section.kind {
    SectionKind::Preferences => render_preferences_section(...),
    SectionKind::Core => render_core_section(...),
    SectionKind::MemoryIndex => render_memory_index_section(...),
    SectionKind::Workstreams => render_workstreams_section(...),
    SectionKind::Sessions => render_sessions_section(...),
    SectionKind::RetrievalHints => render_retrieval_hints_section(...),
}
```

Do not introduce a `ContextSection` trait in the first slice. Add it later only if section behavior becomes genuinely host-specific or externally extensible.

## 7. Default Policy

### 7.1 Shared Defaults

```rust
ContextPolicy {
    total_char_limit: 12_000,
    candidate_fetch_limit: 120,
    sections: vec![
        Preferences { item_limit: 30, char_limit: 1500 },
        Core { item_limit: 6, char_limit: 3000 },
        Workstreams { item_limit: 5, char_limit: 1200 },
        MemoryIndex { item_limit: 50, char_limit: 4000 },
        Sessions { item_limit: 5, char_limit: 2200 },
        RetrievalHints { item_limit: 1, char_limit: 500 },
    ],
}
```

### 7.2 Section Type Rules

| Section | Include | Exclude |
| --- | --- | --- |
| `Preferences` | `preference` | all others |
| `Core` | `bugfix`, `architecture`, `decision`, `discovery` | `preference`, `session_activity` |
| `MemoryIndex` | all project memory types except preference | `preference` |
| `Sessions` | session summaries | memories |
| `Workstreams` | active workstreams | memories |
| `RetrievalHints` | host profile text | memories |

The key invariant:

```text
preference must not enter Core or MemoryIndex by default.
```

### 7.3 Host Differences

Initial host differences should stay small:

| Setting | Claude Code | Codex CLI |
| --- | ---: | ---: |
| `candidate_fetch_limit` | 120 | 120 |
| `MemoryIndex.item_limit` | 50 | 50 |
| `Core.item_limit` | 6 | 6 |
| `Sessions.item_limit` | 5 | 5 |
| `RetrievalHints` | Claude hook/MCP wording | Codex hook/MCP wording |

Future differences may include lower total char limit for a host, different retrieval hint wording, or hiding sections a host cannot use.

## 8. Environment Variables

New env names:

| Variable | Default | Meaning |
| --- | ---: | --- |
| `REMEM_CONTEXT_HOST` | auto | `claude-code`, `codex-cli`, or `unknown` |
| `REMEM_CONTEXT_TOTAL_CHAR_LIMIT` | `12000` | whole rendered context soft cap |
| `REMEM_CONTEXT_CANDIDATE_FETCH_LIMIT` | `120` | candidate memories fetched before section selection |
| `REMEM_CONTEXT_MEMORY_INDEX_LIMIT` | `50` | non-preference memory index item cap |
| `REMEM_CONTEXT_MEMORY_INDEX_CHAR_LIMIT` | `4000` | non-preference memory index char cap |
| `REMEM_CONTEXT_CORE_ITEM_LIMIT` | `6` | core memory item cap |
| `REMEM_CONTEXT_CORE_CHAR_LIMIT` | `3000` | core memory char cap |
| `REMEM_CONTEXT_SESSION_COUNT` | `5` | recent session summary cap |
| `REMEM_CONTEXT_PREFERENCE_PROJECT_LIMIT` | `20` | project preference query cap |
| `REMEM_CONTEXT_PREFERENCE_GLOBAL_LIMIT` | `10` | global preference query cap |
| `REMEM_CONTEXT_PREFERENCE_CHAR_LIMIT` | `1500` | rendered preference char cap |
| `REMEM_CONTEXT_SELF_DIAGNOSTIC_LIMIT` | `2` | self-diagnostic memory cap |

Compatibility:

- Keep `REMEM_CONTEXT_OBSERVATIONS` as a deprecated alias for `REMEM_CONTEXT_MEMORY_INDEX_LIMIT`.
- If both old and new vars are set, the new var wins.
- Update `docs/ARCHITECTURE.md` so it no longer lists unimplemented context env vars as active behavior.

## 9. Compiler Flow

```text
ContextRequest
  -> resolve_host_profile(request.host)
  -> policy = host_profile.default_policy().with_env_overrides()
  -> candidates = load_context_candidates(policy.candidate_fetch_limit)
  -> plan = build_section_plan(policy, candidates, summaries, workstreams)
  -> output = render_context_header()
  -> output += render_sections(plan)
  -> output += render_context_footer(plan.stats)
```

### 9.1 Candidate Retrieval

Replace `load_context_data()` with a clearer split:

```rust
pub struct ContextCandidates {
    pub memories: Vec<Memory>,
    pub summaries: Vec<SessionSummaryBrief>,
    pub workstreams: Vec<WorkStream>,
}

fn load_context_candidates(
    conn: &Connection,
    request: &ContextRequest,
    policy: &ContextPolicy,
) -> ContextCandidates;
```

Candidate retrieval may still use existing logic:

- recent memories;
- basename search;
- dedup clusters;
- branch-aware ordering;
- self-diagnostic cap.

But final section inclusion must be controlled by section policies.

### 9.2 Section Planning

```rust
pub struct SectionPlan {
    pub kind: SectionKind,
    pub selected_memories: Vec<Memory>,
    pub selected_summaries: Vec<SessionSummaryBrief>,
    pub rendered_char_estimate: usize,
}
```

Rules:

- `Preferences` pulls from preference query path, not the main memory index path.
- `Core` and `MemoryIndex` use non-preference memory candidates by default.
- `Sessions` uses session summaries and its own limit.
- `Workstreams` uses active workstreams and its own limit.
- `RetrievalHints` comes from the host profile.

### 9.3 Footer Stats

The existing footer:

```text
50 memories loaded.
```

should become more precise after section splitting:

```text
120 context memories loaded. 6 core memories. 50 indexed memories. 5 sessions.
```

This separates the candidate pool from section counts so a compact index does not imply a smaller Core pool.

## 10. Migration Plan

### Phase 1: Host-aware request and policy shell

- Add `HostKind`.
- Add `--host` to `remem context`.
- Add `REMEM_CONTEXT_HOST`.
- Make install hooks set `REMEM_CONTEXT_HOST`.
- Add `ContextRequest`, `ContextPolicy`, `SectionPolicy`.
- Keep rendered output mostly unchanged.

Validation:

```bash
cargo test context:: --lib
cargo test install:: --lib
```

### Phase 2: Split Preferences from Main Memory Pool

- Move preference selection fully into `Preferences` section.
- Exclude `preference` from `Core` and `MemoryIndex`.
- Make preference limits env-driven.

Validation:

```bash
cargo test context:: --lib preference
cargo run --quiet -- context --cwd /Users/lifcc/Desktop/code/AI/tools/computer
```

Expected live smoke shape:

```text
## Your Preferences
...
## Core
decision / discovery / bugfix / architecture items
## Index
Decisions (...)
Discoveries (...)
```

The main index should no longer be dominated by `Preferences (46)`.

### Phase 3: Section Budgets

- Apply item and char caps per section.
- Add `core_item_limit`, `core_char_limit`, `memory_index_char_limit`, `session_limit`.
- Update footer stats.
- Update `docs/ARCHITECTURE.md`.

Validation:

```bash
cargo test context:: --lib env
cargo test context:: --lib budget
```

### Phase 4: Type Diversity

- Add soft limits for `decision`, `bugfix`, `architecture`, `discovery`.
- Keep branch relevance and recent high-score behavior.
- Ensure old but high-value decisions are not permanently starved.

Validation:

```bash
cargo test context:: --lib diversity
```

### Phase 5: Optional Retrieval Quality

Only after the above is stable:

- Lightweight topic diversity using existing `topic_key` / cluster keys.
- Token or char cost estimates in logs.
- Optional host-specific policy tuning.

Do not block Phase 1-3 on MMR, embeddings, or graph retrieval.

## 11. Acceptance Tests

Minimum tests for the full spec:

- `context_host_env_overrides_auto_detection`
- `install_hooks_set_context_host_for_claude_and_codex`
- `preference_flood_does_not_starve_core_memories`
- `preferences_rendered_separately_not_in_index`
- `context_limits_env_override_is_respected`
- `deprecated_context_observations_alias_still_works`
- `core_char_budget_does_not_split_utf8`
- `codex_profile_uses_codex_retrieval_hints`
- `claude_profile_uses_claude_retrieval_hints`
- `footer_reports_indexed_memory_count_not_total_db_count`

Live smoke:

```bash
cargo run --quiet -- context --cwd /Users/lifcc/Desktop/code/AI/tools/computer
REMEM_CONTEXT_HOST=codex-cli cargo run --quiet -- context --cwd /Users/lifcc/Desktop/code/AI/tools/computer
REMEM_CONTEXT_HOST=claude-code cargo run --quiet -- context --cwd /Users/lifcc/Desktop/code/AI/tools/computer
```

Review the section composition, not just the final item count.

## 12. Open Questions

1. Should `--host` be exposed as public CLI API, or should hooks only use `REMEM_CONTEXT_HOST`?
2. Should footer stats include rendered char estimates?
3. Should `ContextPolicy` be serialized later for user config, or remain env-only?
4. Should `PreferenceSection` eventually promote stable preferences into a profile document rather than rendering raw memory rows?
5. Should context output include explicit `search` examples, or only tool names?

## 13. Recommended First PR

First PR should be structural but narrow:

1. Add `HostKind`, `ContextRequest`, `ContextPolicy`, `SectionPolicy`.
2. Add env parsing with old alias compatibility.
3. Add `REMEM_CONTEXT_HOST` to installed SessionStart hook commands.
4. Keep current section rendering as much as possible.
5. Add tests for host/env parsing and hook command generation.

Second PR should change behavior:

1. Exclude `preference` from main memory candidates.
2. Make preference rendering use policy limits.
3. Add preference flood tests.
4. Run the `computer` live smoke.

This split keeps the architectural boundary reviewable before changing the user-visible context composition.
