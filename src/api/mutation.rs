use std::fmt;

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

pub(crate) const MUTATION_RESPONSE_SCHEMA_VERSION: i64 = 1;
const IDEMPOTENCY_KEY_MAX_BYTES: usize = 128;
const OPERATION_NAMESPACE: &[u8] = b"remem-web-operation-v1\0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MutationIdentity {
    pub idempotency_key_hash: String,
    pub operation_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IdempotencyKeyError {
    Invalid,
}

impl fmt::Display for IdempotencyKeyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("idempotency_key_invalid")
    }
}

impl std::error::Error for IdempotencyKeyError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredMutation {
    pub operation_id: String,
    pub response_schema_version: i64,
    pub response_json: String,
    pub audit_id: i64,
    pub created_at_epoch: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MutationLookup {
    Miss,
    Replay(StoredMutation),
    Conflict,
    UnsupportedSchema(i64),
}

pub(crate) struct NewMutationRecord<'a> {
    pub identity: &'a MutationIdentity,
    pub request_hash: &'a str,
    pub resource_kind: &'a str,
    pub resource_id: i64,
    pub action: &'a str,
    pub response_json: &'a str,
    pub audit_id: i64,
    pub created_at_epoch: i64,
}

/// Marker for normalized business payloads that intentionally exclude transport
/// credentials and the raw idempotency key.
///
/// Safe endpoint modules must define a dedicated hash DTO and opt it into this
/// trait. Wire request DTOs must never implement it.
pub(crate) trait CredentialFreeMutationBody: Serialize {}

pub(crate) fn validate_idempotency_key(
    raw_key: &str,
) -> std::result::Result<MutationIdentity, IdempotencyKeyError> {
    let normalized = raw_key.trim();
    if normalized.is_empty()
        || normalized.len() > IDEMPOTENCY_KEY_MAX_BYTES
        || !normalized.is_ascii()
        || !normalized.bytes().all(is_allowed_idempotency_byte)
    {
        return Err(IdempotencyKeyError::Invalid);
    }

    let idempotency_key_hash = sha256_digest(normalized.as_bytes());
    let mut operation_input = Vec::with_capacity(OPERATION_NAMESPACE.len() + normalized.len());
    operation_input.extend_from_slice(OPERATION_NAMESPACE);
    operation_input.extend_from_slice(normalized.as_bytes());
    let operation_id = format!("op_{}", sha256_digest(&operation_input));
    Ok(MutationIdentity {
        idempotency_key_hash,
        operation_id,
    })
}

fn is_allowed_idempotency_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'~' | b'-')
}

pub(crate) fn mutation_request_hash<T: CredentialFreeMutationBody>(
    resource_kind: &str,
    resource_id: i64,
    action: &str,
    body: &T,
) -> Result<String> {
    anyhow::ensure!(!resource_kind.is_empty(), "resource kind must not be empty");
    anyhow::ensure!(resource_id > 0, "resource id must be positive");
    anyhow::ensure!(!action.is_empty(), "mutation action must not be empty");
    let value = serde_json::json!({
        "action": action,
        "body": serde_json::to_value(body).context("serialize mutation request body")?,
        "resource_id": resource_id,
        "resource_kind": resource_kind,
    });
    Ok(sha256_digest(&canonical_json_bytes(&value)?))
}

pub(crate) fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    write_canonical_json(value, &mut output)?;
    Ok(output)
}

fn write_canonical_json(value: &Value, output: &mut Vec<u8>) -> Result<()> {
    match value {
        Value::Null => output.extend_from_slice(b"null"),
        Value::Bool(boolean) => output.extend_from_slice(boolean.to_string().as_bytes()),
        Value::Number(number) => output.extend_from_slice(number.to_string().as_bytes()),
        Value::String(string) => {
            output.extend_from_slice(serde_json::to_string(string)?.as_bytes());
        }
        Value::Array(items) => {
            output.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                write_canonical_json(item, output)?;
            }
            output.push(b']');
        }
        Value::Object(object) => {
            output.push(b'{');
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                output.extend_from_slice(serde_json::to_string(key)?.as_bytes());
                output.push(b':');
                write_canonical_json(&object[key], output)?;
            }
            output.push(b'}');
        }
    }
    Ok(())
}

pub(crate) fn lookup_mutation(
    conn: &Connection,
    identity: &MutationIdentity,
    request_hash: &str,
) -> Result<MutationLookup> {
    let stored = conn
        .query_row(
            "SELECT request_hash, operation_id, response_schema_version,
                    response_json, audit_id, created_at_epoch
             FROM api_mutation_requests
             WHERE idempotency_key_hash = ?1",
            params![identity.idempotency_key_hash],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    StoredMutation {
                        operation_id: row.get(1)?,
                        response_schema_version: row.get(2)?,
                        response_json: row.get(3)?,
                        audit_id: row.get(4)?,
                        created_at_epoch: row.get(5)?,
                    },
                ))
            },
        )
        .optional()
        .context("read API mutation replay ledger")?;
    let Some((stored_hash, stored)) = stored else {
        return Ok(MutationLookup::Miss);
    };
    if stored_hash != request_hash {
        return Ok(MutationLookup::Conflict);
    }
    if stored.response_schema_version != MUTATION_RESPONSE_SCHEMA_VERSION {
        return Ok(MutationLookup::UnsupportedSchema(
            stored.response_schema_version,
        ));
    }
    Ok(MutationLookup::Replay(stored))
}

