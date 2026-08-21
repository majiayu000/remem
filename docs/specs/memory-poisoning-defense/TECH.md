# Memory Poisoning Defense Technical Spec

Status: Current contract
Date: 2026-07-02

Tracking:
- Spec/tracking issue: #672
- Production-path eval issue: #991

## Existing Implementation Facts

- Capture provenance is already recorded: `captured_events` keeps host,
  session, and event identity (`src/migrations/v006_capture_pipeline.sql`);
  candidates keep evidence ids; `memory_facts` keeps `source_event_ids`.
- Secret redaction runs in the capture adapters
  (`src/adapter/redaction.rs`); it does not classify instructions.
- The auto-promote gate (`src/memory_candidate.rs`, `should_auto_promote`)
  checks scope, risk class, confidence, routing, evidence, memory type, unsafe
  markers, and observation support — but not content instruction patterns or
  source trust.
- Candidate review exists (`src/memory_candidate/review.rs`) with approve,
  discard, and edit paths.
- The renderer (`src/context/render.rs`) injects memory content verbatim;
  `memory_suppressions` (v051) can hide items but nothing populates it
  automatically for adversarial content.
- Direct memory saves (`src/memory/service/save.rs`) bypass candidate review
  and write active memories directly, so they must receive equivalent trust,
  scan, and acknowledgement metadata rather than relying only on candidate
  insertion.

## Design Rules

- Deterministic, versioned pattern matching; no LLM in scan or injection
  paths.
- Quarantine is a candidate review status, not deletion; evidence is
  preserved for audit.
- Trust class is derived from capture provenance, never self-declared by
  extraction output.
- Injection-time drops log at error level with enough detail to diagnose
  (memory id, pattern id, provenance) — no silent degradation.

## Proposed Design

### Trust classification

At candidate insertion, derive `source_trust_class` from the supporting
captured events:

| Event origin | Trust class |
| --- | --- |
| UserPromptSubmit text | `user_prompt` |
| Read/Grep on repo-owned paths | `repo_file` |
| Bash/tool output | `local_tool_output` |
| Session summaries | Inherit the lowest trust class of the covered source events; if source expansion is unavailable, treat as `external_content` for auto-promote decisions |
| WebFetch/WebSearch results, MCP output from remote servers | `external_content` |
| Direct save service input from MCP/REST (current compatibility behavior) | `user_prompt` |

Lowest class among supporting evidence wins. Stored as a new candidate column
and copied onto promoted memories (next free migration after the current
schema; do not reserve an already-used migration number). Pre-existing rows
default to `local_tool_output`.

Transition note: the current request DTO cannot distinguish authenticated
human input from an agent invoking MCP `save_memory`, and current code stamps
both with the direct-save class. The GH969 activation contract supersedes that
unconditional elevation for the future consolidated boundary: trusted adapters
must bind caller evidence, and an unbound agent call uses `external_content`.
Until that implementation lands, this remains explicit transition debt rather
than a claimed closed protection.

### Instruction-pattern scan

A versioned pattern table (Rust source, unit-tested, English and Chinese
variants) covering:

