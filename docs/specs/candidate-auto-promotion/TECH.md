# Candidate Auto-Promotion Technical Contract

Status: Current contract
Date: 2026-08-03

Tracking: #955

## Risk Rubric

`memory_candidate/prompt.rs` owns the extraction prompt and defines the closed
rubric:

- `low`: already-true repository-local claims directly supported by supplied
  evidence, including an observed failure lesson;
- `medium`: preferences, procedures, recommendations, inferences, proposals,
  future plans, or applicability that needs review;
- `high`: credentials, auth/authz state, private/personal/payment data,
  destructive operations, or other security-sensitive claims.

`parse::normalize_risk_class` accepts only those three values. The extraction
report contains explicit low/medium/high counts, whose sum must equal all
candidate predictions.

## Claim-Level Support

`support::has_claim_level_source_support` normalizes whitespace/case, splits a
candidate into sentence claims, and requires every claim to match at least one
eligible source observation. A source can support one claim and another source
can support the next; one supported sentence cannot mask an unsupported one.

Exact and ordered-overlap matching retain the existing actor, identifier,
security-modifier, ordering, and minimum-overlap checks. Semantic signatures
add explicit handling for negative polarity, uncertainty, prospective language,
prescriptive language, and conditions.

Candidate/source signatures must match. Only an empty signature or negative
factual signature is auto-promotable. `won't` and `can't`, including curly
apostrophes, normalize to their real prospective/capability modal forms.
Outer meta-negation is a separate signature so a false or incorrect quoted
claim cannot support its embedded text. This permits supported negative facts
but keeps future, uncertain, conditional, prescriptive, modal, and ambiguous
quoted claims pending.

The structured claim gate also review-routes imperative control text,
auth/authz control state, and affirmative destructive actions even if the
model labels them `low`. A locally negated destructive fact such as “does not
delete active rows” remains eligible; the gate does not return to a whole-text
common-word blacklist.

## Type and Secret Gates

The canonical `MemoryType::auto_promote` vocabulary remains unchanged for core
types. The observation-path decision adds a narrow failure-lesson case:

- type is `lesson`;
- candidate and an eligible bugfix/decision observation both contain an
  affirmative failure outcome linked to a recovery action in the same claim;
- ordinary scope, risk, confidence, trust, routing, evidence-id, unsafe-marker,
  and claim-support gates all pass.

Non-failure lessons record `lesson_not_failure_qualified`; preference and
procedure candidates record `memory_type_not_auto_promotable`.

The unsafe-marker list removes bare `token`. A deterministic canonicalizer
folds case and full-width ASCII and treats punctuation, underscores, dashes,
and Unicode separators as word boundaries. Credential-qualified forms such as
`access-token`, `GITHUB_TOKEN`, `deployment token`, and `OAuth2 token` are
blocked. Only explicit non-credential contexts such as token budgets, counts,
limits, and windows bypass this token check; ambiguous contexts fail closed.
Existing API-key, bearer, password, private-key, payment, secret, and key-format
markers remain. Instruction-pattern scanning and quarantine run before the
promotion decision, with the claim gate as a second defense for generic
imperatives outside the fixed scanner vocabulary.

## Verification

- `cargo test memory_candidate::tests_autopromote`
- `cargo test memory_candidate::tests_autopromote_gh955`
- `cargo test eval::extraction`
- `cargo run -- eval-extraction --json --check-baseline`
- `cargo fmt --check`
- `cargo check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`
