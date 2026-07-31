# GH933 Migration and Cutover Contract
Refs #933.
## Status and Authority

This is the normative Phase A v2 migration, retry-ledger, hashing, and local-copy
cutover contract referenced by `TECH.md`. It remains pending until implementation,
`MIGRATION-REHEARSAL.md` evidence, and `ROLLOUT.md` gates pass. SQL is executable,
not pseudocode; production must preserve every constraint and trigger body.

The cutover is a breaking, maintenance-window migration. All 0.6.x writers are
stopped before the foreground transaction begins and remain stopped until the
new binary passes postflight. There is no mixed-writer mode and no down
migration after a v2 write.
## Implementation Scope

- `Cargo.toml`/`Cargo.lock`: enable rusqlite `functions`.
- `src/db/sql_functions.rs` and every connection constructor: register the
  versioned function after SQLCipher keying and before schema access or writes.
- The migration SQL/runner install this DDL, rebuild `memories`, and backfill in
  one `BEGIN IMMEDIATE`.
- Every insert and named route/lifecycle update creates intent before mutation,
  populates all declared bindings, and seals last.
- `src/memory/service/types.rs`, `save.rs`, `local_copy.rs`, and all API/MCP
  save adapters: require the caller key and implement the journal protocol.
- `src/doctor/`: reconcile safe journals and report every pending or ambiguous
  journal as a visible diagnostic.
- Run the migration/API/writer/DDL/UDF/retry/fault tests in the rehearsal.
No connection may register different framing; no fallback hash is legal.
## Versioned SHA-256 Data Flow

`remem_sha256_frame_v1` is variadic and takes alternating names and values:

```text
remem_sha256_frame_v1(name_0, value_0, name_1, value_1, ...)
```

It rejects zero or odd argument counts, non-TEXT, blank, non-ASCII, or duplicate
field names. For each pair, in call order, it feeds SHA-256:

```text
u32_be(name UTF-8 byte length)
name UTF-8 bytes
u8(type)                         # 0=NULL, 1=INTEGER, 2=REAL, 3=TEXT, 4=BLOB
u64_be(value byte length)
value bytes
```

INTEGER is signed i64 big-endian. REAL is the exact IEEE-754 f64 bit pattern in
big-endian order. TEXT is exact UTF-8; BLOB is exact bytes; NULL has length zero
and differs from empty TEXT/BLOB. The return is exactly 64 lowercase hex
characters. Registration is `DETERMINISTIC | INNOCUOUS`; failure aborts.

Rust hashes requests before SQL; SQL chains results, hashes request/terminal
result/schema/response, and INSERT triggers hash typed `NEW`.
Golden vectors cover NULL/empty, i64 bounds, negative zero/non-finite rejection,
multibyte/NUL TEXT, BLOB, order and duplicate names; independent Python matches.
## Caller Idempotency

Every direct save entrypoint requires `idempotency_key`. The adapter trims ASCII
outer whitespace once, then requires 1–128 bytes entirely in
`[A-Za-z0-9._~-]`. It derives:

```text
request_id = "save_" || lower_hex(
    SHA-256("remem/save-idempotency/v1\0" || normalized_key)
)
```

Only `request_id` is retained; raw/normalized keys never enter serialization,
database, journals, logs, errors, traces, metrics, or responses.

Fingerprint excludes key/credentials and covers every other raw field, Option
presence, list order/duplicates, reference time, defaults, and effective inputs:

| Existing row | Incoming key/payload | Result |
| --- | --- | --- |
| none | any valid key/payload | execute once |
| sealed, equal request fingerprint | same key/equal payload | return stored response without mutation |
| sealed, different request fingerprint | same key/different payload | `idempotency_conflict` before mutation |
| any | different key/byte-identical payload | execute as a distinct request |
| intent without seal | any retry after restart | impossible after DB rollback; journal reconciliation runs first |

Different keys preserve the second lesson reinforcement, operation, claim, and
knowledge transition.
## Final Schema Preconditions

The migration rebuilds `memories` from the canonical current schema rather than
using `ALTER TABLE ... NOT NULL` on populated data. The rebuilt table adds:

```sql
insert_writer_kind TEXT NOT NULL,
insert_request_id TEXT NOT NULL,
insert_result_ordinal INTEGER NOT NULL CHECK (insert_result_ordinal >= 0),
UNIQUE (insert_writer_kind, insert_request_id, insert_result_ordinal),
FOREIGN KEY (insert_writer_kind, insert_request_id)
  REFERENCES memory_write_requests(writer_kind, request_id)
  ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED
```

Postflight compares all rebuilt columns/defaults/checks/FKs/indexes/FTS/triggers
with a fresh same-binary database; any omission aborts.
## Executable Ledger DDL

