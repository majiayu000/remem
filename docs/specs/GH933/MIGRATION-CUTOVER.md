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
Its rebuilt `scope` column is `TEXT NOT NULL CHECK (scope IN ('project','global'))`; the copy materializes `COALESCE(NULLIF(TRIM(scope),''),'project')`, so padded valid values become canonical and every other noncanonical value aborts.

Postflight compares all rebuilt columns/defaults/checks/FKs/indexes/FTS/triggers
with a fresh same-binary database; any omission aborts.
## Executable Ledger DDL

The block runs before any v2 writer. Isolated tests create FK parents with their
production primary-key types.
```sql
PRAGMA foreign_keys = ON;
CREATE TABLE memory_write_lock_anchors (
    request_id TEXT PRIMARY KEY CHECK (length(request_id) BETWEEN 1 AND 128 AND instr(request_id,char(0))=0 AND request_id GLOB '[0-9A-Za-z]*' AND request_id NOT GLOB '*[^-0-9A-Z_a-z]*'),
    lock_dev INTEGER NOT NULL CHECK (typeof(lock_dev)='integer' AND lock_dev >= 0), lock_ino INTEGER NOT NULL CHECK (typeof(lock_ino)='integer' AND lock_ino > 0),
    lock_nonce TEXT NOT NULL CHECK (length(lock_nonce)=32 AND lock_nonce NOT GLOB '*[^0-9a-f]*'),
    anchored_at_epoch INTEGER NOT NULL CHECK (typeof(anchored_at_epoch)='integer' AND anchored_at_epoch >= 0),
    UNIQUE (lock_dev, lock_ino)
) WITHOUT ROWID;
CREATE TABLE memory_write_requests (
    writer_kind TEXT NOT NULL
      CHECK (length(writer_kind) BETWEEN 1 AND 64)
      CHECK (writer_kind NOT GLOB '*[^0-9a-z._:-]*'),
    request_id TEXT NOT NULL
      CHECK (length(request_id) BETWEEN 1 AND 128 AND instr(request_id,char(0))=0)
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
    id INTEGER PRIMARY KEY CHECK (id > 0),
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
    id INTEGER PRIMARY KEY CHECK (id > 0),
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
    CHECK ((source_kind IN ('insert','legacy_backfill') AND lifecycle_version=1 AND previous_status IS NULL AND source_action='baseline' AND source_operation_id IS NULL AND source_api_operation_id IS NULL AND audit_event_id IS NULL) OR (source_kind='memory_governance' AND lifecycle_version>1 AND previous_status IS NOT NULL AND source_operation_id IS NOT NULL AND source_api_operation_id IS NULL AND ((source_action='delete' AND new_status='deleted') OR (source_action='reject' AND new_status='rejected') OR (source_action='stale' AND new_status='stale') OR (source_action='acknowledge_pattern' AND new_status=previous_status))) OR (source_kind='web_governance' AND lifecycle_version>1 AND previous_status IS NOT NULL AND source_api_operation_id IS NOT NULL AND source_operation_id IS NULL AND ((source_action='archive' AND previous_status='active' AND new_status='archived') OR (source_action='restore' AND previous_status='archived' AND new_status='active'))) OR (source_kind='scope_cleanup' AND lifecycle_version>1 AND previous_status IS NOT NULL AND source_operation_id IS NULL AND source_api_operation_id IS NULL AND ((source_action='archive' AND new_status='archived') OR (source_action='reroute' AND new_status=previous_status) OR (source_action='memory_cleanup' AND new_status IN ('active','stale'))))),
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
    operation_id INTEGER, api_operation_id TEXT,
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
    FOREIGN KEY (operation_id) REFERENCES memory_operation_log(id) ON DELETE RESTRICT,
    FOREIGN KEY (api_operation_id) REFERENCES api_mutation_requests(operation_id) ON DELETE RESTRICT,
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
  SELECT CASE WHEN (NEW.route_version=1 AND (NEW.previous_route_id IS NOT NULL OR NEW.source_kind NOT IN ('insert','legacy_backfill'))) OR (NEW.route_version>1 AND (NEW.source_kind IN ('insert','legacy_backfill') OR NOT EXISTS (SELECT 1 FROM memory_route_ledger AS OLD WHERE OLD.id=NEW.previous_route_id AND OLD.memory_id=NEW.memory_id AND OLD.route_version=NEW.route_version-1 AND OLD.effective_at_epoch<=NEW.effective_at_epoch))) THEN RAISE(ABORT, 'invalid route predecessor') END;
  SELECT CASE WHEN NEW.source_fingerprint IS NOT remem_sha256_frame_v1('domain','memory_route_ledger/v1','old_memory_id',(SELECT memory_id FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_route_version',(SELECT route_version FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_previous_route_id',(SELECT previous_route_id FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_effective_at_epoch',(SELECT effective_at_epoch FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_source_kind',(SELECT source_kind FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_audit_event_id',(SELECT audit_event_id FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_source_writer_kind',(SELECT source_writer_kind FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_source_ref',(SELECT source_ref FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_source_result_ordinal',(SELECT source_result_ordinal FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_request_fingerprint',(SELECT request_fingerprint FROM memory_write_requests WHERE writer_kind=(SELECT source_writer_kind FROM memory_route_ledger WHERE id=NEW.previous_route_id) AND request_id=(SELECT source_ref FROM memory_route_ledger WHERE id=NEW.previous_route_id)),'old_coverage_kind',(SELECT coverage_kind FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_coverage_start_epoch',(SELECT coverage_start_epoch FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_placement_project',(SELECT placement_project FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_source_project',(SELECT source_project FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_target_project',(SELECT target_project FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_owner_scope',(SELECT owner_scope FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_owner_key',(SELECT owner_key FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_memory_type',(SELECT memory_type FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_topic_key',(SELECT topic_key FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_topic_domain',(SELECT topic_domain FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_routing_confidence',(SELECT routing_confidence FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_routing_reason',(SELECT routing_reason FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_context_class',(SELECT context_class FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_memory_scope',(SELECT memory_scope FROM memory_route_ledger WHERE id=NEW.previous_route_id),'old_branch',(SELECT branch FROM memory_route_ledger WHERE id=NEW.previous_route_id),'new_memory_id',NEW.memory_id,'new_route_version',NEW.route_version,'new_previous_route_id',NEW.previous_route_id,'new_effective_at_epoch',NEW.effective_at_epoch,'new_source_kind',NEW.source_kind,'new_audit_event_id',NEW.audit_event_id,'new_source_writer_kind',NEW.source_writer_kind,'new_source_ref',NEW.source_ref,'new_source_result_ordinal',NEW.source_result_ordinal,'new_request_fingerprint',(SELECT request_fingerprint FROM memory_write_requests WHERE writer_kind=NEW.source_writer_kind AND request_id=NEW.source_ref),'new_coverage_kind',NEW.coverage_kind,'new_coverage_start_epoch',NEW.coverage_start_epoch,'new_placement_project',NEW.placement_project,'new_source_project',NEW.source_project,'new_target_project',NEW.target_project,'new_owner_scope',NEW.owner_scope,'new_owner_key',NEW.owner_key,'new_memory_type',NEW.memory_type,'new_topic_key',NEW.topic_key,'new_topic_domain',NEW.topic_domain,'new_routing_confidence',NEW.routing_confidence,'new_routing_reason',NEW.routing_reason,'new_context_class',NEW.context_class,'new_memory_scope',NEW.memory_scope,'new_branch',NEW.branch) THEN RAISE(ABORT, 'route fingerprint mismatch') END;
END;
CREATE TRIGGER memory_lifecycle_ledger_fingerprint_guard BEFORE INSERT ON memory_lifecycle_ledger BEGIN
  SELECT CASE WHEN (NEW.lifecycle_version=1 AND (NEW.previous_lifecycle_id IS NOT NULL OR NEW.previous_status IS NOT NULL OR NEW.source_kind NOT IN ('insert','legacy_backfill') OR NEW.source_action<>'baseline')) OR (NEW.lifecycle_version>1 AND (NEW.source_kind IN ('insert','legacy_backfill') OR NEW.source_action='baseline' OR NOT EXISTS (SELECT 1 FROM memory_lifecycle_ledger AS OLD WHERE OLD.id=NEW.previous_lifecycle_id AND OLD.memory_id=NEW.memory_id AND OLD.lifecycle_version=NEW.lifecycle_version-1 AND OLD.new_status=NEW.previous_status AND OLD.effective_at_epoch<=NEW.effective_at_epoch))) THEN RAISE(ABORT, 'invalid lifecycle predecessor') END;
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
  SELECT CASE WHEN NEW.binding_kind<>'lifecycle_transition' AND NEW.api_operation_id IS NOT NULL THEN RAISE(ABORT, 'API operation only valid for lifecycle result') END;
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
      AND NEW.route_ledger_id IS NULL AND NEW.claim_id IS NULL AND NOT (NEW.operation_id IS NOT NULL AND NEW.api_operation_id IS NOT NULL)
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
      AND NEW.operation_id IS NULL AND NEW.claim_id IS NULL AND NEW.audit_event_id IS NULL
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
    'operation_id', NEW.operation_id, 'api_operation_id', NEW.api_operation_id,
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
        OR route.source_result_ordinal<>result.result_ordinal OR route.audit_event_id IS NOT result.audit_event_id
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
        OR lifecycle.source_result_ordinal<>result.result_ordinal OR lifecycle.source_operation_id IS NOT result.operation_id OR lifecycle.source_api_operation_id IS NOT result.api_operation_id OR lifecycle.audit_event_id IS NOT result.audit_event_id
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
          AND result.memory_id=route.memory_id AND result.route_ledger_id=route.id AND result.audit_event_id IS route.audit_event_id
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
          AND result.lifecycle_ledger_id=lifecycle.id AND result.operation_id IS lifecycle.source_operation_id AND result.api_operation_id IS lifecycle.source_api_operation_id AND result.audit_event_id IS lifecycle.audit_event_id
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
CREATE TRIGGER memory_write_lock_anchors_insert_once BEFORE INSERT ON memory_write_lock_anchors WHEN EXISTS (SELECT 1 FROM memory_write_lock_anchors WHERE request_id=NEW.request_id OR (lock_dev=NEW.lock_dev AND lock_ino=NEW.lock_ino)) BEGIN SELECT RAISE(ABORT, 'memory write lock anchor conflicts with immutable row'); END;
CREATE TRIGGER memory_write_lock_anchors_no_update BEFORE UPDATE ON memory_write_lock_anchors BEGIN SELECT RAISE(ABORT, 'memory write lock anchors are append-only'); END;
CREATE TRIGGER memory_write_lock_anchors_no_delete BEFORE DELETE ON memory_write_lock_anchors BEGIN SELECT RAISE(ABORT, 'memory write lock anchors are append-only'); END;
CREATE TRIGGER memory_write_requests_insert_once BEFORE INSERT ON memory_write_requests WHEN EXISTS (SELECT 1 FROM memory_write_requests WHERE writer_kind=NEW.writer_kind AND request_id=NEW.request_id) BEGIN SELECT RAISE(ABORT, 'memory write request conflicts with immutable row'); END;
CREATE TRIGGER memory_write_requests_no_update BEFORE UPDATE ON memory_write_requests BEGIN SELECT RAISE(ABORT, 'memory write requests are append-only'); END;
CREATE TRIGGER memory_write_requests_no_delete BEFORE DELETE ON memory_write_requests BEGIN SELECT RAISE(ABORT, 'memory write requests are append-only'); END;
CREATE TRIGGER memory_write_results_insert_once BEFORE INSERT ON memory_write_request_results WHEN EXISTS (SELECT 1 FROM memory_write_request_results WHERE (writer_kind=NEW.writer_kind AND request_id=NEW.request_id AND result_ordinal=NEW.result_ordinal AND binding_kind=NEW.binding_kind) OR (NEW.route_ledger_id IS NOT NULL AND route_ledger_id=NEW.route_ledger_id) OR (NEW.lifecycle_ledger_id IS NOT NULL AND lifecycle_ledger_id=NEW.lifecycle_ledger_id)) BEGIN SELECT RAISE(ABORT, 'memory write result conflicts with immutable row'); END;
CREATE TRIGGER memory_write_results_no_update BEFORE UPDATE ON memory_write_request_results BEGIN SELECT RAISE(ABORT, 'memory write results are append-only'); END;
CREATE TRIGGER memory_write_results_no_delete BEFORE DELETE ON memory_write_request_results BEGIN SELECT RAISE(ABORT, 'memory write results are append-only'); END;
CREATE TRIGGER memory_write_commits_insert_once BEFORE INSERT ON memory_write_request_commits WHEN EXISTS (SELECT 1 FROM memory_write_request_commits WHERE writer_kind=NEW.writer_kind AND request_id=NEW.request_id) BEGIN SELECT RAISE(ABORT, 'memory write commit conflicts with immutable row'); END;
CREATE TRIGGER memory_write_commits_no_update BEFORE UPDATE ON memory_write_request_commits BEGIN SELECT RAISE(ABORT, 'memory write commits are append-only'); END;
CREATE TRIGGER memory_write_commits_no_delete BEFORE DELETE ON memory_write_request_commits BEGIN SELECT RAISE(ABORT, 'memory write commits are append-only'); END;
CREATE TRIGGER memory_route_ledger_insert_once BEFORE INSERT ON memory_route_ledger WHEN (NEW.id>0 AND EXISTS (SELECT 1 FROM memory_route_ledger WHERE id=NEW.id)) OR EXISTS (SELECT 1 FROM memory_route_ledger WHERE (memory_id=NEW.memory_id AND route_version=NEW.route_version) OR (NEW.previous_route_id IS NOT NULL AND previous_route_id=NEW.previous_route_id) OR (memory_id=NEW.memory_id AND source_kind=NEW.source_kind AND source_fingerprint=NEW.source_fingerprint)) BEGIN SELECT RAISE(ABORT, 'memory route ledger conflicts with immutable row'); END;
CREATE TRIGGER memory_route_ledger_no_update BEFORE UPDATE ON memory_route_ledger BEGIN SELECT RAISE(ABORT, 'memory route ledger is append-only'); END;
CREATE TRIGGER memory_route_ledger_no_delete BEFORE DELETE ON memory_route_ledger BEGIN SELECT RAISE(ABORT, 'memory route ledger is append-only'); END;
CREATE TRIGGER memory_lifecycle_ledger_insert_once BEFORE INSERT ON memory_lifecycle_ledger WHEN (NEW.id>0 AND EXISTS (SELECT 1 FROM memory_lifecycle_ledger WHERE id=NEW.id)) OR EXISTS (SELECT 1 FROM memory_lifecycle_ledger WHERE (memory_id=NEW.memory_id AND lifecycle_version=NEW.lifecycle_version) OR (NEW.previous_lifecycle_id IS NOT NULL AND previous_lifecycle_id=NEW.previous_lifecycle_id) OR (memory_id=NEW.memory_id AND source_kind=NEW.source_kind AND source_fingerprint=NEW.source_fingerprint) OR (NEW.source_operation_id IS NOT NULL AND source_operation_id=NEW.source_operation_id AND memory_id=NEW.memory_id) OR (NEW.source_api_operation_id IS NOT NULL AND source_api_operation_id=NEW.source_api_operation_id AND memory_id=NEW.memory_id)) BEGIN SELECT RAISE(ABORT, 'memory lifecycle ledger conflicts with immutable row'); END;
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
4. Create `memories_rebuild` with the exact target schema, copy all rows with
   `scope=COALESCE(NULLIF(TRIM(scope),''),'project')`, reject any result outside
   `project|global`, validate, drop old `memories`, rename, and recreate exact owned
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
maps, immutable unique lock anchors, and no schema or dependent-row/object drift.

Failure before step 3 leaves the live database unmodified; every precommit
failure rolls back that one transaction. A postcommit FK-restore/check failure
discards the connection and blocks writers. The operator restores the backup
only after proving the failed
database is closed. Once a v2 writer seals any non-migration request, rollback
means disabling the v2 projection while retaining schema/history; running 0.6.x
or restoring the old backup would lose writes and is forbidden.

## Durable Local-Copy Journal
The verified nonsymlink journal root `Q=${REMEM_DATA_DIR}/write-journal/save/`, `Q/locks/`, and `Q/quarantine/` are app-owned mode 0700 and durable before use: secure first creation of Q fsyncs Q and its parent, then first creation of each child fsyncs that child and Q. Internal request R is exactly `[A-Za-z0-9][A-Za-z0-9_-]{0,127}` (1–128 ASCII bytes), independently of the caller-key grammar. Names are retained lock `L=Q/locks/R.lock`, immutable anchor `K=(R,IL,NL)`, journal `J=Q/R.json`, ordinary phase temp `T=Q/.R.json.tmp`, cleanup transition temp `Tc=Q/.R.<stage_nonce>.cleanup.tmp`, cleanup/source read-lift marker `V=Q/.R.<stage_nonce>.read-lift.<group>.<mode:04o>`, ordered-removal capture `Xc=Q/.R.<stage_nonce>.cleanup-capture.<H|S|B|C|N>`, stage build `U=Q/.R.<nonce>.stage-build`, rollback retained-new `G=Q/quarantine/R.<stage_nonce>.retained-new`, matching-seal retained-old `O=Q/quarantine/R.<stage_nonce>.retained-old`, and target-parent siblings stage `S=.remem-save-R.stage`, backup `B=.remem-save-R.backup`, prepublication new pin `N=.remem-save-R.new-pin`, restore pin `C=.remem-save-R.restore-pin`, and hold `H=.remem-save-R.hold`. `<nonce>` and `<stage_nonce>` are the same 128-bit CSPRNG value encoded as exactly 32 lowercase ASCII hex characters; V's group is a grammar-valid source-name token resolving to its structural alias group and mode is the four-octal-digit observed original mode; one outcome may create G or O, never both. Scanner grammar reserves these exact request-qualified names and every lexical same-R Tc/V/Xc-prefix candidate across nonce/name forms, including extra-dot malformed forms while isolating distinct valid request prefixes; L/K and completed G/O are retained indefinitely, excluded from pending artifacts, and reported separately.
Resolve a canonical absolute configured local-copy root from `/`, and resolve every root-to-Q/P descendant component from directory FDs with no-follow/beneath semantics. Record and later exact-match each component `(dev,ino,uid,gid,type,mode)` chain; reject relative/noncanonical input, raw `.`/`..`, outside-root escape, any symlink/non-directory component, Q alias, wrong uid, missing owner rwx, group/world write, or missing fsync/no-replace. Securely create and durably fsync missing descendants and parent entries. Mode 0755 is valid; Q, P, target, U, G, O, S, B, N, C and H share one device. Present target additionally requires atomic exchange before target/B mutation.
Keep P open, operate on exact basenames, and record root-relative path, `IP=(dev,ino)` and `MP=(uid,gid,mode)`. Re-resolve/match before mutation and on recovery. Supported concurrency may replace/write/chmod only the user target, including an already-open target FD, through durable `cleanup_intent`; afterward target/nonpermanent pins must be quiescent until J removal and lock release, detectable drift is `local_copy_cleanup_concurrency_violation` with all remaining pins/J preserved, and post-boundary activity on them has no preservation guarantee. Q and `.remem-save-R.*` are remem-reserved. Distinguishable reserved identity/name/type/uid/gid/nlink drift fails security-visible. For an inode already exposed as target, phase-qualified same-inode mode/bytes/size/mtime/digest drift under B/S/C/H/N/G/O is accepted through the boundary; permanent G/O may keep drifting afterward because they are never removed. Active same-uid nonprotocol mutation inside private Q, including a check-to-Xc-unlink substitution, remains outside the threat model; a canonical-Q or `Q/locks` replacement observed at a mandatory validation boundary is a typed lock-unsafe abort and must mutate neither the retained old Q nor the replacement.
Re-resolve and exact-match canonical trusted-root→P to retained P at every path-dependent entry, callback, mutation, recovery step, and successful return, including inspection and V begin/finish/restore/recovery; possession of the old P dirfd never authorizes progress after its canonical binding changes.
Canonical J records format/request fingerprints, one closed phase but no goal, epoch, canonical paths plus component proofs, `before_kind`, publication/expected-seal state, semantic D1/D0, stage nonce/IU, restore identities IN/IC/IH, G/O identities, and phase observations. Before cleanup conversion, source J records the decomposed source state, `semantic_d0_digest`/`semantic_d1_digest`, and `source_namespace`, whose exact per-name fields are dev/inode/uid/gid/type/nlink; aliases are structurally equal and nlink-exact before J creation or chmod, while source J deliberately does not freeze current mode/size/mtime/digest. `cleanup_intent` first freezes the current full mutable namespace snapshot and ordered list after guarded reads. Snapshot construction and every cleanup revalidation perform two complete ordered passes over every member and every present/absent predicate: the first may run the designated callback, the second runs with no callback, and mutation/success requires both passes and the expected snapshot to agree. G/O-backed inodes remain exact for name/identity/type/owner/link but are excluded from mutable mode/content fields because their permanent link survives; J never records content, key, token, or response.
Every caller-facing direct save, including `local_copy_enabled=false`, acquires L and proves K before request lookup or intent. If L is absent, an existence-only preflight with no DB transaction must find no K, request/result/commit, J/T/Tc/V/Xc/U/G/O grammar match, or applicable S/B/N/C/H; it inspects only names/row existence. Then create/reopen L and acquire candidate lock. Before other R access, fstat/re-resolve L and use short `BEGIN IMMEDIATE`: SELECT anchors by R or candidate IL. Existing R exact-matches IL/NL; IL owned by another R fails before L write. When both absent, repeat virgin-R proof. Any old state/IL collision fails without nonce/K mutation. Only virgin direct save writes/resumes CSPRNG NL, fdatasyncs L/locks dir, plain INSERTs K, exact SELECTs and commits; insert-once makes conflict `OR REPLACE|IGNORE` invalid. Scanner/doctor require K. Mismatch, reused IL, alias, wrong type/uid/mode/nlink/content/path, or malformed L is `local_copy_lock_unsafe`. Revalidate after commit/before mutation. Phase A never mutates K or removes L.
The verified writer holds L through committed seal, cleanup/J fsync, terminal reconciliation, or visible ambiguity. Because POSIX record locks are process-scoped and closing another L FD can release the process lock, acquisition atomically reserves `(Q/locks dev,Q/locks ino,R)` in a process registry before opening L; a same-process duplicate opens/closes no L FD, and the reservation never replaces the kernel lock. The child at-fork callback closes and invalidates every inherited capability FD before resetting the registry, so an old child object cannot close a recycled FD or release a fresh child lock. Acquisition returns a mandatory R- and PID-bound `HeldRequestLock`, retaining a dup of the exact `Q/locks` FD after the caller may close its original. Inspection, source/cleanup journal prepare/load/transition, V begin/finish/recovery, snapshot proof, Xc capture/restart, ordered cleanup and J unlink have no unlocked/default API. Public request/nonce/basename/group values are exact built-in `str` and mode/ordinal values exact built-in `int`, never `bool` or subclasses, and must pass their canonical grammar/range before proof or mutation. `load_cleanup_journal(path_contract=None)` is the only compatibility exception: each Q-only bootstrap access proves held L and canonical Q/`Q/locks` immediately before and after the operation; a Q replacement is typed lock-unsafe, and any explicitly supplied path contract must validate normally. At entry, every callback and each rename/unlink/J boundary, reprove the live exclusive kernel lock plus immutable L fd/path proof, no-follow-open canonical Q and canonical-Q/retained-Q identity, then canonical `Q/locks`/retained-locks identity. Closed, fork-inherited, wrong-R, missing process reservation, replaced-L, decoy-locks or replaced-Q capability returns `local_copy_lock_unsafe` before mutation. Every safety invariant is an explicit typed branch, never an AST assertion or hand-raised AssertionError; public journal/snapshot/inspection/transition/read-lift/cleanup boundaries map external proof failures to typed errors and remain active under optimized execution. L is never unlinked. A busy direct-save contender for R retries only nonblocking L acquisition for a monotonic 5,000 ms budget, sleeping at most 5 ms and doing no unlocked R inspection; after proof it reconciles then replays an equal sealed winner or rejects a different payload without mutation. Deadline returns `local_copy_writer_in_progress`. Busy scanner/doctor attempts remain one-shot and inspect no J/U/G/O/S/B/N/C/H/target or request DB state. Crash/SIGKILL releases only the OS lock on the same retained inode, after which exactly one anchor-verified contender reconciles; a child may not reuse its parent's capability.
Every owned descriptor close moves and invalidates its owner before exactly one close attempt; after close returns the numeric FD is never probed or retried, every distinct sibling is attempted once, and capability/process-registry state is released or reset after ownership consumption. Among close-only failures the first is preserved and later failures remain diagnostic. A body/callback exception retains exact identity and outranks close failures; a mode-restoration/finish safety failure instead outranks the callback while retaining it diagnostically. A callback failure crossing public cleanup revalidation maps to `local_copy_reconciliation_ambiguous` with the original exception as cause, never boolean `False` or `local_copy_cleanup_concurrency_violation`.
Proof classes and parent locations are deliberately different:
| Path | Required source and proof |
| --- | --- |
| `L`/`K` | L is first-created/reopened only below `Q/locks`, exact current-uid regular nlink=1 mode 0600 with 32-byte NL; immutable K uniquely pins R→IL/NL; locked fd, path, row and bytes must match; neither is removed |
| `T` / `Tc` / `V` / `Xc` | ordinary journal phases alone use T with `O_CREAT\|O_EXCL\|O_NOFOLLOW`, mode 0600 and exact name/current uid/regular/nlink=1. Cleanup conversion alone uses nonce-bound Tc under the same device/gid proof: a request-wide scanner reserves every same-R nonce/name candidate; source J authorizes discarding provisional bytes only from the sole exact current-nonce Tc, while only its fresh complete exact document may replace J; stale/malformed/multiple candidates are preserved ambiguous and cleanup J rejects every same-R Tc. Before any owner-read lift, a lexical request-wide scan reserves every same-R V across nonce/name forms, including extra-dot malformed prefixes while isolating distinct valid requests; exact-match the pathname and retained write FD to the expected full raw snapshot proof, then atomically no-replace hard-link canonical J→the unique current-nonce `V=Q/.R.<stage_nonce>.read-lift.<group>.<mode:04o>` and fsync Q. Its grammar binds source group/original mode; only exact same-inode/bytes J/V nlink=2 is valid, stale/malformed/multiple candidates fail closed, and V may coexist with Tc. Capture/restart independently require exact canonical cleanup J, the complete intent field set, trusted-root/directory-handle proofs and logical path bindings, the allowlisted contract, no forbidden V/Tc/Xc coexistence and the exact next ordered member before mutation. Xc exists only after native no-replace source→Q capture for one ordered H/S/B/C/N; exactly one grammar-valid current-uid regular same-Q-device nlink≥1 candidate may restart, while malformed/multiple/stale/unsafe Xc and V+Xc or Tc+Xc are preserved ambiguous |
| `U` | after durable `stage_building`, remem creates the nonce name below private Q with T's creation proof; arbitrary bytes are owned partial build only under that exact proof |
| `S` | only atomic no-replace publication of fully fdatasynced U after durable `stage_ready`; exact IU/D1 nlink=1 until `new_pin_intent`, then S/N are the only two links |
| `B` | after durable `swap_intent`, an atomic no-replace hard-link pin accepted only when target and B both still prove `I0/M0/D0`, the same inode, and nlink=2; later B/S may be that same proved pair |
| `N/C/H/G/O` | N pins S/IU/D1 before publication; C pins post-exchange S/IC; H evacuates target; no-seal rollback renames N→G and matching-seal prior-file cleanup renames B→O across same-device P/Q. Phases record complete names/exact nlink; once an inode was target its phase-qualified mode/bytes/size/mtime/digest may drift under B/S/C/H/N/G/O through cleanup, and permanent G/O may continue drifting because neither is auto-removed |
| target | exact basename below verified P; current-uid regular identity, owner and phase-derived link set, or recorded absence. Mode/bytes/size/mtime/digest are exact before exposure, may drift phase-qualified through a target fd until `cleanup_intent`, then exact-match its snapshot while cleanup runs. Only terminal target-only state requires nlink=1; symlink, alias, extra link and nonregular types are forbidden |
Thus S is never partial. After durable `new_pin_intent`, no-replace hard-link S→N, prove S/N=IU/D1 nlink=2, fsync P, and persist `new_pinned` before publication. U→S and absent S→target are no-replace; native rename yields target/N, while portable link-first accepts `{target,S,N}=D1` nlink=3 until S unlink/fsync yields target/N nlink=2. For present target, durable `swap_intent` creates/rechecks B, then `exchange_intent` atomically exchanges S/target. Before `swapped`, prove target/N=IU/D1 and B/S are the same I0 inode with exact names/type/owner/nlink, recording current phase-qualified mutable predecessor state I0* rather than requiring original D0 after exposure; otherwise preserve. No present-target plain-rename fallback exists.
Durable `exchange_intent` accepts exact pre `(D0,D0-link,D1-link,N=D1)`, same-I0 drift, exact post `(D1,N=D1,D0-link,D0-link)`, or post `(target=IU/D1,N=IU/D1,B=I0*,S=IC*)`. Pre-exchange drift keeps incumbent and cleans only reserved D1 links. Because N predates exchange, later target replacement cannot orphan D1. If postcheck target is not IU/D1, preserve target/J/B/S/N. Stable target D1 enters no-replace restore; no recovery reverse-exchanges.
Persist `restore_intent` with S/IC, then no-replace link S→C, require target=N=IU/D1 and S=C=IC, fsync P, and persist `restore_ready`. Reserved identity drift is security ambiguity. Rename target→H no-replace; N retains D1. If H=N=D1, link C→target no-replace; otherwise link H. EEXIST wins; same-inode drift may leave target=C and newer bytes at H/N, so preserve collision. Exact target=C=D0 and H=N=D1 enters `restore_published`. Before removing D1 names, persist `quarantine_intent`, rename N→G no-replace, fsync P and Q/quarantine, and persist `quarantined`; G retains late writes. For prior absence, terminal requires target and H both absent; classify H first, treat target≠G as collision, and rename only observed target=G to H no-replace. H=G enters cleanup order `[H]`, while H=X renames back no-replace or remains with an EEXIST target. Before any final-pin cleanup persist exact-source `cleanup_intent`; every user-byte inode remains target/G/O-linked. Never unlink target.
Unreadable owner-writable targets including 0200 need no readability precondition: initial `inspect_intent` is legal only with exact target and B/S/U absent, records I0/M0 plus component proofs, adds only owner-read through no-follow FDs, double-hashes under stable identity/size/mtime, restores/fsyncs exact mode, then persists `reserved` with D0. Later cleanup first persists source J and validates its exact structural `source_namespace` aliases/nlink before chmod; current mutable fields are observed only for the cleanup snapshot. Each unreadable source or cleanup read exact-matches both pathname and retained write FD to the cleanup snapshot's full dev/ino/uid/gid/type/mode/nlink/size/mtime proof before atomically hard-linking canonical J→the mode-qualified V, fsyncing Q and chmod; only exact same-inode/bytes J/V nlink=2 authorizes original/single-read-bit mode. On the snapshot-construction/cleanup-revalidation snapshot-proof path, immediately before restoration, a fresh fstat/re-resolution must prove the retained writer still has exactly the lifted mode `(encoded original mode | owner-read)` plus the recorded dev/ino/uid/gid/type/nlink; any third mode is drift, is never overwritten, and leaves V armed. Recovery finds any surviving exact structural alias even if target was replaced or content drifted, restores and fsyncs the encoded mode plus all relevant parents, retains V on fchmod/fsync failure, removes V/fsyncs Q only after success, then performs ordinary validation. V may coexist with Tc; without V, even the single-bit mode difference is drift.
While the mandatory capability proves held L and canonical Q/`Q/locks`, each ordinary phase update writes/fdatasyncs T, renames over J and fsyncs Q; neither Tc, V nor Xc substitutes for T. With J absent and active Tc/V/Xc/U/S/B/N/C/H absent, scanner may remove proved T; grammar-valid completed G/O is retained/reported, not pending. Any other J-absent active artifact or failed proof is ambiguous.
After `reserved`, persist `stage_building` with nonce/D1 and `IU=NULL`, then O_EXCL-create U, fstat it and persist the same phase with IU before the first content byte; write injected chunks, fdatasync and double-check IU/D1, then persist `stage_ready`. A crash in the create→IU-fsync gap still owns U only through its exact nonce/private-Q/type/uid/mode/nlink proof. `stage_building` accepts U absent or any empty/partial/full bytes under that proof and no S; `stage_ready` accepts full U, full S with IU after no-replace U→S, or portable same-inode U+S/nlink=2. Existing no-seal recovery unlinks only proved U/S and fsyncs Q/P; wrong proof or wrong-byte S is ambiguous. Persist `staged` only after durable S=D1/U-absent.
J.phase is a closed persisted enum: `inspect_intent,reserved,stage_building,stage_ready,staged,new_pin_intent,new_pinned,swap_intent,backed_up,exchange_intent,swapped,sealed,recover_before_file,recover_before_absent,recover_after_file,recover_after_absent,restore_intent,restore_ready,restore_published,quarantine_intent,quarantined,predecessor_quarantine_intent,predecessor_quarantined,cleanup_intent,collision_preserved`; J contains no goal and only the DB seal proves commit. Phases through `swapped` require no seal except that `swapped`+matching seal is the sole valid COMMIT-before-J-`sealed` tuple; `sealed`/predecessor-quarantine/recover-after require a matching seal, recover-before/restore/rollback-quarantine/collision require no seal, and cleanup inherits one exact source tuple. Every other phase×seal combination and every unknown goal-like field fail closed. In `(target,B,S,N,U)`, staged is `(D0,Ø,D1,Ø,Ø)`; pin intent accepts N absent or S/N; new-pinned is `(D0,Ø,D1,D1,Ø)`. Absent publication reaches `(D1,Ø,Ø,D1,Ø)` natively or via `(D1,Ø,D1,D1,Ø)` nlink3.
Every exchange is postvalidated. Through durable `cleanup_intent`, supported target replace/open-FD bytes stay at target, captured S/H, pre-pinned N, or retained G/O. Before seal, reverify target/N exact D1 and B/S as the same structural I0* pair; phase-qualified predecessor mode/content drift never blocks seal because matching-seal recovery first persists `predecessor_quarantine_intent`, atomically renames B→O no-replace, fsyncs P and Q/quarantine, persists `predecessor_quarantined`, and never removes O. Prior absence creates no O. A sealed exact replay creates no journal; a fresh attempt after any completed outcome uses a new stage nonce.
Before recovery validate tuple, fsync one recovery phase, then mutate. Ø is absent. Terminal D0 requires M0/nlink1; intermediate nlink equals exact known names. During cleanup, each runtime expected nlink is recomputed from every still-named snapshot alias sharing the inode—including permanent G/O—rather than reusing the initial snapshot count. Restore rows use `(target,B,S,N,C,H,G,O,U)` and X for proved unexpected entry:
| Recovery phase | DB seal state | Accepted physical states | Idempotent normalization |
| --- | --- | --- | --- |
| `recover_before_file` | no seal, prior file | early U/S states may lack N; every `new_pinned` or later state has N=IU/D1. Pre-exchange target is D0/I0*; post-exchange is target/N=D1 with B/S exact or drift | remove unexposed U/S/N directly; removing B requires target=B eligibility then `cleanup_intent`; post-exchange target D1 persists restore; replaced target stays and N preserves D1 |
| `restore_intent` | no seal, prior file | `(D1,I0*,IC*,D1,Ø,Ø,Ø,Ø,Ø)` then C=IC prefix | create/postvalidate C, require target/N D1 and S/C IC, fsync, persist ready; distinguishable reserved identity/name/type/owner/link drift is security ambiguity |
| `restore_ready` | no seal, prior file | pre-evacuation; target absent with H; target=C; target=H; EEXIST X; post-choice H/N drift, always N=D1* and G/O absent | evacuate target; link C only for observed H=N=D1, else H; EEXIST wins; exact D0 advances, drift preserves |
| `restore_published` | no seal, uncontested prior file | D0 names `{target,B,S,C}`=4 and D1 `{H,N}`=2; G/O absent | persist `quarantine_intent`; never unlink last D1 pin |
| `quarantine_intent` | no seal, D1 reached target | same tuple with exactly one of N or G after atomic N→G; absent rollback analog has target/N or target/G D1; O absent | verify IU identity, rename N→G no-replace, fsync P and Q/quarantine, persist `quarantined`; both namespace sides are restartable |
| `quarantined` | no seal, rollback | present: D0 `{target,B,S,C}`, D1 `{H,G}`, O absent. Prior absent: absence/G; target=G; target=X≠G; target absent or Y with H=G/X; restored target=X; all with G present/O absent | present enters cleanup source `[H,S,B,C]`. Prior absent classifies H first and is terminal only with target/H absent; target≠G stays, only target=G evacuates, H=G enters cleanup source `[H]`, and H=X restores no-replace or becomes collision |
| `collision_preserved` | no seal, collision | any proved restore terminal; target plus N/H/C/B/S/G/O retain every supported-concurrency inode | no cleanup/seal; report byte locations; distinguishable reserved identity/name/type/owner/link drift is security ambiguity |
| `recover_before_absent` | no seal, prior absent | early U/S; new-pinned S/N; portable target/S/N nlink3; published target/N; quarantined target/G; target≠G collision; all H evacuation/restoration prefixes; terminal target/H absence plus G; O absent | before publication remove U/S/N; after publication quarantine N→G, leave target≠G, classify H, and evacuate only target=G as above. Never unlink target; EEXIST preserves every entry |
| `recover_after_file` / `predecessor_quarantine_intent` / `predecessor_quarantined` | matching seal, prior file | start target/N=D1 and same-I0* B/S with G/O absent; intent accepts exactly one B or O plus S=I0*; quarantined is target/N=D1 and same-I0* S/O, with mutable predecessor drift allowed throughout | verify exact structural pairs; persist intent; rename B→O no-replace, fsync P and Q/quarantine, persist quarantined; O is permanent, then enter cleanup source `[S,N]` |
| `recover_after_absent` | matching seal, prior absent | target/N=D1; G/O absent | verify target=N=D1 and enter cleanup source `[N]`; create no O |
| `cleanup_intent` | exact source tuple | only `(recover_before_file,file,preexchange,no-seal)→[B]`, `(quarantined,file,no-seal)→[H,S,B,C]`, `(predecessor_quarantined,file,matching-seal)→[S,N]`, `(recover_after_absent,absent,matching-seal)→[N]`, or `(quarantined,absent,no-seal)→[H]`; each has exact `source_namespace` structural dev/ino/uid/gid/type/nlink facts, component paths and target/G/O retention | after request-wide scans, source J accepts Tc absent or the sole exact current-nonce candidate whose provisional bytes may be discarded/Q-fsynced; only its fresh complete exact document replaces J, while stale/malformed/multiple Tc candidates are preserved. The sole exact current-nonce grammar-valid mode-qualified V may coexist with Tc and must restore/fsync the basename-encoded original mode through a surviving structural alias before Tc classification. Canonical cleanup J requires every same-R Tc absent and accepts V absent or one exact-current same-inode J/V nlink=2 marker. Public capture/restart first exact-match the complete canonical intent, trusted-root/retained-handle and logical path binding, then require the Xc member to be next in order—prior entries absent, later entries present. Before each H/S/B/C/N removal, retain its exact reader/proof, native-no-replace rename source→Xc, fsync Q then source parent, prove source absent and Xc+retained FD exact, then unlink Xc/Q-fsync. Mismatch restores Xc→source no-replace, fsyncs source parent then Q, and errors; EEXIST retains Xc. Immediately before J unlink, full intent/J proof and request-wide Tc/V/Xc absence are rechecked; a late stale candidate preserves itself and J. Restart restores a sole valid Xc before removed-prefix derivation; cleanup J+Xc is recoverable, while source J+Xc, Tc+Xc, V+Xc, Xc at transition/J unlink, and malformed/multiple/unsafe candidates are ambiguous. Restore failure retains V/Xc; unmarked mode drift is never restored. Snapshot drift returns typed `local_copy_cleanup_concurrency_violation`; malformed journal/marker/capture/path proof returns typed `local_copy_reconciliation_ambiguous`; both preserve J/pins and set `doctor_healthy=false` |
Each listed phase remains through its action and required parent fsync; N→G and B→O each need P and Q/quarantine, while cleanup needs its durable snapshot and for every entry the source→Xc rename, Q fsync, source-parent fsync, postcapture proof, Xc unlink and Q fsync. A crash after capture restores Xc no-replace and durably fsyncs source parent then Q before revalidation. Every enumerated adjacent namespace is accepted. Normal rollback removes J only with G durable; matching-seal prior-file cleanup only with O durable; collision or cleanup mismatch retains J/pins/Xc. Rehearsal expands every syscall prefix.
Every unlisted tuple/phase, earlier-phase seal, missing/mismatched DB result, wrong identity/name/type/owner/link/path, forbidden pre-exposure mode/content drift, escape, alias, or unproved artifact is `local_copy_reconciliation_ambiguous`: preserve all bytes/J, return only opaque
`R`/phase and `doctor_healthy=false`. Detectable cleanup-snapshot drift returns `local_copy_cleanup_concurrency_violation` with the same nonhealthy state. Phase-qualified same-inode mode/content drift after target exposure is accepted through durable cleanup; permanent G/O may drift afterward without affecting cleanup proof. Uncontested no-seal after D1 publication
converges to exact D0/absence plus retained G; an earlier abort has no G. Collision preserves supported-concurrency bytes under exact names; a prior-file seal converges to target D1 plus retained-old O, while prior absence converges to target D1 without O.
## Completion Evidence
Rehearsal must match SQL and prove UDFs, typed/sealed writers, retry/duplicate, same-process/fork/cross-process locking, lexical artifact scans, full direct-mutator path binding, crash boundaries, explicit optimized-execution safety gates, and each contract file ≤800 lines.
