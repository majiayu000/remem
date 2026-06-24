# User Memory Policy Refinements Technical Spec

Status: Current contract
Date: 2026-06-24

Tracking:
- Spec/tracking issue: #617
- Profile Markdown snapshot: #619
- Usage policy injection rules: #620
- Automatic extraction blocklist: #618

## Existing Implementation Facts

This spec builds on the current user-context layer:

- `user_context_claims` stores source-attributed user claims with owner scope,
  claim type, claim key, confidence, sensitivity, status, validity windows, and
  supersession.
- `user_context_candidates` stores review-gated automatic extraction output.
- `user_context_summaries` stores compiled profile summaries and source ids.
- `memory_suppressions` applies default-read exclusions across memories, user
  claims, topics, entities, patterns, and summaries.
- `memory_feedback` records relevance and quality signals without changing
  ranking by default.
- `recall_user_context` composes safe profile summaries, active claims, repo
  memory, explicitly requested current-state keys, workstreams, and recent
  sessions.
- Markdown export/import already exists for curated repo memories. This spec
  adds a user-profile snapshot, not a replacement storage backend.

## Design Rules

- SQLite remains canonical.
- Markdown profile output is derived and read-only in the first implementation.
- User-context usage guidance must travel with injected/recalled context.
- Automatic extraction must name both positive retention criteria and explicit
  non-retention criteria.
- Runtime prompt changes must be covered by focused tests.
- No code path may silently include suppressed, deleted, expired, future,
  personal, sensitive, or restricted user context in default profile output.

## Phase 1: Profile Markdown Snapshot

### CLI

Preferred command:

```text
remem user profile export --format markdown [--output <path>] [--owner-scope user|workspace|repo|session] [--owner-key <key>] [--project <path>] [--include-suppressed] [--include-sensitive] [--include-inactive] [--include-deleted]
```

Acceptable first slice if command surface should stay smaller:

```text
remem user summary show --format markdown [--output <path>] [--owner-scope user|workspace|repo|session] [--owner-key <key>] [--project <path>] [--include-suppressed] [--include-sensitive] [--include-inactive] [--include-deleted]
```

The first implementation should choose one command shape and document it in
README. If both are supported, they must share one renderer.
The smaller command shape only changes the command name. It must still expose
the owner/project selection and audit flags above; otherwise it does not satisfy
the snapshot audit contract.

### Output Contract

Default Markdown sections:

```markdown
# remem User Profile Snapshot

Generated: 2026-06-24T00:00:00Z
Effective owners: user:user:default, repo:/path/to/repo
Project: /path/to/repo
Source of truth: <resolved db_path>
Mode: default

## Active Summary

...

## Active Claims

- [claim:123] preference:review-style - Prefer concise code reviews.
  - owner: user:user:default
  - sensitivity: normal
  - source: manual
  - updated: 2026-06-24T00:00:00Z

## Sources

- summary:45 claim_ids=[123] memory_ids=[456] activity_refs=[...]

## Excluded From Default Use

No excluded items shown. Re-run with audit flags to inspect them.
```

The source-of-truth path must be the resolved database path used by the running
command, including `REMEM_DATA_DIR` overrides. Do not render a hard-coded
default path.

With audit flags, excluded items must include `reason`:

```markdown
- [claim:124] identity:location - ...
  - reason: sensitivity:personal
  - owner: user:user:default
```

### Data Selection

Default output must include:

- the applicable active profile summary only when its provenance is clear;
- active normal-sensitivity claims selected by a snapshot-specific owner query;
- summary source ids;
- no personal, sensitive, restricted, suppressed, deleted, expired, or future
  claims.

Do not call `list_claims` without snapshot-specific filtering for the default
export. The existing default list path is broader than this contract because it
excludes `restricted` sensitivity but may still return `personal` or `sensitive`
active claims.

Snapshot owner selection must mirror the context the user is auditing:

- include `owner_scope='user' AND owner_key='user:default'`;
- include `owner_scope='repo'` rows for the current project path or normalized
  project key;
- include workspace/session owners only when explicitly requested;
- exclude unrelated repo, workspace, session, tool, and domain owners by
  default.

Profile summaries require provenance labels:

- deterministic summaries with non-empty source ids may appear as
  `provenance: source-supported`;
- manually edited summaries using `model='manual-edit'` must appear as
  `provenance: manual-edit` and show the preserved source ids separately only
  when an audit flag requests manual summary text. Default snapshots must omit
  or redact manual summary text because the row has no sensitivity field and the
  preserved source ids do not prove every edited sentence is source-supported;
- summaries with empty source ids must appear as `provenance: unsourced` and
  must not render summary text by default or be presented as source-supported
  profile truth;
- if a manual edit changes text beyond the cited sources, the snapshot must
  render only an excluded-summary marker by default. Audit output may render the
  manual user-authored summary text with `reason: provenance:manual-edit`, but
  not as a claim backed by the old source ids.

Audit output may include excluded rows only when explicitly requested:

- `--include-suppressed` includes suppressed and policy-suppressed claims with
  reason labels;
- `--include-sensitive` includes personal, sensitive, and restricted claims with
  sensitivity labels;
- `--include-inactive` includes expired, future, stale, and superseded claims
  with validity/status labels;
- `--include-deleted` includes soft-deleted rows with `status:deleted` labels.