The block runs before any v2 writer. Isolated tests create FK parents with their
production primary-key types.
```sql
PRAGMA foreign_keys = ON;
CREATE TABLE memory_write_requests (
    writer_kind TEXT NOT NULL
      CHECK (length(writer_kind) BETWEEN 1 AND 64)
      CHECK (writer_kind NOT GLOB '*[^0-9A-Za-z._:-]*'),
    request_id TEXT NOT NULL
      CHECK (length(request_id) BETWEEN 1 AND 128)
      CHECK (request_id NOT GLOB '*[^0-9A-Za-z._:-]*'),
    request_fingerprint TEXT NOT NULL
      CHECK (length(request_fingerprint) = 64)
      CHECK (request_fingerprint NOT GLOB '*[^0-9a-f]*'),
    request_schema_version INTEGER NOT NULL
      CHECK (request_schema_version > 0),
    requested_at_epoch INTEGER NOT NULL CHECK (requested_at_epoch >= 0),
    expected_results_json TEXT NOT NULL
      CHECK (json_valid(expected_results_json) = 1)
      CHECK (json_type(expected_results_json) = 'array'),
    PRIMARY KEY (writer_kind, request_id),
    FOREIGN KEY (writer_kind, request_id)
      REFERENCES memory_write_request_commits(writer_kind, request_id)
      ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED
);
CREATE TABLE memory_route_ledger (
    id INTEGER PRIMARY KEY,
    memory_id INTEGER NOT NULL,
    route_version INTEGER NOT NULL CHECK (route_version > 0),
    previous_route_id INTEGER,
    effective_at_epoch INTEGER NOT NULL CHECK (effective_at_epoch >= 0),
    source_kind TEXT NOT NULL CHECK (
      source_kind IN (
        'insert', 'legacy_backfill', 'save_upsert',
        'markdown_import', 'scope_cleanup'
      )
    ),
    audit_event_id INTEGER,
    source_writer_kind TEXT NOT NULL,
    source_ref TEXT NOT NULL,
    source_result_ordinal INTEGER NOT NULL CHECK (source_result_ordinal >= 0),
    source_fingerprint TEXT NOT NULL
      CHECK (length(source_fingerprint) = 64)
      CHECK (source_fingerprint NOT GLOB '*[^0-9a-f]*'),
    coverage_kind TEXT NOT NULL
      CHECK (coverage_kind IN ('complete', 'forward_only')),
    coverage_start_epoch INTEGER NOT NULL CHECK (coverage_start_epoch >= 0),
    placement_project TEXT NOT NULL,
    source_project TEXT,
    target_project TEXT,
    owner_scope TEXT,
    owner_key TEXT,
    memory_type TEXT NOT NULL,
    topic_key TEXT,
    topic_domain TEXT,
    routing_confidence REAL,
    routing_reason TEXT,
    context_class TEXT,
    memory_scope TEXT NOT NULL,
    branch TEXT,
    CHECK (
      (owner_scope IS NULL AND owner_key IS NULL)
      OR (owner_scope IS NOT NULL
        AND owner_scope IN (
          'user', 'workspace', 'repo', 'tool',
          'domain', 'workstream', 'session'
        )
        AND owner_key IS NOT NULL
        AND length(owner_key) > 0
        AND owner_key = trim(
          owner_key, char(9)||char(10)||char(11)||char(12)||char(13)||' ')
      )
    ),
    CHECK (memory_scope IN ('project', 'global')),
    UNIQUE (memory_id, route_version),
    UNIQUE (previous_route_id),
    UNIQUE (memory_id, source_kind, source_fingerprint),
    FOREIGN KEY (memory_id) REFERENCES memories(id) ON DELETE RESTRICT,
    FOREIGN KEY (previous_route_id)
      REFERENCES memory_route_ledger(id) ON DELETE RESTRICT,
    FOREIGN KEY (source_writer_kind, source_ref)
      REFERENCES memory_write_requests(writer_kind, request_id)
      ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED
);
CREATE INDEX idx_memory_route_time ON memory_route_ledger(memory_id,effective_at_epoch,id);
CREATE INDEX idx_memory_route_owner ON memory_route_ledger(owner_scope,owner_key,memory_scope,branch,effective_at_epoch,memory_id);
CREATE INDEX idx_memory_route_target ON memory_route_ledger(owner_scope,target_project,memory_scope,branch,effective_at_epoch,memory_id);
CREATE INDEX idx_memory_route_legacy ON memory_route_ledger(placement_project,memory_scope,branch,effective_at_epoch,memory_id) WHERE owner_scope IS NULL AND owner_key IS NULL;
CREATE INDEX idx_memory_route_coverage ON memory_route_ledger(coverage_kind,coverage_start_epoch);
CREATE TABLE memory_lifecycle_ledger (
    id INTEGER PRIMARY KEY,
    memory_id INTEGER NOT NULL,
    lifecycle_version INTEGER NOT NULL CHECK (lifecycle_version > 0),
    previous_lifecycle_id INTEGER,
    effective_at_epoch INTEGER NOT NULL CHECK (effective_at_epoch >= 0),
    previous_status TEXT CHECK (previous_status IS NULL OR previous_status IN ('active','stale','archived','deleted','rejected')),
    new_status TEXT NOT NULL CHECK (new_status IN ('active','stale','archived','deleted','rejected')),
    source_kind TEXT NOT NULL CHECK (
      source_kind IN (
        'insert', 'legacy_backfill', 'memory_governance',
        'web_governance', 'scope_cleanup'
      )
    ),
    source_action TEXT NOT NULL,
    source_operation_id INTEGER, source_api_operation_id TEXT, audit_event_id INTEGER,
    source_writer_kind TEXT NOT NULL,
    source_ref TEXT NOT NULL,
    source_result_ordinal INTEGER NOT NULL CHECK (source_result_ordinal >= 0),
    source_fingerprint TEXT NOT NULL
      CHECK (length(source_fingerprint) = 64)
      CHECK (source_fingerprint NOT GLOB '*[^0-9a-f]*'),
    coverage_kind TEXT NOT NULL
      CHECK (coverage_kind IN ('complete', 'forward_only')),
    coverage_start_epoch INTEGER NOT NULL CHECK (coverage_start_epoch >= 0),
    CHECK ((source_kind = 'web_governance' AND source_api_operation_id IS NOT NULL AND source_operation_id IS NULL) OR (source_kind <> 'web_governance' AND source_api_operation_id IS NULL)),
    UNIQUE (memory_id, lifecycle_version),
    UNIQUE (previous_lifecycle_id),
    UNIQUE (memory_id, source_kind, source_fingerprint),
    FOREIGN KEY (memory_id) REFERENCES memories(id) ON DELETE RESTRICT,
    FOREIGN KEY (previous_lifecycle_id)
      REFERENCES memory_lifecycle_ledger(id) ON DELETE RESTRICT,
    FOREIGN KEY (source_operation_id) REFERENCES memory_operation_log(id) ON DELETE RESTRICT,
    FOREIGN KEY (source_api_operation_id) REFERENCES api_mutation_requests(operation_id) ON DELETE RESTRICT,
    FOREIGN KEY (source_writer_kind, source_ref)
      REFERENCES memory_write_requests(writer_kind, request_id)
      ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED
);
CREATE INDEX idx_memory_lifecycle_time ON memory_lifecycle_ledger(memory_id,effective_at_epoch,id);
CREATE INDEX idx_memory_lifecycle_coverage ON memory_lifecycle_ledger(coverage_kind,coverage_start_epoch,memory_id);
CREATE UNIQUE INDEX uq_memory_lifecycle_operation ON memory_lifecycle_ledger(source_operation_id,memory_id) WHERE source_operation_id IS NOT NULL;
CREATE UNIQUE INDEX uq_memory_lifecycle_api_operation ON memory_lifecycle_ledger(source_api_operation_id,memory_id) WHERE source_api_operation_id IS NOT NULL;
CREATE TABLE memory_write_request_results (
    writer_kind TEXT NOT NULL,
    request_id TEXT NOT NULL,
    result_ordinal INTEGER NOT NULL CHECK (result_ordinal >= 0),
    binding_kind TEXT NOT NULL CHECK (
      binding_kind IN (
        'insert_origin', 'route_transition', 'lifecycle_transition',
        'memory_outcome', 'operation_outcome', 'claim_outcome',
        'poisoning_ack', 'local_copy_outcome', 'audit_outcome',
        'response_aux'
      )
    ),
    outcome_code TEXT NOT NULL CHECK (length(outcome_code) > 0),
    memory_id INTEGER,
    route_ledger_id INTEGER,
    lifecycle_ledger_id INTEGER,
    operation_id INTEGER,
    claim_id INTEGER,
    audit_event_id INTEGER,
    local_copy_path TEXT,
    local_copy_digest TEXT
      CHECK (
        local_copy_digest IS NULL
        OR (
          length(local_copy_digest) = 64
          AND local_copy_digest NOT GLOB '*[^0-9a-f]*'
        )
      ),
    binding_json TEXT NOT NULL
      CHECK (json_valid(binding_json) = 1)
      CHECK (json_type(binding_json) = 'object')
      CHECK (binding_json = json(binding_json)),
    previous_binding_fingerprint TEXT
      CHECK (
        previous_binding_fingerprint IS NULL
        OR (
          length(previous_binding_fingerprint) = 64
          AND previous_binding_fingerprint NOT GLOB '*[^0-9a-f]*'
        )
      ),
    binding_fingerprint TEXT NOT NULL
      CHECK (length(binding_fingerprint) = 64)
      CHECK (binding_fingerprint NOT GLOB '*[^0-9a-f]*'),
    PRIMARY KEY (writer_kind, request_id, result_ordinal, binding_kind),
    FOREIGN KEY (writer_kind, request_id)
      REFERENCES memory_write_requests(writer_kind, request_id)
      ON DELETE RESTRICT,
    FOREIGN KEY (memory_id) REFERENCES memories(id) ON DELETE RESTRICT,
    FOREIGN KEY (route_ledger_id)
      REFERENCES memory_route_ledger(id) ON DELETE RESTRICT,
    FOREIGN KEY (lifecycle_ledger_id)
      REFERENCES memory_lifecycle_ledger(id) ON DELETE RESTRICT,
    FOREIGN KEY (operation_id)
      REFERENCES memory_operation_log(id) ON DELETE RESTRICT,
    FOREIGN KEY (claim_id) REFERENCES memory_claims(id) ON DELETE RESTRICT
);
CREATE UNIQUE INDEX uq_memory_write_result_route
  ON memory_write_request_results(route_ledger_id)
  WHERE route_ledger_id IS NOT NULL;
CREATE UNIQUE INDEX uq_memory_write_result_lifecycle
  ON memory_write_request_results(lifecycle_ledger_id)
  WHERE lifecycle_ledger_id IS NOT NULL;
CREATE TABLE memory_write_request_commits (
    writer_kind TEXT NOT NULL,
    request_id TEXT NOT NULL,
    result_fingerprint TEXT NOT NULL
      CHECK (length(result_fingerprint) = 64)
      CHECK (result_fingerprint NOT GLOB '*[^0-9a-f]*'),
    response_schema_version INTEGER NOT NULL
      CHECK (response_schema_version > 0),
    response_json TEXT NOT NULL
      CHECK (json_valid(response_json) = 1)
      CHECK (response_json = json(response_json)),
    committed_at_epoch INTEGER NOT NULL CHECK (committed_at_epoch >= 0),
    PRIMARY KEY (writer_kind, request_id),
    FOREIGN KEY (writer_kind, request_id)
      REFERENCES memory_write_requests(writer_kind, request_id)
      ON DELETE RESTRICT
);
CREATE TRIGGER memory_write_request_manifest_guard
BEFORE INSERT ON memory_write_requests
BEGIN
  SELECT CASE WHEN NEW.expected_results_json<>json(NEW.expected_results_json)
    OR json_array_length(NEW.expected_results_json)=0
    THEN RAISE(ABORT, 'invalid request result manifest') END;
  SELECT CASE WHEN EXISTS (
    SELECT 1 FROM json_each(NEW.expected_results_json) AS item
    WHERE item.type<>'object' OR json_type(item.value,'$.result_ordinal')<>'integer'
       OR json_extract(item.value,'$.result_ordinal')<0
       OR json_type(item.value,'$.binding_kind')<>'text'
       OR json_extract(item.value,'$.binding_kind') NOT IN (
         'insert_origin','route_transition','lifecycle_transition','memory_outcome',
         'operation_outcome','claim_outcome','poisoning_ack','local_copy_outcome',
         'audit_outcome','response_aux'
       )
       OR (SELECT count(*) FROM json_each(item.value))<>2
       OR json(item.value)<>json_object(
         'result_ordinal',json_extract(item.value,'$.result_ordinal'),
         'binding_kind',json_extract(item.value,'$.binding_kind'))
  ) THEN RAISE(ABORT, 'invalid request result manifest entry') END;
  SELECT CASE WHEN EXISTS (
    SELECT 1 FROM json_each(NEW.expected_results_json) AS earlier
    JOIN json_each(NEW.expected_results_json) AS later
      ON CAST(earlier.key AS INTEGER)<CAST(later.key AS INTEGER)
    WHERE json_extract(earlier.value,'$.result_ordinal')>
            json_extract(later.value,'$.result_ordinal')
       OR (json_extract(earlier.value,'$.result_ordinal')=
             json_extract(later.value,'$.result_ordinal')
         AND json_extract(earlier.value,'$.binding_kind')>=
             json_extract(later.value,'$.binding_kind'))
  ) THEN RAISE(ABORT, 'request result manifest is not strictly sorted') END;
  SELECT CASE WHEN (
    SELECT count(*) FROM json_each(NEW.expected_results_json)
    WHERE json_extract(value,'$.binding_kind')='response_aux'
  )<>1 THEN RAISE(ABORT, 'request manifest needs one response_aux') END;
END;
CREATE TRIGGER memory_route_ledger_insert_guard
BEFORE INSERT ON memory_route_ledger
BEGIN
  SELECT CASE WHEN EXISTS (
    SELECT 1 FROM memory_write_request_commits
    WHERE writer_kind=NEW.source_writer_kind AND request_id=NEW.source_ref
  ) THEN RAISE(ABORT, 'sealed request cannot append route ledger') END;
  SELECT CASE WHEN NOT EXISTS (
    SELECT 1 FROM memory_write_requests AS request,
      json_each(request.expected_results_json) AS expected
    WHERE request.writer_kind=NEW.source_writer_kind
      AND request.request_id=NEW.source_ref
      AND json_extract(expected.value,'$.result_ordinal')=NEW.source_result_ordinal
      AND json_extract(expected.value,'$.binding_kind') IN ('insert_origin','route_transition')
  ) THEN RAISE(ABORT, 'route ledger lacks typed manifest slot') END;
END;
CREATE TRIGGER memory_lifecycle_ledger_insert_guard
BEFORE INSERT ON memory_lifecycle_ledger
BEGIN
  SELECT CASE WHEN EXISTS (
    SELECT 1 FROM memory_write_request_commits
    WHERE writer_kind=NEW.source_writer_kind AND request_id=NEW.source_ref
  ) THEN RAISE(ABORT, 'sealed request cannot append lifecycle ledger') END;
  SELECT CASE WHEN NOT EXISTS (
    SELECT 1 FROM memory_write_requests AS request,
      json_each(request.expected_results_json) AS expected
    WHERE request.writer_kind=NEW.source_writer_kind
      AND request.request_id=NEW.source_ref
      AND json_extract(expected.value,'$.result_ordinal')=NEW.source_result_ordinal
      AND json_extract(expected.value,'$.binding_kind') IN ('insert_origin','lifecycle_transition')
  ) THEN RAISE(ABORT, 'lifecycle ledger lacks typed manifest slot') END;
END;
CREATE TRIGGER memory_write_result_guard
BEFORE INSERT ON memory_write_request_results
BEGIN
  SELECT CASE WHEN EXISTS (
    SELECT 1 FROM memory_write_request_commits
    WHERE writer_kind = NEW.writer_kind AND request_id = NEW.request_id
  ) THEN RAISE(ABORT, 'request is already sealed') END;
  SELECT CASE WHEN NOT EXISTS (
    SELECT 1 FROM memory_write_requests AS request,
      json_each(request.expected_results_json) AS expected
    WHERE request.writer_kind=NEW.writer_kind AND request.request_id=NEW.request_id
      AND json_extract(expected.value,'$.result_ordinal')=NEW.result_ordinal
      AND json_extract(expected.value,'$.binding_kind')=NEW.binding_kind
  ) THEN RAISE(ABORT, 'result is absent from manifest') END;
  SELECT CASE WHEN EXISTS (
    SELECT 1 FROM memory_write_requests AS request,
      json_each(request.expected_results_json) AS expected
    WHERE request.writer_kind=NEW.writer_kind AND request.request_id=NEW.request_id
      AND (
        json_extract(expected.value,'$.result_ordinal')<NEW.result_ordinal
        OR (json_extract(expected.value,'$.result_ordinal')=NEW.result_ordinal
          AND json_extract(expected.value,'$.binding_kind')<NEW.binding_kind)
      )
      AND NOT EXISTS (
        SELECT 1 FROM memory_write_request_results AS actual
        WHERE actual.writer_kind=NEW.writer_kind AND actual.request_id=NEW.request_id
          AND actual.result_ordinal=json_extract(expected.value,'$.result_ordinal')
          AND actual.binding_kind=json_extract(expected.value,'$.binding_kind')
      )
  ) THEN RAISE(ABORT, 'result bindings must follow manifest order') END;
  SELECT CASE WHEN NEW.previous_binding_fingerprint IS NOT (
    SELECT actual.binding_fingerprint FROM memory_write_request_results AS actual
    WHERE actual.writer_kind=NEW.writer_kind AND actual.request_id=NEW.request_id
      AND (
        actual.result_ordinal<NEW.result_ordinal
        OR (actual.result_ordinal=NEW.result_ordinal
          AND actual.binding_kind<NEW.binding_kind)
      )
    ORDER BY actual.result_ordinal DESC, actual.binding_kind DESC
    LIMIT 1
  ) THEN RAISE(ABORT, 'result fingerprint predecessor mismatch') END;
  SELECT CASE WHEN NOT (
    (
      NEW.binding_kind='insert_origin' AND NEW.outcome_code IN ('inserted','backfilled')
      AND NEW.memory_id IS NOT NULL AND NEW.route_ledger_id IS NOT NULL
      AND NEW.lifecycle_ledger_id IS NOT NULL AND NEW.operation_id IS NULL
      AND NEW.claim_id IS NULL AND NEW.audit_event_id IS NULL
      AND NEW.local_copy_path IS NULL AND NEW.local_copy_digest IS NULL
    ) OR (
      NEW.binding_kind='route_transition' AND NEW.outcome_code='changed'
      AND NEW.memory_id IS NOT NULL AND NEW.route_ledger_id IS NOT NULL
      AND NEW.lifecycle_ledger_id IS NULL AND NEW.operation_id IS NULL
      AND NEW.claim_id IS NULL AND NEW.local_copy_path IS NULL
      AND NEW.local_copy_digest IS NULL
    ) OR (
      NEW.binding_kind='lifecycle_transition'
      AND NEW.outcome_code IN ('changed','acknowledged')
      AND NEW.memory_id IS NOT NULL AND NEW.lifecycle_ledger_id IS NOT NULL
      AND NEW.route_ledger_id IS NULL AND NEW.claim_id IS NULL
      AND NEW.local_copy_path IS NULL AND NEW.local_copy_digest IS NULL
    ) OR (
      NEW.binding_kind='memory_outcome'
      AND NEW.outcome_code IN ('inserted','updated','reinforced','noop')
      AND NEW.memory_id IS NOT NULL AND NEW.route_ledger_id IS NULL
      AND NEW.lifecycle_ledger_id IS NULL AND NEW.operation_id IS NULL
      AND NEW.claim_id IS NULL AND NEW.audit_event_id IS NULL
      AND NEW.local_copy_path IS NULL AND NEW.local_copy_digest IS NULL
    ) OR (
      NEW.binding_kind='operation_outcome' AND NEW.outcome_code='recorded'
      AND NEW.operation_id IS NOT NULL AND NEW.memory_id IS NULL
      AND NEW.route_ledger_id IS NULL AND NEW.lifecycle_ledger_id IS NULL
      AND NEW.claim_id IS NULL AND NEW.audit_event_id IS NULL
      AND NEW.local_copy_path IS NULL AND NEW.local_copy_digest IS NULL
    ) OR (
      NEW.binding_kind='claim_outcome'
      AND NEW.outcome_code IN ('created','reused','disabled','failed')
      AND (
        (NEW.outcome_code IN ('created','reused') AND NEW.claim_id IS NOT NULL)
        OR (NEW.outcome_code IN ('disabled','failed') AND NEW.claim_id IS NULL)
      )
      AND NEW.memory_id IS NULL AND NEW.route_ledger_id IS NULL
      AND NEW.lifecycle_ledger_id IS NULL AND NEW.operation_id IS NULL
      AND NEW.audit_event_id IS NULL AND NEW.local_copy_path IS NULL
      AND NEW.local_copy_digest IS NULL
    ) OR (
      NEW.binding_kind='poisoning_ack'
      AND NEW.outcome_code IN ('acknowledged','not_required','failed')
      AND (
        (NEW.outcome_code='acknowledged' AND NEW.memory_id IS NOT NULL)
        OR (NEW.outcome_code<>'acknowledged' AND NEW.memory_id IS NULL)
      )
      AND NEW.route_ledger_id IS NULL AND NEW.lifecycle_ledger_id IS NULL
      AND NEW.operation_id IS NULL AND NEW.claim_id IS NULL
      AND NEW.local_copy_path IS NULL AND NEW.local_copy_digest IS NULL
    ) OR (
      NEW.binding_kind='local_copy_outcome'
      AND NEW.outcome_code IN ('written','disabled','failed')
      AND (
        (NEW.outcome_code='written' AND NEW.local_copy_path IS NOT NULL
          AND NEW.local_copy_digest IS NOT NULL
        )
        OR (NEW.outcome_code IN ('disabled','failed') AND NEW.local_copy_path IS NULL
          AND NEW.local_copy_digest IS NULL
        )
      )
      AND NEW.memory_id IS NULL AND NEW.route_ledger_id IS NULL
      AND NEW.lifecycle_ledger_id IS NULL AND NEW.operation_id IS NULL
      AND NEW.claim_id IS NULL AND NEW.audit_event_id IS NULL
    ) OR (
      NEW.binding_kind='audit_outcome'
      AND NEW.outcome_code IN ('recorded','not_required','failed')
      AND (
        (NEW.outcome_code='recorded' AND NEW.audit_event_id IS NOT NULL)
        OR (NEW.outcome_code<>'recorded' AND NEW.audit_event_id IS NULL)
      )
      AND NEW.memory_id IS NULL AND NEW.route_ledger_id IS NULL
      AND NEW.lifecycle_ledger_id IS NULL AND NEW.operation_id IS NULL
      AND NEW.claim_id IS NULL AND NEW.local_copy_path IS NULL
      AND NEW.local_copy_digest IS NULL
    ) OR (
      NEW.binding_kind='response_aux' AND NEW.outcome_code='returned'
      AND NEW.memory_id IS NULL AND NEW.route_ledger_id IS NULL
      AND NEW.lifecycle_ledger_id IS NULL AND NEW.operation_id IS NULL
      AND NEW.claim_id IS NULL AND NEW.audit_event_id IS NULL
      AND NEW.local_copy_path IS NULL AND NEW.local_copy_digest IS NULL
    )
  ) THEN RAISE(ABORT, 'result binding shape mismatch') END;
  SELECT CASE WHEN NEW.binding_fingerprint <> remem_sha256_frame_v1(
    'domain', 'memory_write_result/v1',
    'writer_kind', NEW.writer_kind,
    'request_id', NEW.request_id,
    'request_fingerprint', (
      SELECT request_fingerprint FROM memory_write_requests
      WHERE writer_kind = NEW.writer_kind AND request_id = NEW.request_id
    ),
    'result_ordinal', NEW.result_ordinal,
    'binding_kind', NEW.binding_kind,
    'outcome_code', NEW.outcome_code,
    'memory_id', NEW.memory_id,
    'route_ledger_id', NEW.route_ledger_id,
    'lifecycle_ledger_id', NEW.lifecycle_ledger_id,
    'operation_id', NEW.operation_id,
    'claim_id', NEW.claim_id,
    'audit_event_id', NEW.audit_event_id,
    'local_copy_path', NEW.local_copy_path,
    'local_copy_digest', NEW.local_copy_digest,
    'binding_json', NEW.binding_json,
    'previous_binding_fingerprint', NEW.previous_binding_fingerprint
  ) THEN RAISE(ABORT, 'result binding fingerprint mismatch') END;
END;
CREATE TRIGGER memory_write_commit_guard
BEFORE INSERT ON memory_write_request_commits
BEGIN
  SELECT CASE WHEN EXISTS (
    SELECT 1 FROM memory_write_requests AS request,
      json_each(request.expected_results_json) AS expected
    WHERE request.writer_kind=NEW.writer_kind AND request.request_id=NEW.request_id
      AND NOT EXISTS (
        SELECT 1 FROM memory_write_request_results AS actual
        WHERE actual.writer_kind=NEW.writer_kind AND actual.request_id=NEW.request_id
          AND actual.result_ordinal=json_extract(expected.value,'$.result_ordinal')
          AND actual.binding_kind=json_extract(expected.value,'$.binding_kind')
      )
  ) THEN RAISE(ABORT, 'request results are incomplete') END;
  SELECT CASE WHEN EXISTS (
    SELECT 1 FROM memory_write_request_results AS actual
    WHERE actual.writer_kind=NEW.writer_kind AND actual.request_id=NEW.request_id
      AND NOT EXISTS (
        SELECT 1 FROM memory_write_requests AS request,
          json_each(request.expected_results_json) AS expected
        WHERE request.writer_kind=NEW.writer_kind AND request.request_id=NEW.request_id
          AND json_extract(expected.value,'$.result_ordinal')=actual.result_ordinal
          AND json_extract(expected.value,'$.binding_kind')=actual.binding_kind
      )
  ) THEN RAISE(ABORT, 'request has unexpected results') END;
  SELECT CASE WHEN EXISTS (
    SELECT 1 FROM memories AS memory
    WHERE memory.insert_writer_kind = NEW.writer_kind
      AND memory.insert_request_id = NEW.request_id
      AND NOT EXISTS (
        SELECT 1 FROM memory_write_request_results AS result
        JOIN memory_route_ledger AS route ON route.id=result.route_ledger_id
        JOIN memory_lifecycle_ledger AS lifecycle ON lifecycle.id=result.lifecycle_ledger_id
        WHERE result.writer_kind=NEW.writer_kind AND result.request_id=NEW.request_id
          AND result.result_ordinal=memory.insert_result_ordinal
          AND result.binding_kind='insert_origin' AND result.memory_id=memory.id
          AND route.memory_id=memory.id AND route.route_version=1
          AND route.previous_route_id IS NULL
          AND route.source_kind IN ('insert','legacy_backfill')
          AND route.source_writer_kind=NEW.writer_kind AND route.source_ref=NEW.request_id
          AND route.source_result_ordinal=memory.insert_result_ordinal
          AND lifecycle.memory_id=memory.id AND lifecycle.lifecycle_version=1
          AND lifecycle.previous_lifecycle_id IS NULL
          AND lifecycle.previous_status IS NULL AND lifecycle.source_action='baseline'
          AND lifecycle.source_kind IN ('insert','legacy_backfill')
          AND lifecycle.source_writer_kind=NEW.writer_kind
          AND lifecycle.source_ref=NEW.request_id
          AND lifecycle.source_result_ordinal=memory.insert_result_ordinal
      )
  ) THEN RAISE(ABORT, 'insert origin lacks matching v1 ledgers') END;
  SELECT CASE WHEN EXISTS (
    SELECT 1 FROM memory_write_request_results AS result
    WHERE result.writer_kind = NEW.writer_kind
      AND result.request_id = NEW.request_id
      AND result.binding_kind = 'insert_origin'
      AND NOT EXISTS (
        SELECT 1 FROM memories AS memory
        WHERE memory.id = result.memory_id
          AND memory.insert_writer_kind = NEW.writer_kind
          AND memory.insert_request_id = NEW.request_id
          AND memory.insert_result_ordinal = result.result_ordinal
      )
  ) THEN RAISE(ABORT, 'insert result lacks matching memory origin') END;
  SELECT CASE WHEN EXISTS (
    SELECT 1 FROM memory_write_request_results AS result
    JOIN memory_route_ledger AS route ON route.id = result.route_ledger_id
    WHERE result.writer_kind=NEW.writer_kind AND result.request_id=NEW.request_id
      AND result.binding_kind='route_transition'
      AND (
        route.memory_id IS NOT result.memory_id OR route.source_writer_kind<>NEW.writer_kind
        OR route.source_ref<>NEW.request_id
        OR route.source_result_ordinal<>result.result_ordinal
      )
  ) THEN RAISE(ABORT, 'route result binding mismatch') END;
  SELECT CASE WHEN EXISTS (
    SELECT 1 FROM memory_write_request_results AS result
    JOIN memory_lifecycle_ledger AS lifecycle ON lifecycle.id=result.lifecycle_ledger_id
    WHERE result.writer_kind=NEW.writer_kind AND result.request_id=NEW.request_id
      AND result.binding_kind='lifecycle_transition'
      AND (
        lifecycle.memory_id IS NOT result.memory_id
        OR lifecycle.source_writer_kind<>NEW.writer_kind
        OR lifecycle.source_ref<>NEW.request_id
        OR lifecycle.source_result_ordinal<>result.result_ordinal
      )
  ) THEN RAISE(ABORT, 'lifecycle result binding mismatch') END;
  SELECT CASE WHEN EXISTS (
    SELECT 1 FROM memory_route_ledger AS route
    WHERE route.source_writer_kind=NEW.writer_kind AND route.source_ref=NEW.request_id
      AND NOT EXISTS (
        SELECT 1 FROM memory_write_request_results AS result
        JOIN memory_write_requests AS request
          ON request.writer_kind=result.writer_kind AND request.request_id=result.request_id
        JOIN json_each(request.expected_results_json) AS expected
          ON json_extract(expected.value,'$.result_ordinal')=result.result_ordinal
         AND json_extract(expected.value,'$.binding_kind')=result.binding_kind
        WHERE result.writer_kind=NEW.writer_kind AND result.request_id=NEW.request_id
          AND result.result_ordinal=route.source_result_ordinal
          AND result.binding_kind IN ('insert_origin','route_transition')
          AND result.memory_id=route.memory_id AND result.route_ledger_id=route.id
      )
  ) THEN RAISE(ABORT, 'route ledger lacks typed result binding') END;
  SELECT CASE WHEN EXISTS (
    SELECT 1 FROM memory_lifecycle_ledger AS lifecycle
    WHERE lifecycle.source_writer_kind=NEW.writer_kind AND lifecycle.source_ref=NEW.request_id
      AND NOT EXISTS (
        SELECT 1 FROM memory_write_request_results AS result
        JOIN memory_write_requests AS request
          ON request.writer_kind=result.writer_kind AND request.request_id=result.request_id
        JOIN json_each(request.expected_results_json) AS expected
          ON json_extract(expected.value,'$.result_ordinal')=result.result_ordinal
         AND json_extract(expected.value,'$.binding_kind')=result.binding_kind
        WHERE result.writer_kind=NEW.writer_kind AND result.request_id=NEW.request_id
          AND result.result_ordinal=lifecycle.source_result_ordinal
          AND result.binding_kind IN ('insert_origin','lifecycle_transition')
          AND result.memory_id=lifecycle.memory_id
          AND result.lifecycle_ledger_id=lifecycle.id
      )
  ) THEN RAISE(ABORT, 'lifecycle ledger lacks typed result binding') END;
  SELECT CASE WHEN NEW.result_fingerprint <> remem_sha256_frame_v1(
    'domain', 'memory_write_commit/v1',
    'writer_kind', NEW.writer_kind,
    'request_id', NEW.request_id,
    'request_fingerprint', (
      SELECT request_fingerprint FROM memory_write_requests
      WHERE writer_kind = NEW.writer_kind AND request_id = NEW.request_id
    ),
    'terminal_binding_fingerprint', (
      SELECT binding_fingerprint
      FROM memory_write_request_results
      WHERE writer_kind = NEW.writer_kind AND request_id = NEW.request_id
      ORDER BY result_ordinal DESC, binding_kind DESC
      LIMIT 1
    ),
    'response_schema_version', NEW.response_schema_version,
    'response_json', NEW.response_json
  ) THEN RAISE(ABORT, 'request commit fingerprint mismatch') END;
END;
CREATE TRIGGER memory_origin_tuple_immutable BEFORE UPDATE OF
insert_writer_kind, insert_request_id, insert_result_ordinal ON memories
WHEN NEW.insert_writer_kind IS NOT OLD.insert_writer_kind
  OR NEW.insert_request_id IS NOT OLD.insert_request_id
  OR NEW.insert_result_ordinal IS NOT OLD.insert_result_ordinal
BEGIN SELECT RAISE(ABORT, 'memory insert origin is immutable'); END;
CREATE TRIGGER memory_write_requests_no_update BEFORE UPDATE ON memory_write_requests BEGIN SELECT RAISE(ABORT, 'memory write requests are append-only'); END;
CREATE TRIGGER memory_write_requests_no_delete BEFORE DELETE ON memory_write_requests BEGIN SELECT RAISE(ABORT, 'memory write requests are append-only'); END;
CREATE TRIGGER memory_write_results_no_update BEFORE UPDATE ON memory_write_request_results BEGIN SELECT RAISE(ABORT, 'memory write results are append-only'); END;
CREATE TRIGGER memory_write_results_no_delete BEFORE DELETE ON memory_write_request_results BEGIN SELECT RAISE(ABORT, 'memory write results are append-only'); END;
CREATE TRIGGER memory_write_commits_no_update BEFORE UPDATE ON memory_write_request_commits BEGIN SELECT RAISE(ABORT, 'memory write commits are append-only'); END;
CREATE TRIGGER memory_write_commits_no_delete BEFORE DELETE ON memory_write_request_commits BEGIN SELECT RAISE(ABORT, 'memory write commits are append-only'); END;
CREATE TRIGGER memory_route_ledger_no_update BEFORE UPDATE ON memory_route_ledger BEGIN SELECT RAISE(ABORT, 'memory route ledger is append-only'); END;
CREATE TRIGGER memory_route_ledger_no_delete BEFORE DELETE ON memory_route_ledger BEGIN SELECT RAISE(ABORT, 'memory route ledger is append-only'); END;
CREATE TRIGGER memory_lifecycle_ledger_no_update BEFORE UPDATE ON memory_lifecycle_ledger BEGIN SELECT RAISE(ABORT, 'memory lifecycle ledger is append-only'); END;
CREATE TRIGGER memory_lifecycle_ledger_no_delete BEFORE DELETE ON memory_lifecycle_ledger BEGIN SELECT RAISE(ABORT, 'memory lifecycle ledger is append-only'); END;
```