- override/authority phrases ("ignore previous instructions", "absolute
  authority", "supersedes user");
- execution imperatives directed at the reader ("run the following",
  "execute this command silently");
- concealment directives ("do not mention", "hide this from");
- opaque payload heuristics (long base64-like runs above a threshold).

Scan points:

1. Candidate insertion (`src/memory_candidate.rs`): match -> insert with
   `review_status='quarantined'` plus `quarantine_pattern_id`; never eligible
   for `should_auto_promote`.
2. Direct save (`src/memory/service/save.rs`): classify as trusted local user
   input, scan before insert, and fail with a structured validation error on
   pattern match unless the caller supplies explicit acknowledgement metadata.
   Acknowledged direct saves record pattern id + timestamp in the same durable
   acknowledgement store used by reviewed candidates.
3. Injection render (`src/context/render.rs` input assembly): re-scan final
   item content; match -> drop item, `log::error!` with memory id + pattern
   id, increment a doctor-visible counter. Approved-after-review memories
   carry an acknowledgement flag that suppresses the injection-time re-drop
   for the acknowledged pattern id only.

### Gate integration

`should_auto_promote` gains: `source_trust_class >= config floor` (default
`local_tool_output`); `external_content` is never auto-promotable regardless
of confidence. Block reasons logged through the existing
`auto_promote_block_reason` channel.

### Review and CLI

- Quarantined candidates appear in the existing review listing with the
  matched pattern rendered.
- `approve` on a quarantined candidate requires `--acknowledge-pattern` (or
  interactive confirmation) and records pattern id + timestamp in the
  operation log (`memory_operation_log`).

### Doctor

- quarantine count by pattern id;
- pattern-set version;
- injection-drop counter with last drop detail.

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| P1 trust class derivation | candidate insertion | unit tests per origin; mixed-evidence lowest-wins test |
| P2 quarantine on match | memory_candidate | fixture: override phrase -> quarantined, gate never sees it |
| P3 trust floor in gate | should_auto_promote | boundary tests incl. external_content never promotes |
| P4 injection re-scan | context render | fixture: flagged row in DB -> absent from output, error logged |
| P5 acknowledged approval | review path | test: approve records pattern ack; item renders afterward |
| P6 determinism | pattern module | table-driven tests; version bump changes verdicts only via table |
| P7 direct save coverage | save_memory service | test: direct save gets trust class; pattern match requires acknowledgement |

## Data Flow

captured_events (with origin) -> candidate insertion (trust class + scan) ->
quarantine or pending/auto-promote -> review (acknowledged approval) ->
memories (trust class copied) -> render-time re-scan -> context block. Drops
flow to log + doctor counters.

Direct save path: local caller input -> trust class `user_prompt` + pattern
scan -> either structured validation error or acknowledged active memory ->
render-time re-scan with acknowledgement check.

## Alternatives Considered

- LLM-based intent classification at write time: deferred; adds cost and
  non-determinism; the pattern layer is the enforceable baseline (mirrors the
  SEC-14-style first-pass model), and a semantic layer can compose later.
- Deleting matched candidates outright: rejected; destroys audit evidence and
  removes the false-positive escape hatch.
- Scanning only at injection time: rejected; a poisoned memory would still
  reach the store, MCP `search`, and export surfaces.

## Risks

- Security: pattern lists are bypassable by paraphrase; trust floor is the
  backstop (external content cannot auto-promote at all). Documented as
  defense-in-depth, not a guarantee.
- Compatibility: the next free migration adds columns with defaults; old rows
  readable unchanged.
- Performance: regex table scan per candidate and per rendered item; bounded
  and measured by existing latency benchmarks.
- Maintenance: pattern table growth is versioned and unit-tested; MCP tool
  output classification depends on adapter origin fidelity.

## Test Plan

- [ ] Unit tests: pattern table (positive/negative per pattern), trust
      derivation, gate floor boundaries.
- [ ] Integration: seeded poisoned captured_events fixture end-to-end
      (capture -> extraction -> quarantine -> render absence -> doctor).
- [ ] Manual verification: real session with a fetched web page containing an
      override phrase; confirm quarantine and doctor visibility.

## Production-path adversarial evaluation (GH-991)

The public `adversarial-policy` suite has two deliberately distinct kinds of
paths:

- `remem_default` is production-path evidence. Each task uses a fresh migrated
  SQLite database, records real `captured_events`, runs deterministic fixture
  responses through `observation_extract::process_with_extractor`, runs the
  resulting follow-up through `memory_candidate::process_with_generator`, and
  then performs normal candidate promotion and retrieval.
- `retrieved_memory` and the other fixture baselines remain comparative
  capability paths. If they insert a memory directly, their artifact says
  `direct_memory_fixture`; their results do not substantiate a production
  poisoning-defense claim.

The production-path evaluator never substitutes fixture expectations for
observed governance state. It measures:

1. active claims from current, unsuppressed `memories` rows;
2. reviewable candidates from persisted candidate review states; and
3. summary inputs from the actual active memory rows admitted by the same
   current-memory query.

The source-event verdict calls `scan_source_instruction_pattern` exactly as
production does (`opaque_payload` disabled for raw capture). Generated
observation and candidate surfaces still use the full scanner. Artifacts record
the source verdict and generated-surface block separately, plus a combined
defense verdict, so an opaque raw payload cannot be falsely described as a
source-scanner hit. Poisoned observations are excluded from candidate batches;
their durable quarantine row is evidence of the generated-surface block, not
an input to promotion.

Every run artifact places `verification_path`, `measurement_source`, and
scanner configuration next to its policy counts. The aggregate report records
the set of verification paths used. Tests assert the full adversarial
`remem_default` path, non-zero measured state for the explicitly approved case,
zero leakage for blocked cases, and the raw-vs-generated opaque-payload split.

## Rollback Plan

Config flags disable the injection re-scan and the trust floor independently;
quarantined rows can be bulk-moved back to `pending_review` with a one-line
SQL update documented in the migration notes. The additive migration columns
can remain in place when the feature is disabled.

## Capture/Extraction Path Extension (GH-855, staged 0.6.24)

The GH-672 contract covered candidates, direct saves, and memory/lesson
injection. GH-855 extends the same v1 pattern set (`src/memory/poisoning.rs`,
`INSTRUCTION_PATTERN_SET_VERSION`) to every capture/extraction surface:

- **Combined source + generated verdict for rollups**
  (`src/session_rollup/persist.rs`): before a session rollup is persisted, the
  captured source events (ascending event id) and every generated summary
  field (`summary_text`, `request`, `decisions`, `learned`, `next_steps`,
  `preferences`, segment titles/summaries) are scanned. A source hit cannot be
  laundered by a clean summary. A hit stores a durable `quarantined` summary
  row and withholds topic segments and all model-visible side effects; worker
  retries keep the block without re-calling the summarizer.
- **Durable summary state (migration v072)**: `session_summaries` gains
  `poisoning_status` (`legacy_unscanned|safe|quarantined|acknowledged`),
  quarantine stage/field/event/pattern metadata, acknowledgement columns, and
  block counters. Pre-migration rows are marked `legacy_unscanned` and are
  re-scanned by every reader until a verdict lands; new writers write an
  explicit verdict.
- **Fail-closed reader gate** (`src/db/summary_poisoning.rs`): all
  model-visible summary sinks — SessionStart recent sessions, MEMORY.md native
  sync, observation/summarize prompt context, user-context
  extraction/recall/activity, git/MCP commit trace, and the shared summary
  queries — exclude quarantined rows in SQL and re-scan the exact fields they
  expose immediately before use. An unacknowledged hit quarantines the row in
  place (loud error log, block counter); state-load errors drop the row.
  Acknowledgement requires an exact pattern id + version match.
- **Observation and legacy summarize writers**: `insert_observation*` scans
  generated fields and lands hits as `status='poisoning_quarantined'`
  (excluded from all active/stale queries); `finalize_summarize` writes the
  same explicit summary verdict.
- **Observability**: `remem doctor` (Memory poisoning defense check),
  `remem status`, and HTTP `/status` expose a `poisoning_defense` aggregate:
  pattern set version, candidate/summary/observation quarantine counts,
  legacy-unscanned summaries, summary block count, and injection drops.
  Metadata only — no payload or matched text is ever emitted.
- **Eval**: the public `adversarial-policy` suite is revised to v2 with
  `instruction_injection` (EN/ZH), `authority_claim`, `opaque_payload`, and
  `benign_quoted_instruction` categories. Policy scoring runs the production
  scanner over fixture evidence; quarantine wins over `retention_allowed`,
  and a declared injection fixture the scanner misses counts as a policy
  failure. Deterministic capture E2E lives in
  `src/session_rollup/tests/poisoning.rs` (real capture -> rollup pipeline
  with a fixture summarizer, no network).

Failure semantics: prefer false-positive quarantine over letting content reach
a model-visible sink; scan/state errors exclude the row rather than degrade to
"assume safe"; captured events and raw archives are never deleted by a match.

## Dream Generated-Output Extension (GH-969, staged 0.6.46)

Dream output never inherits trust from the memories in its prompt. Before a
`MergeDecision` branch writes anything, `scan_generated_surfaces` scans the
generated topic key, memory type, title, content, no-merge reason, and conflict
reason in a fixed order. Merge output is also scanned as the exact
`title + "\n" + content` render used by review and promotion, preventing a split
instruction from crossing a field boundary.

The source cluster has a domain-separated, length-prefixed SHA-256 signature.
It binds project, memory type, and each sorted member's id, database version,
update epoch, topic key, title, and content. The immutable decision digest binds
the decision kind and its complete payload: structured Merge fields plus exact
intended ids, the no-merge reason, or conflict ids plus reason. A separate
semantic discriminator binds the cluster signature, decision digest, generated
field, and instruction-pattern id/version.

Cluster selection and every write branch use the canonical current-memory,
state-key-current, TTL, and suppression predicates. Merge, no-merge, conflict,
and quarantine then revalidate the full snapshot under the same immediate
write transaction before their first mutation. A source payload/version change,
expiry, state-key pointer replacement, or active suppression therefore fails
closed even when the memory row itself did not change during the model call.

A match uses one immediate transaction to:

1. claim a route-scoped external identity and create or reuse a quarantined
   `dream_model_output` candidate;
2. insert an immutable `dream_quarantine_artifacts` payload, or advance only
   its version/occurrence/update epoch for an exact recurrence;
3. terminalize only older pending/quarantined candidates for the same project
   and exact cluster signature when the semantic discriminator changes, marking
   them `dream_semantic_superseded` and writing a payload-free audit event; and
4. record a durable Dream `no_merge` decision containing safe identifiers only.

Any failure rolls the candidate, identity, artifact, terminalization, audit, and
no-merge write back together. Source memories stay active; no replacement,
supersede operation, conflict edge, raw model reason, MCP result, or
SessionStart item is emitted. A later A→B→A semantic recurrence creates a fresh
review candidate rather than treating the system supersede as a human reject.
Every later clean `no_merge` or `conflict` decision also terminalizes all still
reviewable quarantine candidates for the exact cluster signature inside that
decision's immediate transaction. The terminalization, audit event, and final
decision/operation therefore commit or roll back together.

Candidate detail recomputes the source snapshot and full decision digest before
issuing a review token. The token binds candidate and artifact versions, every
immutable artifact field, and the exact authorized supersede ids. A source
version/content/status/type/project change, artifact recurrence, or semantic
replacement invalidates the old token. API output redacts generated topic,
type, title, and content in both the base candidate and provenance projection;
CLI review output escapes terminal controls and Unicode bidi controls.

Clean generated merges remain model output: the resulting active memory is
explicitly `source_trust_class='external_content'`. Dream may create a new row
or reuse only an exact reviewed cluster member. If generic state-key,
preference, or semantic dedup resolves to any pre-existing row outside that
set, the transaction fails and rolls back the tentative rewrite, supersedes,
operation, edges, and decision together.

Safe approval requires the current candidate version, pattern acknowledgement,
and current Dream token inside one immediate transaction. Promotion preserves
the structured title/content and must supersede exactly the reviewed current
source set—neither a subset nor an extra active target is accepted. Legacy,
edit, and batch paths cannot bypass this contract. No-merge and conflict
artifacts are reject-only and can never become active memories.

The v076 external identity ledger stores SHA-256 summaries, not candidate text.
Its length-prefixed identity covers source kind, memory type, optional semantic
discriminator, source project, owner scope/key, null-tagged target project,
topic key, and text. Candidate mutation and ledger/recurrence writes share a
savepoint, and every digest hit validates the stored route fields. Semantic
identities never adopt unverifiable legacy candidates; ordinary legacy native
imports retain deterministic exact-row adoption.

## Dream Stock Backfill (GH-990, staged 0.6.51)

The forward boundary leaves one gap: memories Dream merged before v076 carry
the v060 default trust class `local_tool_output` and stay active. The backfill
closes that stock half as an explicit operator command
(`remem dream-backfill`), never inside a migration — the quarantine ledger is
append-only and irreversible, so writing to it is a deliberate decision with a
dry-run first.

Stock identification is a pure projection of the pre-v076 write path:
`session_id='dream' AND status='active' AND source_trust_class=
'local_tool_output'`. Post-v076 merges are stamped `external_content` by
`mark_dream_generated`, so they never match, which also makes re-runs
idempotent.

Planning scans every stock row with the exact forward convention: the
individual generated fields in declared order, then the combined
`title + "\n" + content` surface (`scan_generated_surfaces`). The plan splits
rows into hits, no-hits, and skipped (rows whose empty generated title/content
cannot satisfy the merge artifact CHECK are reported, never quarantined).

Applying the complete plan opens one immediate transaction, re-plans the full
stock set, and compares its digest plus each row snapshot with the rehearsal
before it writes anything. Any plan drift aborts atomically. Each hit is then
re-loaded and re-scanned inside that transaction (a stale snapshot aborts
rather than writing a wrong binding), then:

1. creates or reuses a quarantined `dream_model_output` candidate through the
   external identity ledger (`risk_class='high'`, `confidence=0.5`);
2. inserts a `dream_quarantine_artifacts` row whose v077
   `backfill_memory_id` binds the exact retired memory — the column is
   merge-only at insert and immutable at update by trigger, and the restore
   path treats `decision_ids == intended_superseded_ids == [backfill_memory_id]`;
3. retires the memory (`status='archived'`) and stamps
   `source_trust_class='external_content'` in the same statement, so a later
   restore never re-enters the stock set; and
4. records a `dream_backfill_quarantine` operation log entry.

Approval goes through the normal review path with pattern acknowledgement and
the current Dream token, then branches on the backfill binding: instead of
promoting generated payload, it restores the bound memory in place after
verifying the row still exists, is still the Dream memory with
`source_trust_class='external_content'`, still belongs to the same project and
memory type, is still archived, and its effective topic key, title, and content
still equal the reviewed merge payload exactly — any drift fails closed with
`dream_backfill_restore_payload_mismatch`. The restore keeps the original
memory id and writes the same review metadata, ack write-back, and audit
event as a forward approval. Provenance loading treats backfill members as
archived-by-design and skips the cluster-signature staleness recompute for
backfill rows (retirement necessarily moved version/update epoch; the
restore-time payload comparison is the integrity check).

A no-hit row only has its trust class backfilled to `external_content`,
deliberately without touching `updated_at_epoch` so a maintenance pass cannot
make old memories look freshly written to recency-sensitive ranking. It is
re-checked as a no-hit inside the same transaction, and each backfilled row
records a `dream_backfill_trust_class` operation log entry.

The CLI defaults to dry-run and rejects `--apply --dry-run` together; the
report lists a plan digest, per-project stock/hit/skip counts, and the matched
field/pattern per hit, capped for readability, with full detail under `--json`.
`--apply --expect-plan-digest <sha256>` binds the write to a reviewed
rehearsal; apply without that optional flag still performs the in-transaction
plan binding. The v077 backfill binding is merge-only, immutable, and unique
per retired memory, preventing duplicate recurrence artifacts.
