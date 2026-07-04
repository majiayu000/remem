# Tech Spec

## Linked Issue

GH-716

## Product Spec

`product.md`

## Codebase Context

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| Golden fixtures | `eval/golden.json`, `src/eval/golden/*` | Golden cases are deterministic retrieval fixtures with slice/category metadata. | GH-716 needs paraphrase/CJK cases that can be scored per slice. |
| Eval CLI | `src/cli/types.rs`, `src/cli/dispatch.rs`, `src/eval/gates.rs`, `src/eval/golden.rs` | Existing commands run extraction and retrieval gates, but no provider-comparison mode emits provider rows and default-flip decision. | Add a focused comparison entrypoint rather than overloading unrelated gates. |
| Embedding providers | `src/retrieval/embedding.rs`, `src/retrieval/embedding/local_semantic.rs` | `Auto` still resolves to API with key, otherwise feature-hash. Explicit `local` requires an installed verified model. | Comparison must force provider-specific configs and report unavailable providers honestly. |
| Fixture sandbox | `src/eval/current_memory_contracts/sandbox.rs`, `src/eval/golden/run.rs` | Eval code can seed fixture memory in isolated temp databases. | Provider comparison should run against isolated data and not touch user data. |
| Docs/spec index | `docs/specs/README.md`, `specs/GH682/*` | GH682 tracks the default-flip evidence phase. | The decision and report path must be discoverable. |

## Proposed Design

Add a provider-comparison eval mode that runs a deterministic golden retrieval
comparison for the required providers:

- `feature-hash`: always runnable and used as the baseline.
- `local`: runs only when the local model manifest is installed and verified;
  otherwise records an unavailable row.
- `api`: runs only when a remem embedding API key/config is available; otherwise
  records an unavailable row.

The report contains:

- schema/version metadata and generated timestamp;
- provider rows with provider id, model id, availability, unavailable reason,
  hit-rate/MRR/NDCG metrics for runnable rows, and query embedding latency
  p95 where measured;
- paraphrase/CJK slice metrics separate from the overall golden aggregate;
- a default decision object with `change_default: true|false`, criteria
  status, blockers, and evidence paths.

The command must not auto-download local weights and must not silently use a
fallback provider for a provider-specific row. For this issue, the expected
checked-in report may keep the default unchanged if local/API rows are
unavailable in the build environment.

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| P1 provider rows exist | provider-comparison report builder | Unit test asserts required providers are present |
| P2 rows include availability, model, metrics, latency | report schema/types | Serialization test and focused eval test |
| P3 EN/CJK paraphrase fixtures exist | `eval/golden.json` | Golden validation test for provider-comparison slice |
| P4 flip criteria explicit | decision builder | Unit test for no-flip on unavailable local/API |
| P5 unavailable providers are honest | provider resolver/report row | Test with missing local model/API key |
| P6 docs link evidence | docs/spec index and GH682 spec | `checks/check_workflow.py` and grep/assertion test if practical |

## Data Flow

The provider-comparison command creates an isolated eval database, seeds the
golden fixture corpus, forces one provider configuration at a time, embeds
queries/memories for runnable providers, records metrics, and writes a JSON
report. Missing local model or API credentials produce unavailable rows without
writing vectors or falling back to another provider. The final decision is
derived only from runnable, same-provider evidence.

## Alternatives Considered

- Auto-download the local model during comparison: rejected because CI and hook
  safety require no surprise downloads.
- Use a real API call in CI: rejected because tests must not require secrets or
  paid network access.
- Flip default from issue intent alone: rejected because GH-682 requires
  committed eval evidence first.

## Risks

- Security: API credentials must be read only from configured env variables and
  never written to reports.
- Compatibility: existing eval commands must keep their current JSON shape.
- Performance: local model comparison can be slow; it must be opt-in and report
  latency rather than blocking normal gates.
- Maintenance: report schema should be small and versioned so future provider
  rows can be compared.

## Test Plan

- [x] Unit tests for provider-comparison decision and unavailable-provider
      rows.
- [x] Golden fixture validation for English and CJK provider-comparison cases.
- [x] Focused command/report test that runs the feature-hash row without local
      model or API key.
- [x] `cargo test eval`
- [x] `cargo run -- eval-extraction --json --check-baseline`
- [x] `cargo run -- eval-gates --json-out /tmp/remem-eval-gates.json`
- [x] `cargo fmt --check`
- [x] `cargo check --message-format=short`
- [x] `python3 checks/check_workflow.py --repo . --spec-dir specs/GH682`
- [x] `python3 checks/check_workflow.py --repo . --spec-dir specs/GH716`

## Rollback Plan

Revert the provider-comparison command, committed report, and fixture additions.
Because this issue does not change the default provider unless criteria pass,
rollback should not require data migration. Users can continue selecting
`feature-hash`, `local`, `api`, or `off` explicitly.