The migration also has literal route/lifecycle fingerprint guards over every
typed column except ID/digest and an `AFTER INSERT ON memories` guard requiring
`insert_origin`, hashing exact `NEW`, and creating v1; missing UDF aborts.
Templates, SQL concatenation, post-insert patches, and fallback hashes are
forbidden. Rehearsal extracts trigger SQL, runs all insert families, and mutates
every covered `NEW` field.

## Backfill and Foreground Cutover

The migration runner performs these steps under one exclusive maintenance
window and one `BEGIN IMMEDIATE` transaction:

1. Register and self-test `remem_sha256_frame_v1`; reject a missing function,
   wrong golden vector, disabled FK enforcement, or nonempty migration journal.
2. Verify schema, checkpoint WAL after all writers stop, close every handle, copy
   the main database byte-for-byte, fsync file/directory, hash, and test-open it.
3. Create the mutually referenced request/commit tables and migration intents;
   capture one migration epoch.
4. Rebuild `memories` with its exact current objects and origin tuple, then
   create ledgers/results. For each legacy ID, use
   `migration_vNNN:<memory_id>` with sorted `insert_origin`/`response_aux`
   manifest. Exhaustive durable evidence may form complete history; otherwise
   create only forward-only baselines. Never infer from current bytes/events.
5. Append typed bindings and canonical response, seal every deterministic
   migration request, install literal guards/insert triggers, recreate FTS and
   indexes, run postflight, and commit.

