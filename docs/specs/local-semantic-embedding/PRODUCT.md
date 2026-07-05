# Local Semantic Embedding Product Spec

Status: Current contract
Date: 2026-07-04

Tracking:
- Epic issue: #682
- Design lineage: #358 (closed, provider config contract in comments), #643
  (closed, long-term semantic-model follow-up)
- Related contracts: #385 (coding-agent A/B), #675 (capacity eval axis)

## Problem

The vector channel carries the highest fusion weight in hybrid retrieval, but
a default install never produces semantic vectors:

- `Auto` provider resolves to OpenAI only when an API key is configured;
  otherwise it falls back to `remem-local-feature-hash-v1`, a deterministic
  hashing-trick bag-of-features with no learned semantics.
- Paraphrase and synonym recall — the reason a vector channel exists — does
  not work out of the box. The channel largely duplicates FTS5 for default
  installs.
- Downstream consumers inherit the same ceiling: the semantic dedup funnel and
  preference same-intent consolidation both measure similarity in the same
  non-semantic space.

remem's positioning is memory quality. The single highest-weighted retrieval
signal being pseudo-semantic for the default install contradicts that
positioning.

## Goals

- A real local semantic embedding model becomes available to every install
  without an API key, with CJK+English coverage as a hard requirement.
- Provider selection is an explicit, user-visible contract (config file), not
  an env-only switch.
- Degradation is never silent: when the effective provider differs from the
  configured one, `remem status` and `remem doctor` say so.
- Any change to the default provider is gated on measured eval wins, not
  assumption.
- Existing vectors remain valid: cosine comparison happens only within one
  model id, and switching providers offers an explicit backfill.

## Non-Goals

- No ANN index in the first cycle. Brute-force cosine over candidate
  embeddings stays until scale evidence demands more.
- No removal of the feature-hash embedder. It remains the labeled
  zero-download fallback.
- No bundling of model weights into the release binary.
- No fusion-weight retuning beyond what the eval gate justifies.
- No server-side or hosted embedding service.

## Product Principles

### Semantic By Default, Honest About Fallbacks

The out-of-box goal is that `remem search` benefits from real semantic recall
without any account or key. When that is impossible (weights not yet
downloaded, download disabled, unsupported platform), the system must say
which embedder is actually active rather than implying semantic quality it
does not have.

### One Contract For Provider Selection

Provider selection follows the config contract designed in #358:

```toml
[embeddings]
# "api"          - remote embedding API (highest quality, needs network + key)
# "local"        - local ONNX small model (offline-capable, first run downloads weights)
# "feature-hash" - hashing-trick bag-of-features (no deps, no real semantics; legacy fallback)
# "off"          - disable the vector channel entirely
provider = "local"
fallback = "feature-hash"
```

Environment variables keep working as overrides for automation, but the
config file is the documented surface.

### Evidence Before Defaults

The default provider changes only after a committed eval comparison
(feature-hash vs local model vs API embeddings) on the golden set and
retrieval gates shows the local model wins or ties FTS-blended quality
without unacceptable latency. The comparison artifacts live in `eval/` and
are referenced from the epic before any default flip ships.

GH-716 evidence lives at `eval/provider-comparison/report.json`. The current
decision is no default flip: feature-hash is the runnable baseline, while the
local semantic row is unavailable without a verified model manifest in the
reference data dir and the API row is unavailable unless an intentional
`--allow-api` run is performed.

## User Stories

### Out-Of-Box Semantic Recall

As a new user with no API key, after install my paraphrase queries find
memories whose wording differs from my query.

Acceptance:

- A fresh install can download and activate the local model with one
  documented command (or on first use, with clear progress output).
- Paraphrase fixtures in the eval suite pass with the local model where
  feature-hash fails them today.

### Visible Provider State

As a user, I can see which embedding provider and model are active, what
fraction of memories have vectors for the active model, and whether the
system is running degraded.

Acceptance:

- `remem status` shows active provider, model id, and vector coverage.
- `remem doctor` reports a finding when the configured provider is
  unavailable and a fallback is active.

### Safe Provider Switching

As a user, when I switch providers, existing search does not silently mix
incomparable vector spaces, and I am told how to backfill.

Acceptance:

- Search only compares vectors sharing the active model id.
- `remem embedding backfill` re-embeds memories lacking vectors for the
  active model and reports progress and final coverage.
- Old-model vectors are pruned only after active-model coverage reaches 100%
  and the user passes an explicit prune flag.

## Rollout

1. Provider contract: config section, resolution order, status/doctor
   visibility. No new model yet.
2. Local ONNX model: fastembed-backed download/status flow, verified model
   manifests, embed pipeline, same-model-id guard, backfill command, and
   explicit prune gating.
3. Eval gate: committed comparison artifacts; default flip decision recorded
   in the epic and this spec's index entry.
4. Downstream adoption: dedup funnel and preference consolidation switch to
   the active semantic space where the eval shows wins. GH-717 lands this phase
   by keeping curated dedup on same-model vectors, adding an observation vector
   stage wired into extraction persistence, and moving preference embedding
   fallback to active-provider embeddings with model-specific thresholds. It
   does not change the default provider because GH-716 recorded a no-flip
   decision.

Each phase ships independently with focused tests plus:

```bash
cargo fmt --check
cargo check
cargo test
```

## Open Questions

- Should first-use auto-download ever become default-on, or should users keep
  activating local semantics explicitly through `remem embedding download` to
  keep hooks latency-safe?

## Resolved Decisions

- GH-716 does not flip `Auto` from API-when-key-present /
  feature-hash-without-key to local semantics. Local semantic embeddings remain
  an explicit opt-in until the reference provider comparison includes verified
  local and API rows that justify changing the default.