pub(crate) fn insert_mutation(conn: &Connection, record: &NewMutationRecord<'_>) -> Result<()> {
    conn.execute(
        "INSERT INTO api_mutation_requests(
             idempotency_key_hash, request_hash, operation_id, resource_kind,
             resource_id, action, response_schema_version, response_json,
             audit_id, created_at_epoch
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            record.identity.idempotency_key_hash,
            record.request_hash,
            record.identity.operation_id,
            record.resource_kind,
            record.resource_id,
            record.action,
            MUTATION_RESPONSE_SCHEMA_VERSION,
            record.response_json,
            record.audit_id,
            record.created_at_epoch,
        ],
    )
    .context("insert API mutation replay ledger")?;
    Ok(())
}

fn sha256_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use rusqlite::Connection;
    use serde::Serialize;
    use serde_json::{json, Value};

    use super::*;

    #[derive(Serialize)]
    struct CanonicalBody(Value);

    impl CredentialFreeMutationBody for CanonicalBody {}

    #[derive(Serialize)]
    struct EmptyBody {}

    impl CredentialFreeMutationBody for EmptyBody {}

    struct WireRequest<'a> {
        bearer_token: &'a str,
        idempotency_key: &'a str,
        reason: &'a str,
        expected_version: i64,
    }

    #[derive(Serialize)]
    struct MemoryGovernanceHashBody<'a> {
        reason: &'a str,
        expected_version: i64,
    }

    impl CredentialFreeMutationBody for MemoryGovernanceHashBody<'_> {}

    impl WireRequest<'_> {
        fn credential_free_body(&self) -> MemoryGovernanceHashBody<'_> {
            let _transport_credentials = (self.bearer_token, self.idempotency_key);
            MemoryGovernanceHashBody {
                reason: self.reason,
                expected_version: self.expected_version,
            }
        }
    }

    #[test]
    fn invalid_idempotency_keys_fail_before_identity_creation() {
        for key in ["", "   ", "snowman-☃", "bad key", "line\nbreak", "x\0y"] {
            assert_eq!(
                validate_idempotency_key(key),
                Err(IdempotencyKeyError::Invalid)
            );
        }
        assert_eq!(
            validate_idempotency_key(&"a".repeat(129)),
            Err(IdempotencyKeyError::Invalid)
        );
    }

    #[test]
    fn valid_key_is_trimmed_then_replaced_by_irreversible_identifiers() -> Result<()> {
        let identity = validate_idempotency_key("  01J.TEST_key-~  ")?;
        assert_eq!(identity.idempotency_key_hash.len(), 64);
        assert_eq!(identity.operation_id.len(), 67);
        assert!(!identity.idempotency_key_hash.contains("TEST_key"));
        assert!(!identity.operation_id.contains("TEST_key"));
        assert_eq!(identity, validate_idempotency_key("01J.TEST_key-~")?);
        Ok(())
    }

    #[test]
    fn request_hash_uses_canonical_object_order() -> Result<()> {
        let first = CanonicalBody(json!({"z": 1, "nested": {"b": true, "a": false}}));
        let second = CanonicalBody(json!({"nested": {"a": false, "b": true}, "z": 1}));
        assert_eq!(
            mutation_request_hash("memory", 7, "archive", &first)?,
            mutation_request_hash("memory", 7, "archive", &second)?
        );
        Ok(())
    }

    #[test]
    fn request_hash_excludes_transport_credentials_and_idempotency_key() -> Result<()> {
        let first = WireRequest {
            bearer_token: "bearer-secret-one",
            idempotency_key: "idempotency-secret-one",
            reason: "duplicate cleanup",
            expected_version: 7,
        };
        let second = WireRequest {
            bearer_token: "bearer-secret-two",
            idempotency_key: "idempotency-secret-two",
            reason: "duplicate cleanup",
            expected_version: 7,
        };

        assert_eq!(
            mutation_request_hash("memory", 7, "archive", &first.credential_free_body())?,
            mutation_request_hash("memory", 7, "archive", &second.credential_free_body())?
        );
        Ok(())
    }

    #[test]
    fn ledger_distinguishes_replay_conflict_and_unknown_schema() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        let identity = validate_idempotency_key("01J-ledger-test")?;
        let request_hash = mutation_request_hash("candidate", 3, "approve", &EmptyBody {})?;
        assert_eq!(
            lookup_mutation(&conn, &identity, &request_hash)?,
            MutationLookup::Miss
        );
        insert_mutation(
            &conn,
            &NewMutationRecord {
                identity: &identity,
                request_hash: &request_hash,
                resource_kind: "candidate",
                resource_id: 3,
                action: "approve",
                response_json: r#"{"operation_id":"safe"}"#,
                audit_id: 9,
                created_at_epoch: 10,
            },
        )?;
        assert!(matches!(
            lookup_mutation(&conn, &identity, &request_hash)?,
            MutationLookup::Replay(_)
        ));
        assert_eq!(
            lookup_mutation(&conn, &identity, "different")?,
            MutationLookup::Conflict
        );
        conn.execute(
            "UPDATE api_mutation_requests SET response_schema_version = 99",
            [],
        )?;
        assert_eq!(
            lookup_mutation(&conn, &identity, &request_hash)?,
            MutationLookup::UnsupportedSchema(99)
        );
        let persisted = conn.query_row(
            "SELECT idempotency_key_hash || response_json FROM api_mutation_requests",
            [],
            |row| row.get::<_, String>(0),
        )?;
        assert!(!persisted.contains("01J-ledger-test"));
        assert!(persisted.contains(&identity.idempotency_key_hash));
        Ok(())
    }
}