Postflight requires zero unsealed requests, exact manifest/results, one valid
terminal route/lifecycle per memory matching `memories`, valid origin/v1 maps,
no schema drift, `integrity_check='ok'`, and empty `foreign_key_check`.

If any step fails, SQLite rolls the transaction back and the old binary remains
stopped. The operator restores the fsynced backup only after proving the failed
database is closed. Once a v2 writer seals any non-migration request, rollback
means disabling the v2 projection while retaining schema/history; running 0.6.x
or restoring the old backup would lose writes and is forbidden.

## Durable Local-Copy Journal
The verified nonsymlink journal root `${REMEM_DATA_DIR}/write-journal/save/` is
app-owned mode 0700. For opaque request `R`, names are exactly `J=R.json`,
journal temp `T=.R.json.tmp`, and target siblings `S=.remem-save-R.stage`,
`B=.remem-save-R.backup`. Canonical JSON records writer/request fingerprints,
phase/recovery goal, paths, `before_kind`, D1/D0 (D0 null only in
`inspect_intent`), epoch, present-target identity `I0=(dev,ino)`, and metadata
`M0=(uid,gid,mode,nlink,size,mtime_ns)`; never content, key, token, or response.
Proof classes are deliberately different:

| Path | Required source and proof |
| --- | --- |
| `T` | remem creates with `O_CREAT\|O_EXCL\|O_NOFOLLOW`, mode 0600; exact name/private parent/current uid/regular/nlink=1 |
| `S` | same creation proof as `T`, plus exact after digest `D1` |
| `B` | not created and has no temp-mode invariant: durable `swap_intent`, then atomic no-replace target→`B`; it retains `I0`, `M0`, and `D0` |
| target | verified regular identity/metadata/digest or recorded absence; symlink, alias, and nonregular types are forbidden |
`B` therefore legitimately retains target mode 0644, 0200, or another recorded
mode; 0600 is not a backup invariant. No-replace must be one atomic platform
primitive (`renameat2(RENAME_NOREPLACE)`/`renameatx_np(RENAME_EXCL)` equivalent);
an existing `B` or unsupported filesystem errors before target mutation.