Audit flags are cumulative gates, not bypass switches. A row with multiple
exclusion reasons may render only when every applicable exclusion is explicitly
allowed, or when the renderer redacts the blocked text and labels all remaining
reasons. For example, a deleted sensitive claim must not expose its text with
`--include-deleted` alone; it also requires `--include-sensitive`, or it must be
redacted.

Deleted rows must never appear in default output. If a future hard-purge command
physically removes deleted rows, the snapshot should report that purged rows are
not recoverable instead of implying they are hidden.

### Renderer

Add a dedicated renderer under `src/user_context/` rather than formatting in CLI
actions. The renderer should accept a structured request and return a string so
CLI, REST, or tests can share it.

Recommended module:

```text
src/user_context/profile_snapshot.rs
```

The renderer should not open the database itself. It should accept `&Connection`
and a request struct, matching existing user-context modules.

### Tests

Required tests:

- active summary and active normal claims render in stable order;
- suppressed/deleted/expired/future/restricted claims are excluded by default;
- `--include-suppressed` labels suppressed claims and does not make them active;
- `--include-sensitive` labels personal/sensitive/restricted claims;
- combined audit flags do not bypass sensitivity gating for suppressed,
  inactive, or deleted sensitive claims;
- snapshot output says it is derived and names SQLite as source of truth;
- CLI `--output` refuses to overwrite an existing file unless a future `--force`
  flag is added.

## Phase 2: Usage Policy Injection Rules

### Policy Text

Add this guidance to user-context injection or recall output when user context is
returned to an agent:

```text
Use user context only when it materially improves the current answer. Prefer
invisible adaptation over explicit memory narration. Limit explicit memory
mentions to 0-1 per response. Do not say "I remember you said" or "from previous
conversations" unless the user is discussing memory, provenance, or correction.
Do not infer profile facts beyond the cited items. If no user context applies,
do not invent a profile.
```

### Integration Points

Add the policy where agents will actually see it:

- SessionStart user-context overlay, if rendered.
- `remem user recall` human output, when context items are returned.
- `recall_user_context` MCP response instructions or metadata.
- REST recall response only if it already carries human-facing guidance. Do not
  add noisy prose to machine-only JSON unless a structured field is available.

### Tests

Required tests:

- rendered user-context output includes the usage policy once;
- no-data recall does not include profile claims or invite inference;
- MCP recall schema remains stable if policy is added as metadata.

## Phase 3: Automatic Extraction Blocklist

### Prompt Contract

Extend `src/user_context/extraction/prompt.rs` quality gates with explicit
non-retention rules:

```text
Do not create candidates for temporary state, mood, fatigue, meals, weather, or
one-off circumstances.
Do not create candidates for world knowledge, project-independent facts, or
general technical facts.
Do not create candidates for third-party details unless the user explicitly
frames them as relevant to their own durable context; even then, keep them
pending for human review and never auto-promote them.
Do not create candidates from guesses, jokes, sarcasm, role-play, fiction, or
hypothetical identities.
Do not create candidates containing credentials, secrets, API keys, tokens,
passwords, account numbers, identity documents, or payment data.
Do not create candidates for illegal, harmful, or clearly false claims.
Do not create assistant-authored claims about the user unless directly supported
by cited user-authored events.
Do not create claims derived from files or external sources without explicit
user approval.
```

Keep the existing positive constraints:

- `source_event_ids` must cite loaded events;
- low risk is only for explicit first-party user preference or constraint
  statements with normal sensitivity;
- assistant-authored summaries, inferred behavior, sensitive categories, and
  speculative statements stay review-gated.

### Store and Review Behavior

The first implementation must enforce the highest-risk blocklist items
deterministically, not only through prompt wording. Prompt guidance is useful,
but it is not fail-closed once a syntactically valid model response reaches the
candidate store.

Required insertion/promotion gates:

- credentials/secrets must be rejected or redacted before candidate insert,
  including `claim_text` and `source_preview`;
- temporary state, world knowledge, general technical facts, role-play,
  fiction, jokes, sarcasm, hypothetical identities, illegal claims, harmful
  claims, clearly false claims, unsupported assistant-authored claims, and
  unapproved file/external-source claims must create no candidate. The only
  allowed retained output for these classes is a non-sensitive aggregate block
  reason that omits the blocked text;
- third-party details must never auto-promote. If the user explicitly frames a
  third-party detail as relevant to their own durable context, create only a
  pending-review candidate with a block reason such as
  `third_party_requires_review`; otherwise create no candidate.

Do not widen auto-promotion. Auto-promotion remains limited to normal,
low-risk, high-confidence, explicit user-authored preferences or constraints
that also pass the deterministic blocklist gates.

### Tests

Required tests:

- prompt JSON contains the full non-retention blocklist;
- malformed model output still creates no candidates;
- secret-like candidate text and previews are rejected or redacted before
  insertion;
- joke/role-play/hypothetical, illegal, harmful, and clearly false candidate
  text creates no candidate even when the model labels it low-risk;
- third-party candidate text stays pending review unless explicitly approved by
  review flow.

## Issue Split

Recommended GitHub issue split:

1. Profile Markdown snapshot.
2. Usage policy injection rules.
3. Automatic extraction blocklist.

Do not combine all three in one implementation PR. Each touches a different
surface and has different verification gates.

## Verification

Documentation-only spec PR:

```bash
git diff --check
```

Implementation PRs:

```bash
cargo fmt --check
cargo check
```

Run targeted tests for the touched module first. Run `cargo test` before
merging changes to extraction, context rendering, MCP behavior, REST behavior,
or schema.
