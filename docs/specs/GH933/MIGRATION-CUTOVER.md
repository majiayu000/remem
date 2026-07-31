# GH933 Migration and Cutover Contract
Refs #933.
## Status and Authority

This is the normative Phase A v2 migration, retry-ledger, hashing, and local-copy cutover contract referenced by `TECH.md`. It remains pending until implementation, `MIGRATION-REHEARSAL.md` evidence, and `ROLLOUT.md` gates pass. SQL is executable, not pseudocode; production preserves every constraint and trigger body.
The breaking cutover runs in a maintenance window: all 0.6.x writers remain stopped from before the foreground transaction through new-binary postflight. There is no mixed-writer mode or down migration after a v2 write.
## Implementation Scope

- `Cargo.toml`/`Cargo.lock`: enable rusqlite `functions`.
- `src/db/sql_functions.rs` and every connection constructor: register the versioned function after SQLCipher keying and before schema access or writes.
- The migration SQL/runner install this DDL, rebuild `memories`, and backfill in one `BEGIN IMMEDIATE`.
- Every insert and named route/lifecycle update creates intent before mutation, populates all declared bindings, and seals last.
- `src/memory/service/{types,save,local_copy}.rs` and all API/MCP save adapters require the caller key and journal protocol.
- `src/doctor/` reconciles safe journals and visibly reports every pending or ambiguous journal.
- Run the migration/API/writer/DDL/UDF/retry/fault tests in the rehearsal.
No connection may register different framing; no fallback hash is legal.
## Versioned SHA-256 Data Flow

`remem_sha256_frame_v1` is variadic and takes alternating names and values:

```text
remem_sha256_frame_v1(name_0, value_0, name_1, value_1, ...)
```

It rejects zero/odd argument counts and non-TEXT, blank, non-ASCII, or duplicate field names. For each ordered pair it feeds SHA-256:

```text
u32_be(name UTF-8 byte length)
name UTF-8 bytes
u8(type)                         # 0=NULL, 1=INTEGER, 2=REAL, 3=TEXT, 4=BLOB
u64_be(value byte length)
value bytes
```

INTEGER is signed i64 big-endian; REAL is exact IEEE-754 f64 bits in big-endian; TEXT/BLOB use exact bytes; NULL has length zero and differs from empty. Return is exactly 64 lowercase hex; registration is `DETERMINISTIC | INNOCUOUS`, and failure aborts. Rust hashes requests before SQL; SQL chains results and hashes request/terminal/schema/response while triggers hash typed OLD/NEW. Golden vectors cover NULL/empty, i64 bounds, negative zero/non-finite rejection, multibyte/NUL TEXT, BLOB, pair order and duplicate names against independent Python.
## Caller Idempotency

Every direct save entrypoint requires `idempotency_key`; the adapter trims ASCII outer whitespace once, then requires 1–128 bytes in `[A-Za-z0-9._~-]` and derives:

```text
request_id = "save_" || lower_hex(
    SHA-256("remem/save-idempotency/v1\0" || normalized_key)
)
```

Only `request_id` is retained; raw/normalized keys never enter serialization, database, journals, logs, errors, traces, metrics, or responses. Fingerprint excludes key/credentials and covers every other raw field, Option presence, list order/duplicates, reference time, defaults, and effective inputs:

| Existing row | Incoming key/payload | Result |
| --- | --- | --- |
| none | any valid key/payload | execute once |
| sealed, equal request fingerprint | same key/equal payload | return stored response without mutation |
| sealed, different request fingerprint | same key/different payload | `idempotency_conflict` before mutation |
| any | different key/byte-identical payload | execute as a distinct request |
| intent without seal | any retry after restart | impossible after DB rollback; journal reconciliation runs first |