An unreadable owner-writable target (including 0200) is supported without a
readability precondition. First persist `inspect_intent` with `I0/M0`, then use
no-follow FDs to add only owner-read, hash twice with identical digest and stable
identity/size/mtime, restore exact mode, fsync, and persist `reserved` with `D0`.
`inspect_intent` accepts only target `I0` at original mode or that single
temporary read-bit mode, with `B/S` absent; restart restores/fsyncs mode, removes
`J`, fsyncs its directory, and repeats if `J` reappears. Unreadable
non-owner-writable files error before chmod. The same journaled helper verifies
a 0200 `B`; ctime is diagnostic, while all fields in `M0` must match.

Each phase update fully writes/fdatasyncs new `T`, renames it over `J`, and
fsyncs the journal directory. Scanner locks `R`: with `J` absent and `S/B`
absent it removes an owned empty/partial `T`; with `J` present it removes owned
`T` and uses `J`. A crash during that unlink may yield `T` present or absent and
repeats safely. Any other J-absent artifact or proof failure is ambiguous.

Writer phases are `reserved`, `staged`, `swap_intent`, `backed_up`, `swapped`,
DB commit, then `sealed`. File/no-seal phase→states are exactly:
`reserved:(D0,Ø,Ø|D1)`; `staged:(D0,Ø,D1)`; `swap_intent:(D0,Ø,D1)|(Ø,D0,D1)`;
`backed_up:(Ø,D0,D1)|(D1,D0,Ø)`; `swapped:(D1,D0,Ø)`. Absent/no-seal uses
`reserved:(Ø,Ø,Ø|D1)`, `staged|swap_intent:(Ø,Ø,D1)`,
`backed_up:(Ø,Ø,D1)|(D1,Ø,Ø)`, `swapped:(D1,Ø,Ø)`. Matching seal requires
file `swapped:(D1,D0,Ø)` or `sealed:(D1,D0|Ø,Ø)`; absent is `(D1,Ø,Ø)`.
Target→B is no-replace after `swap_intent`; S→target/target/parents are fsynced.
Exact retry creates no journal. Only the DB seal proves commit.

