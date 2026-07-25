# Tech Spec

## Linked Issue

GH-882

## Product Spec

[`product.md`](product.md)

## Codebase Context

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| Candidate parser | `src/memory_candidate/parse.rs` | Canonical candidate types are accepted, then legal observation types are mapped; `fact` reaches the explicit malformed-output error. | The failure originates at the parser boundary and currently aborts the whole response. |
| Candidate prompt | `src/memory_candidate.rs` | The task dynamically enumerates the seven canonical types and maps `feature/refactor/change` to `discovery`, but does not explicitly forbid `fact`. | The prompt should steer factual findings to the canonical type before parsing is needed as a fallback. |
| Type model | `src/memory/types.rs` | `from_observation_type` maps only the legal observation vocabulary to candidate types. | `fact` is not a legal observation type, so this shared mapping should remain strict. |
| Regression tests | `src/memory_candidate/parse.rs`, `src/memory_candidate/tests/existing_preferences.rs` | Tests cover canonical types, observation aliases, and invalid values; no test covers `fact` or the factual-finding prompt instruction. | Focused tests can prove both the parser fallback and the model-facing contract. |

## Proposed Design

Add a narrowly scoped parser alias in `normalize_memory_type`: after canonical
candidate parsing and before observation-type fallback, map normalized `fact`
to `MemoryType::Discovery`. Keep `MemoryType::from_observation_type` unchanged
because broadening it would make downstream observation support checks accept a
value that the observation schema does not allow.

Clarify both layers of the memory-candidate prompt. The system instruction and
the task prompt will state that factual findings use `discovery` and must not
emit `fact`. The existing dynamic canonical-type list remains the single source
of truth for the allowed vocabulary.

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| P1 | `normalize_memory_type` explicit alias | Unit assertion for lowercase `fact` returning `discovery` |
| P2 | Existing canonical parse branch | Canonical-type regression test round-trips all seven candidate types |
| P3 | Existing observation fallback | Existing observation-alias regression test |
| P4 | Explicit error branch | Existing invalid-type assertions remain green |
| P5 | System and task prompt strings | Focused prompt test checks all seven types and the `fact` guidance |
| P6 | `parse_memory_candidates` using the normalized member | Parser test includes a response with `fact` and another valid candidate and verifies both are returned |

## Data Flow

The extraction worker sends the system instruction and generated task prompt to
the configured LLM. Returned `<memory_candidate>` blocks are parsed in order.
The normalized type string is persisted through the existing candidate path;
`fact` therefore reaches storage as `discovery`. No schema, migration, network,
or persistence contract changes.

## Alternatives Considered

- Add `fact` to `MemoryType::from_observation_type`: smaller textual diff, but
  it weakens the legal observation vocabulary for every downstream caller.
- Change only the prompt: reduces new occurrences but does not recover output
  from models that ignore or predate the instruction.
- Coerce every unknown type to `discovery`: avoids retries but silently
  misclassifies arbitrary model output and hides contract drift.

## Risks

- Security: No new input capability; arbitrary unknown types remain rejected.
- Compatibility: `fact` changes from an error to the canonical `discovery`
  value, while all other inputs preserve current behavior.
- Performance: One constant-time alias comparison and prompt text only.
- Maintenance: The explicit parser alias is intentionally local so the
  observation vocabulary remains independently auditable.

## Test Plan

- [ ] Unit tests: parser normalization, mixed-candidate parsing, and prompt
  vocabulary/guidance.
- [ ] Integration tests: focused `memory_candidate` library tests covering the
  affected module.
- [ ] Manual verification: `cargo fmt --check`, `cargo check`, `cargo test`,
  workflow/spec checks, PR preflight, and current-head CI.

## Rollback Plan

Revert the parser alias, prompt clarification, and their focused tests together.
No data rollback or migration is required because persisted values use the
pre-existing canonical `discovery` type.