Different keys preserve the second lesson reinforcement, operation, claim, and knowledge transition.
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
      CHECK (writer_kind NOT GLOB '*[^0-9a-z._:-]*'),
    request_id TEXT NOT NULL
      CHECK (length(request_id) BETWEEN 1 AND 128)
      CHECK (request_id NOT GLOB '*[^0-9a-z._:-]*'),
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
    previous_status TEXT CHECK (previous_status IS NULL OR previous_status IN ('active','stale','superseded','archived','deleted','rejected')),
    new_status TEXT NOT NULL CHECK (new_status IN ('active','stale','superseded','archived','deleted','rejected')),
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
CREATE TRIGGER memory_route_ledger_fingerprint_guard BEFORE INSERT ON memory_route_ledger BEGIN
  SELECT CASE WHEN (NEW.route_version=1 AND (NEW.previous_route_id IS NOT NULL OR NEW.source_kind NOT IN ('insert','legacy_backfill'))) OR (NEW.route_version>1 AND NOT EXISTS (SELECT 1 FROM memory_route_ledger AS OLD WHERE OLD.id=NEW.previous_route_id AND OLD.memory_id=NEW.memory_id AND OLD.route_version=NEW.route_version-1 AND OLD.effective_at_epoch<=NEW.effective_at_epoch)) THEN RAISE(ABORT, 'invalid route predecessor') END;
  SELECT CASE WHEN NEW.source_fingerprint IS NOT remem_sha256_frame_v1('domain','memory_route_ledger/v1','old_memory_id',(SELECT memory_id FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_route_version',(SELECT route_version FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_previous_route_id',(SELECT previous_route_id FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_effective_at_epoch',(SELECT effective_at_epoch FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_source_kind',(SELECT source_kind FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_audit_event_id',(SELECT audit_event_id FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_source_writer_kind',(SELECT source_writer_kind FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_source_ref',(SELECT source_ref FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_source_result_ordinal',(SELECT source_result_ordinal FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_request_fingerprint',(SELECT request_fingerprint FROM memory_write_requests WHERE writer_kind=(SELECT source_writer_kind FROM memory_route_ledger WHERE id=NEW.previous_route_id) AND request_id=(SELECT source_ref FROM memory_route_ledger WHERE id=NEW.previous_route_id)),'old_coverage_kind',(SELECT coverage_kind FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_coverage_start_epoch',(SELECT coverage_start_epoch FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_placement_project',(SELECT placement_project FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_source_project',(SELECT source_project FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_target_project',(SELECT target_project FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_owner_scope',(SELECT owner_scope FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_owner_key',(SELECT owner_key FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_memory_type',(SELECT memory_type FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_topic_key',(SELECT topic_key FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_topic_domain',(SELECT topic_domain FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_routing_confidence',(SELECT routing_confidence FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_routing_reason',(SELECT routing_reason FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_context_class',(SELECT context_class FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_memory_scope',(SELECT memory_scope FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_branch',(SELECT branch FROM memory_route_ledger WHERE id=NEW.previous_route_id),'new_memory_id',NEW.memory_id,'new_route_version',NEW.route_version,'new_previous_route_id',NEW.previous_route_id,'new_effective_at_epoch',NEW.effective_at_epoch,'new_source_kind',NEW.source_kind,'new_audit_event_id',NEW.audit_event_id,'new_source_writer_kind',NEW.source_writer_kind,'new_source_ref',NEW.source_ref,'new_source_result_ordinal',NEW.source_result_ordinal,'new_request_fingerprint',(SELECT request_fingerprint FROM memory_write_requests WHERE writer_kind=NEW.source_writer_kind AND request_id=NEW.source_ref),'new_coverage_kind',NEW.coverage_kind,'new_coverage_start_epoch',NEW.coverage_start_epoch,'new_placement_project',NEW.placement_project,'new_source_project',NEW.source_project,'new_target_project',NEW.target_project,'new_owner_scope',NEW.owner_scope,'new_owner_key',NEW.owner_key,'new_memory_type',NEW.memory_type,'new_topic_key',NEW.topic_key,'new_topic_domain',NEW.topic_domain,'new_routing_confidence',NEW.routing_confidence,'new_routing_reason',NEW.routing_reason,'new_context_class',NEW.context_class,'new_memory_scope',NEW.memory_scope,'new_branch',NEW.branch) THEN RAISE(ABORT, 'route fingerprint mismatch') END;
END;
CREATE TRIGGER memory_lifecycle_ledger_fingerprint_guard BEFORE INSERT ON memory_lifecycle_ledger BEGIN
  SELECT CASE WHEN (NEW.lifecycle_version=1 AND (NEW.previous_lifecycle_id IS NOT NULL OR NEW.previous_status IS NOT NULL OR NEW.source_kind NOT IN ('insert','legacy_backfill') OR NEW.source_action<>'baseline')) OR (NEW.lifecycle_version>1 AND NOT EXISTS (SELECT 1 FROM memory_lifecycle_ledger AS OLD WHERE OLD.id=NEW.previous_lifecycle_id AND OLD.memory_id=NEW.memory_id AND OLD.lifecycle_version=NEW.lifecycle_version-1 AND OLD.new_status=NEW.previous_status AND OLD.effective_at_epoch<=NEW.effective_at_epoch)) THEN RAISE(ABORT, 'invalid lifecycle predecessor') END;
  SELECT CASE WHEN NEW.source_fingerprint IS NOT remem_sha256_frame_v1('domain','memory_lifecycle_ledger/v1','old_memory_id',(SELECT memory_id FROM memory_lifecycle_ledger WHERE id=NEW.previous_lifecycle_id),'old_lifecycle_version',(SELECT lifecycle_version FROM memory_lifecycle_ledger WHERE id=NEW.previous_lifecycle_id),'old_previous_lifecycle_id',(SELECT previous_lifecycle_id FROM memory_lifecycle_ledger WHERE id=NEW.previous_lifecycle_id),'old_effective_at_epoch',(SELECT effective_at_epoch FROM memory_lifecycle_ledger WHERE id=NEW.previous_lifecycle_id),'old_previous_status',(SELECT previous_status FROM memory_lifecycle_ledger WHERE id=NEW.previous_lifecycle_id),'old_new_status',(SELECT new_status FROM memory_lifecycle_ledger WHERE id=NEW.previous_lifecycle_id),'old_source_kind',(SELECT source_kind FROM memory_lifecycle_ledger WHERE id=NEW.previous_lifecycle_id),'old_source_action',(SELECT source_action FROM memory_lifecycle_ledger WHERE id=NEW.previous_lifecycle_id),'old_source_operation_id',(SELECT source_operation_id FROM memory_lifecycle_ledger WHERE id=NEW.previous_lifecycle_id),'old_source_api_operation_id',(SELECT source_api_operation_id FROM memory_lifecycle_ledger WHERE id=NEW.previous_lifecycle_id),'old_audit_event_id',(SELECT audit_event_id FROM memory_lifecycle_ledger WHERE id=NEW.previous_lifecycle_id),'old_source_writer_kind',(SELECT source_writer_kind FROM memory_lifecycle_ledger WHERE id=NEW.previous_lifecycle_id),'old_source_ref',(SELECT source_ref FROM memory_lifecycle_ledger WHERE id=NEW.previous_lifecycle_id),'old_source_result_ordinal',(SELECT source_result_ordinal FROM memory_lifecycle_ledger WHERE id=NEW.previous_lifecycle_id),'old_request_fingerprint',(SELECT request_fingerprint FROM memory_write_requests WHERE writer_kind=(SELECT source_writer_kind FROM memory_lifecycle_ledger WHERE id=NEW.previous_lifecycle_id) AND request_id=(SELECT source_ref FROM memory_lifecycle_ledger WHERE id=NEW.previous_lifecycle_id)),'old_coverage_kind',(SELECT coverage_kind FROM memory_lifecycle_ledger WHERE id=NEW.previous_lifecycle_id),'old_coverage_start_epoch',(SELECT coverage_start_epoch FROM memory_lifecycle_ledger WHERE id=NEW.previous_lifecycle_id),'new_memory_id',NEW.memory_id,'new_lifecycle_version',NEW.lifecycle_version,'new_previous_lifecycle_id',NEW.previous_lifecycle_id,'new_effective_at_epoch',NEW.effective_at_epoch,'new_previous_status',NEW.previous_status,'new_new_status',NEW.new_status,'new_source_kind',NEW.source_kind,'new_source_action',NEW.source_action,'new_source_operation_id',NEW.source_operation_id,'new_source_api_operation_id',NEW.source_api_operation_id,'new_audit_event_id',NEW.audit_event_id,'new_source_writer_kind',NEW.source_writer_kind,'new_source_ref',NEW.source_ref,'new_source_result_ordinal',NEW.source_result_ordinal,'new_request_fingerprint',(SELECT request_fingerprint FROM memory_write_requests WHERE writer_kind=NEW.source_writer_kind AND request_id=NEW.source_ref),'new_coverage_kind',NEW.coverage_kind,'new_coverage_start_epoch',NEW.coverage_start_epoch) THEN RAISE(ABORT, 'lifecycle fingerprint mismatch') END;
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
  SELECT CASE WHEN EXISTS (
    SELECT 1 FROM memory_route_ledger AS route JOIN memories AS memory ON memory.id=route.memory_id
    WHERE route.source_writer_kind=NEW.writer_kind AND route.source_ref=NEW.request_id
      AND (EXISTS (SELECT 1 FROM memory_route_ledger AS successor WHERE successor.previous_route_id=route.id)
        OR route.placement_project IS NOT memory.project OR route.source_project IS NOT memory.source_project OR route.target_project IS NOT memory.target_project OR route.owner_scope IS NOT memory.owner_scope OR route.owner_key IS NOT memory.owner_key OR route.memory_type IS NOT memory.memory_type OR route.topic_key IS NOT memory.topic_key OR route.topic_domain IS NOT memory.topic_domain OR route.routing_confidence IS NOT memory.routing_confidence OR route.routing_reason IS NOT memory.routing_reason OR route.context_class IS NOT memory.context_class OR route.memory_scope IS NOT memory.scope OR route.branch IS NOT memory.branch)
  ) THEN RAISE(ABORT, 'route terminal does not match memory at seal') END;
  SELECT CASE WHEN EXISTS (
    SELECT 1 FROM memory_lifecycle_ledger AS lifecycle JOIN memories AS memory ON memory.id=lifecycle.memory_id
    WHERE lifecycle.source_writer_kind=NEW.writer_kind AND lifecycle.source_ref=NEW.request_id
      AND (EXISTS (SELECT 1 FROM memory_lifecycle_ledger AS successor WHERE successor.previous_lifecycle_id=lifecycle.id) OR lifecycle.new_status IS NOT memory.status)
  ) THEN RAISE(ABORT, 'lifecycle terminal does not match memory at seal') END;
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
CREATE TRIGGER memory_insert_v1_ledgers AFTER INSERT ON memories BEGIN
  SELECT CASE WHEN EXISTS (SELECT 1 FROM memory_write_request_commits WHERE writer_kind=NEW.insert_writer_kind AND request_id=NEW.insert_request_id) OR NOT EXISTS (SELECT 1 FROM memory_write_requests AS request,json_each(request.expected_results_json) AS expected WHERE request.writer_kind=NEW.insert_writer_kind AND request.request_id=NEW.insert_request_id AND json_extract(expected.value,'$.result_ordinal')=NEW.insert_result_ordinal AND json_extract(expected.value,'$.binding_kind')='insert_origin') THEN RAISE(ABORT, 'memory insert lacks open insert_origin') END;
  INSERT INTO memory_route_ledger(memory_id,route_version,previous_route_id,effective_at_epoch,source_kind,audit_event_id,source_writer_kind,source_ref,source_result_ordinal,source_fingerprint,coverage_kind,coverage_start_epoch,placement_project,source_project,target_project,owner_scope,owner_key,memory_type,topic_key,topic_domain,routing_confidence,routing_reason,context_class,memory_scope,branch) SELECT NEW.id,1,NULL,request.requested_at_epoch,CASE WHEN NEW.insert_writer_kind='legacy_backfill' THEN 'legacy_backfill' ELSE 'insert' END,NULL,NEW.insert_writer_kind,NEW.insert_request_id,NEW.insert_result_ordinal,remem_sha256_frame_v1('domain','memory_route_ledger/v1','old_memory_id',NULL,'old_route_version',NULL,'old_previous_route_id',NULL,'old_effective_at_epoch',NULL,'old_source_kind',NULL,'old_audit_event_id',NULL,'old_source_writer_kind',NULL,'old_source_ref',NULL,'old_source_result_ordinal',NULL,'old_request_fingerprint',NULL,'old_coverage_kind',NULL,'old_coverage_start_epoch',NULL,'old_placement_project',NULL,'old_source_project',NULL,'old_target_project',NULL,'old_owner_scope',NULL,'old_owner_key',NULL,'old_memory_type',NULL,'old_topic_key',NULL,'old_topic_domain',NULL,'old_routing_confidence',NULL,'old_routing_reason',NULL,'old_context_class',NULL,'old_memory_scope',NULL,'old_branch',NULL,'new_memory_id',NEW.id,'new_route_version',1,'new_previous_route_id',NULL,'new_effective_at_epoch',request.requested_at_epoch,'new_source_kind',CASE WHEN NEW.insert_writer_kind='legacy_backfill' THEN 'legacy_backfill' ELSE 'insert' END,'new_audit_event_id',NULL,'new_source_writer_kind',NEW.insert_writer_kind,'new_source_ref',NEW.insert_request_id,'new_source_result_ordinal',NEW.insert_result_ordinal,'new_request_fingerprint',request.request_fingerprint,'new_coverage_kind','complete','new_coverage_start_epoch',request.requested_at_epoch,'new_placement_project',NEW.project,'new_source_project',NEW.source_project,'new_target_project',NEW.target_project,'new_owner_scope',NEW.owner_scope,'new_owner_key',NEW.owner_key,'new_memory_type',NEW.memory_type,'new_topic_key',NEW.topic_key,'new_topic_domain',NEW.topic_domain,'new_routing_confidence',NEW.routing_confidence,'new_routing_reason',NEW.routing_reason,'new_context_class',NEW.context_class,'new_memory_scope',NEW.scope,'new_branch',NEW.branch),'complete',request.requested_at_epoch,NEW.project,NEW.source_project,NEW.target_project,NEW.owner_scope,NEW.owner_key,NEW.memory_type,NEW.topic_key,NEW.topic_domain,NEW.routing_confidence,NEW.routing_reason,NEW.context_class,NEW.scope,NEW.branch FROM memory_write_requests AS request WHERE request.writer_kind=NEW.insert_writer_kind AND request.request_id=NEW.insert_request_id;
  INSERT INTO memory_lifecycle_ledger(memory_id,lifecycle_version,previous_lifecycle_id,effective_at_epoch,previous_status,new_status,source_kind,source_action,source_operation_id,source_api_operation_id,audit_event_id,source_writer_kind,source_ref,source_result_ordinal,source_fingerprint,coverage_kind,coverage_start_epoch) SELECT NEW.id,1,NULL,request.requested_at_epoch,NULL,NEW.status,CASE WHEN NEW.insert_writer_kind='legacy_backfill' THEN 'legacy_backfill' ELSE 'insert' END,'baseline',NULL,NULL,NULL,NEW.insert_writer_kind,NEW.insert_request_id,NEW.insert_result_ordinal,remem_sha256_frame_v1('domain','memory_lifecycle_ledger/v1','old_memory_id',NULL,'old_lifecycle_version',NULL,'old_previous_lifecycle_id',NULL,'old_effective_at_epoch',NULL,'old_previous_status',NULL,'old_new_status',NULL,'old_source_kind',NULL,'old_source_action',NULL,'old_source_operation_id',NULL,'old_source_api_operation_id',NULL,'old_audit_event_id',NULL,'old_source_writer_kind',NULL,'old_source_ref',NULL,'old_source_result_ordinal',NULL,'old_request_fingerprint',NULL,'old_coverage_kind',NULL,'old_coverage_start_epoch',NULL,'new_memory_id',NEW.id,'new_lifecycle_version',1,'new_previous_lifecycle_id',NULL,'new_effective_at_epoch',request.requested_at_epoch,'new_previous_status',NULL,'new_new_status',NEW.status,'new_source_kind',CASE WHEN NEW.insert_writer_kind='legacy_backfill' THEN 'legacy_backfill' ELSE 'insert' END,'new_source_action','baseline','new_source_operation_id',NULL,'new_source_api_operation_id',NULL,'new_audit_event_id',NULL,'new_source_writer_kind',NEW.insert_writer_kind,'new_source_ref',NEW.insert_request_id,'new_source_result_ordinal',NEW.insert_result_ordinal,'new_request_fingerprint',request.request_fingerprint,'new_coverage_kind','complete','new_coverage_start_epoch',request.requested_at_epoch),'complete',request.requested_at_epoch FROM memory_write_requests AS request WHERE request.writer_kind=NEW.insert_writer_kind AND request.request_id=NEW.insert_request_id;
END;
CREATE TRIGGER memory_route_tuple_update_guard
BEFORE UPDATE OF project,source_project,target_project,owner_scope,owner_key,memory_type,topic_key,topic_domain,routing_confidence,routing_reason,context_class,scope,branch ON memories
WHEN NEW.project IS NOT OLD.project OR NEW.source_project IS NOT OLD.source_project OR NEW.target_project IS NOT OLD.target_project OR NEW.owner_scope IS NOT OLD.owner_scope OR NEW.owner_key IS NOT OLD.owner_key OR NEW.memory_type IS NOT OLD.memory_type OR NEW.topic_key IS NOT OLD.topic_key
  OR NEW.topic_domain IS NOT OLD.topic_domain OR NEW.routing_confidence IS NOT OLD.routing_confidence OR NEW.routing_reason IS NOT OLD.routing_reason OR NEW.context_class IS NOT OLD.context_class OR NEW.scope IS NOT OLD.scope OR NEW.branch IS NOT OLD.branch
BEGIN
  SELECT CASE WHEN NOT EXISTS (
    SELECT 1 FROM memory_route_ledger AS new_route
    JOIN memory_route_ledger AS old_route ON old_route.id=new_route.previous_route_id
    WHERE new_route.memory_id=OLD.id AND old_route.memory_id=OLD.id AND new_route.route_version=old_route.route_version+1
      AND NOT EXISTS (SELECT 1 FROM memory_route_ledger AS successor WHERE successor.previous_route_id=new_route.id)
      AND NOT EXISTS (SELECT 1 FROM memory_write_request_commits AS commit_row WHERE commit_row.writer_kind=new_route.source_writer_kind AND commit_row.request_id=new_route.source_ref)
      AND old_route.placement_project IS OLD.project AND old_route.source_project IS OLD.source_project AND old_route.target_project IS OLD.target_project AND old_route.owner_scope IS OLD.owner_scope AND old_route.owner_key IS OLD.owner_key AND old_route.memory_type IS OLD.memory_type AND old_route.topic_key IS OLD.topic_key AND old_route.topic_domain IS OLD.topic_domain AND old_route.routing_confidence IS OLD.routing_confidence AND old_route.routing_reason IS OLD.routing_reason AND old_route.context_class IS OLD.context_class AND old_route.memory_scope IS OLD.scope AND old_route.branch IS OLD.branch
      AND new_route.placement_project IS NEW.project AND new_route.source_project IS NEW.source_project AND new_route.target_project IS NEW.target_project AND new_route.owner_scope IS NEW.owner_scope AND new_route.owner_key IS NEW.owner_key AND new_route.memory_type IS NEW.memory_type AND new_route.topic_key IS NEW.topic_key AND new_route.topic_domain IS NEW.topic_domain AND new_route.routing_confidence IS NEW.routing_confidence AND new_route.routing_reason IS NEW.routing_reason AND new_route.context_class IS NEW.context_class AND new_route.memory_scope IS NEW.scope AND new_route.branch IS NEW.branch
  ) THEN RAISE(ABORT, 'memory route update lacks matching staged next version') END;
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

`memory_route_ledger_fingerprint_guard` and `memory_lifecycle_ledger_fingerprint_guard` hash every typed OLD/NEW column except row ID/digest, including both request fingerprints; `memory_insert_v1_ledgers` atomically creates both v1 rows; `memory_route_tuple_update_guard` requires an open exact next route; and `memory_write_commit_guard` requires request-owned ledger terminals to match current memory. These literal bodies are the sole executable authority: no templates, SQL concatenation, post-insert patch, or fallback hash.

## Backfill and Foreground Cutover

The migration runner performs these steps under one exclusive maintenance
window. Steps 1–2 precede any migration write transaction; steps 3–5 use one
uninterrupted `BEGIN IMMEDIATE`:

1. Register and self-test `remem_sha256_frame_v1`; reject a missing function,
   wrong golden vector, disabled FK enforcement, or nonempty migration journal.
2. Verify schema, checkpoint WAL after all writers stop, close every handle, copy
   the main database byte-for-byte, fsync file/directory, hash, and test-open it.
3. Reopen the exact live database, register/self-test the UDF, revalidate
   database/schema/backup identity, snapshot every dependent FK/table object,
   set `foreign_keys=OFF` and verify it before starting `BEGIN IMMEDIATE`.
4. Create `memories_rebuild` with the exact target schema, copy and validate all
   rows, drop old `memories`, rename the rebuild, and recreate exact owned
   indexes/triggers/FTS without altering dependent tables. Then create
   ledgers/results. For each legacy ID, use
   `migration_vNNN:<memory_id>` with sorted `insert_origin`/`response_aux`
   manifest. Exhaustive durable evidence may form complete history; otherwise
   create only forward-only baselines. Never infer from current bytes/events.
5. Append typed bindings/response/seals and install literal guards. Before
   commit require row/count/digest/object equality, unchanged dependent DDL,
   `integrity_check='ok'`, and empty `foreign_key_check`; commit, immediately
   restore/verify `foreign_keys=ON`, then repeat both checks before any writer.

Postflight also requires zero unsealed requests, exact manifest/results, one
valid terminal route/lifecycle per memory matching `memories`, valid origin/v1
maps, and no schema or dependent-row/object drift.

Failure before step 3 leaves the live database unmodified; every precommit
failure rolls back that one transaction. A postcommit FK-restore/check failure
discards the connection and blocks writers. The operator restores the backup
only after proving the failed
database is closed. Once a v2 writer seals any non-migration request, rollback
means disabling the v2 projection while retaining schema/history; running 0.6.x
or restoring the old backup would lose writes and is forbidden.

## Durable Local-Copy Journal
The verified nonsymlink journal root `Q=${REMEM_DATA_DIR}/write-journal/save/` and `Q/locks/` are app-owned mode 0700. For opaque request `R`, names are retained lock `L=Q/locks/R.lock`, journal `J=Q/R.json`, journal temp `T=Q/.R.json.tmp`, private stage build `U=Q/.R.<nonce>.stage-build`, and target-parent siblings `S=.remem-save-R.stage`, `B=.remem-save-R.backup`. Scanner grammar reserves proved `Q/locks/` plus J/T/U; L is retained and excluded from pending-artifact counts.
Resolve configured local-copy root and target parent `P` component-by-component from directory FDs with no-follow/beneath semantics. Convert an allowed absolute input to its root-relative descendant; reject outside-root/`..` escape, symlink/non-directory components, Q alias, wrong uid, missing owner rwx, group/world write, or missing fsync/no-replace. Securely create missing descendants. Mode 0755 is valid; Q, P, target, U, S and B must share one device. A present target additionally requires atomic exchange support proved before any target/B mutation.
Keep the `P` fd open, operate on exact basenames relative to it, and record root-relative path, `IP=(dev,ino)` and `MP=(uid,gid,mode)`. Re-resolve and match `IP/MP` before publication or mutation and after reopening for recovery; replacement or permission drift is a visible no-mutation identity error, while already-open operations remain bound to the proved directory.
Canonical J records fingerprints, phase/goal, paths, `IP/MP`, `before_kind`, D1/D0, epoch, `I0=(dev,ino)`, `M0=(uid,gid,mode,nlink,size,mtime_ns)`, and for stage build a CSPRNG 128-bit lowercase-hex nonce plus optional `IU=(dev,ino)`. A compensation intent also records the observed competitor `IC/MC/DC`; J never records content, key, token, or response.

Create `L` initially with `O_CREAT|O_EXCL|O_NOFOLLOW` mode 0600 or reopen that exact current-uid regular single-link file through `Q/locks`; Phase A never unlinks or replaces it. A wrong L/locks-dir path, inode, type, uid, mode or link count is `local_copy_lock_unsafe` before any R inspection. The local-copy writer, startup scanner, doctor and reconciler all take an exclusive nonblocking OS lock on that same inode, not merely a process mutex or PID/age heuristic, before opening J/artifacts or reading/mutating R-scoped database state. Independently opened descriptors must contend across processes; add in-process serialization where the platform primitive needs it. Lock order is always L then database.
The writer takes L before `inspect_intent`, J/T/U/S/B, `BEGIN IMMEDIATE`, or target mutation and holds it through committed seal, cleanup/J fsync, terminal reconciliation, or a visible no-mutation ambiguity. Busy scanner/doctor/reconciler returns `local_copy_writer_in_progress` without inspecting J/U/S/B/target or R-scoped DB state; this includes D1 before seal. Crash releases only the OS lock, after which exactly one contender reconciles.

Proof classes and parent locations are deliberately different:

| Path | Required source and proof |
| --- | --- |
| `L` | first-created/reopened only below `Q/locks`; exact stable inode/current uid/regular/nlink=1/mode 0600; never removed |
| `T` | remem creates below private `Q` with `O_CREAT\|O_EXCL\|O_NOFOLLOW`, mode 0600; exact name/current uid/regular/nlink=1 |
| `U` | after durable `stage_building`, remem creates the nonce name below private Q with T's creation proof; arbitrary bytes are owned partial build only under that exact proof |
| `S` | only atomic no-replace publication of fully fdatasynced U after durable `stage_ready`; below verified P with `IU`, exact mode 0600/current uid/regular/nlink=1/entry identity/D1 |
| `B` | after durable `swap_intent`, an atomic no-replace hard-link pin accepted only when target and B both still prove `I0/M0/D0`, the same inode, and nlink=2; later B/S may be that same proved pair |
| target | exact basename below verified `P`; verified current-uid regular nlink=1 identity/metadata/digest or recorded absence; symlink, alias, and nonregular types are forbidden |
Thus S never exists empty/partial and T/U private-parent proof never replaces S's P/IU/D1 proof. U→S and absent-target S→target use Linux `renameat2(RENAME_NOREPLACE)`, macOS `renameatx_np(RENAME_EXCL)`, or portable `linkat` create-if-absent plus proved same-inode nlink=2 and source unlink/fsync (`D1-link`). For a present target, after durable `swap_intent` create B with no-replace `linkat(target,B)`, reverify target+B are exactly the I0/D0 pair, fsync P, persist `backed_up`, then persist `exchange_intent` before atomically exchanging S and target with Linux `renameat2(RENAME_EXCHANGE)` or macOS `renameatx_np(RENAME_SWAP)`. Prove target=IU/D1 plus B/S=I0/D0 before `swapped`. There is no portable or plain-rename fallback for present targets.
Durable `exchange_intent` accepts the exact pre-exchange tuple, normal exchanged tuple, or captured-competitor tuple `(target=D1,B=D0,S=C)`. In the last case, recovery first proves C and persists `compensate_intent` with exact `IC/MC/DC`, then reverse-exchanges only target=IU/D1 and S=C; success must prove C restored at target and D1 at S. Any precondition/postcondition drift preserves target/J/B/S, remains unsealed, and is `local_copy_reconciliation_ambiguous`; stable compensation returns `local_copy_publish_collision`. No unproved entry is unlinked or classified as D0.

Unreadable owner-writable targets including 0200 need no readability precondition: persist `inspect_intent` with I0/M0, add only owner-read through no-follow FDs, double-hash under stable identity/size/mtime, restore exact mode, fsync, then persist `reserved` with D0. Recovery accepts only I0 at original/single-read-bit mode with B/S/U absent, restores/fsyncs mode and removes J reentrantly. Non-owner-writable errors before chmod; the same helper verifies 0200 B/S, allowing only the journaled nlink=2 pin while requiring every other M0 field and D0 exact.

While holding L, each phase update writes/fdatasyncs T, renames it over J and fsyncs Q. With J absent and U/S/B absent the scanner may remove proved partial T; with J present it removes T and uses J. Any J-absent U/S/B or failed proof is ambiguous.

After `reserved`, persist `stage_building` with nonce/D1 and `IU=NULL`, then O_EXCL-create U, fstat it and persist the same phase with IU before the first content byte; write injected chunks, fdatasync and double-check IU/D1, then persist `stage_ready`. A crash in the create→IU-fsync gap still owns U only through its exact nonce/private-Q/type/uid/mode/nlink proof. `stage_building` accepts U absent or any empty/partial/full bytes under that proof and no S; `stage_ready` accepts full U, full S with IU after no-replace U→S, or portable same-inode U+S/nlink=2. Existing before-goal recovery unlinks only proved U/S and fsyncs Q/P; wrong proof or wrong-byte S is ambiguous. Persist `staged` only after durable S=D1/U-absent.

Writer phases are `reserved,stage_building,stage_ready,staged,swap_intent,backed_up,exchange_intent,swapped`, DB commit, then `sealed`; `compensate_intent` is exceptional/unsealed. In `(target,B,S,U)`, present states are `reserved:(D0,Ø,Ø,Ø)`; `stage_building:(D0,Ø,Ø,U*)`; `stage_ready:(D0,Ø,Ø,D1)|(D0,Ø,D1,Ø|D1-link)`; `staged:(D0,Ø,D1,Ø)`; `swap_intent:(D0,Ø,D1,Ø)|(D0,D0-link,D1,Ø)`; `backed_up:(D0,D0-link,D1,Ø)`; `exchange_intent:(D0,D0-link,D1,Ø)|(D1,D0-link,D0-link,Ø)|(D1,D0,C,Ø)`; `swapped:(D1,D0-link,D0-link,Ø)`; compensation is `(D1,D0,C,Ø)→(C,D0,D1,Ø)` with exact proofs. Absent uses the build rows then atomic no-replace `(Ø,Ø,D1,Ø)→(D1,Ø,Ø,Ø)`. Matching seal allows only normal D1; cleanup removes proved S then B after seal.
Every exchange is postvalidated before phase advance, cleanup, or seal. A competing create/replace is either left at target before mutation, captured in S then durably compensated back to target, or preserved with every artifact on ambiguous drift. Reverify target IU/D1 immediately before seal. Exact retry creates no journal; only the DB seal proves commit.

Before filesystem recovery, validate the writer phase/physical tuple, persist
and fsync exactly one recovery phase, then mutate. `Ø` means absent; every `D0`
also requires `I0/M0` (or the documented temporary read bit while hashing):

| Recovery phase | DB goal | Accepted `(target,B,S,U)` states | Idempotent normalization |
| --- | --- | --- | --- |
| `recover_before_file` | no seal, prior file | `(D0,Ø,Ø,U*)`, `(D0,Ø,D1,Ø\|D1-link)`, `(D0,D0-link,D1,Ø)`, `(D1,D0-link,D0-link,Ø)` | pre-exchange: unlink proved U/S then B; post-exchange: atomically exchange exact D1 target with exact D0 S, verify D0 target/D1 S, then unlink S and B; each step fsyncs P |
| `recover_before_absent` | no seal, prior absent | `(Ø,Ø,Ø,U*)`, `(Ø,Ø,D1,Ø\|D1-link)`, `(D1,Ø,Ø\|D1-link,Ø)` | remove only proved U/S/D1; collision preserves all |
| `recover_after_file` | matching seal, prior file | `(D1,D0-link,D0-link,Ø)`, `(D1,D0,Ø,Ø)`, `(D1,Ø,Ø,Ø)` | keep D1; unlink proved S then B with P fsync after each |
| `recover_after_absent` | matching seal, prior absent | `(D1,Ø,Ø,Ø)` | keep D1 |

Before a recovery exchange, identity drift enters the same durable compensation protocol; the exact reverse restores a stable competitor to target, while any repeated race freezes all names and J without cleanup. The recovery phase remains unchanged through every exchange/unlink, target
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