Before filesystem recovery, validate the writer phase/physical tuple, persist
and fsync exactly one recovery phase, then mutate. `Ø` means absent; every `D0`
also requires `I0/M0` (or the documented temporary read bit while hashing):

| Recovery phase | DB goal | Accepted `(target,B,S)` states | Idempotent normalization |
| --- | --- | --- | --- |
| `recover_before_file` | no seal, prior file | `(D0,Ø,Ø\|D1)`, `(Ø,D0,D1)`, `(D1,D0,Ø)` | restore `B` no-replace into Ø or atomically over exact D1; unlink exact S |
| `recover_before_absent` | no seal, prior absent | `(Ø,Ø,Ø\|D1)`, `(D1,Ø,Ø)` | unlink exact S/D1 target |
| `recover_after_file` | matching seal, prior file | `(D1,D0,Ø)`, `(D1,Ø,Ø)` | keep D1; unlink proved B |
| `recover_after_absent` | matching seal, prior absent | `(D1,Ø,Ø)` | keep D1 |

The recovery phase remains unchanged through every rename/unlink, target
fdatasync, and parent fsync. Each post-action/pre-fsync and post-fsync state is
an adjacent accepted row above, so a second or repeated crash resumes the same
normalization rather than reclassifying it ambiguous. After the terminal tuple
is fsynced, unlink `J` and fsync its directory; another crash yields either the
same terminal recovery row or no journal/artifacts. Rehearsal expands these
transitions into all post-action states and kills after every recovery syscall.

Every unlisted tuple, earlier-phase seal, missing/mismatched DB result, wrong
digest/identity/metadata/path, escape, alias, or unproved artifact is
`local_copy_reconciliation_ambiguous`: preserve all bytes/J, error with opaque
`R`/phase, and keep doctor nonhealthy. No-seal converges to exact D0/absence;
seal converges to D1. Cleanup failure never rewrites the sealed response.
## Completion Evidence

Rehearsal must match SQL and prove UDFs, typed/sealed writers, retry/duplicate,
crash boundaries, and each contract file ≤800 lines.
