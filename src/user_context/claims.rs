use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};

pub const DEFAULT_USER_KEY: &str = "user:default";
pub const DEFAULT_OWNER_SCOPE: &str = "user";
pub const DEFAULT_OWNER_KEY: &str = "user:default";

const DEFAULT_SOURCE_KIND: &str = "manual";
const DEFAULT_SOURCE_REFS_JSON: &str = r#"[{"kind":"manual_cli","command":"user remember"}]"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserContextClaimType {
    Identity,
    Role,
    Preference,
    Skill,
    Goal,
    Project,
    Relationship,
    Constraint,
    Activity,
}

impl UserContextClaimType {
    pub fn db_value(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Role => "role",
            Self::Preference => "preference",
            Self::Skill => "skill",
            Self::Goal => "goal",
            Self::Project => "project",
            Self::Relationship => "relationship",
            Self::Constraint => "constraint",
            Self::Activity => "activity",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserContextSensitivity {
    Normal,
    Personal,
    Sensitive,
    Restricted,
}

impl UserContextSensitivity {
    pub fn db_value(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Personal => "personal",
            Self::Sensitive => "sensitive",
            Self::Restricted => "restricted",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct UserContextClaim {
    pub id: i64,
    pub user_key: String,
    pub owner_scope: String,
    pub owner_key: String,
    pub claim_type: String,
    pub claim_key: String,
    pub claim_text: String,
    pub confidence: f64,
    pub sensitivity: String,
    pub source_kind: String,
    pub source_refs_json: String,
    pub status: String,
    pub valid_from_epoch: Option<i64>,
    pub valid_to_epoch: Option<i64>,
    pub last_confirmed_at_epoch: Option<i64>,
    pub supersedes_claim_id: Option<i64>,
    pub created_at_epoch: i64,
    pub updated_at_epoch: i64,
}

#[derive(Debug, Clone)]
pub struct ManualClaimRequest<'a> {
    pub text: &'a str,
    pub owner_scope: Option<&'a str>,
    pub owner_key: Option<&'a str>,
    pub claim_type: UserContextClaimType,
    pub claim_key: Option<&'a str>,
    pub confidence: f64,
    pub sensitivity: UserContextSensitivity,
    pub valid_from_epoch: Option<i64>,
    pub valid_to_epoch: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ClaimListRequest<'a> {
    pub owner_scope: Option<&'a str>,
    pub owner_key: Option<&'a str>,
    pub include_inactive: bool,
    pub limit: i64,
}

#[derive(Debug, Clone)]
pub struct ClaimEditRequest<'a> {
    pub text: &'a str,
    pub claim_type: Option<UserContextClaimType>,
    pub claim_key: Option<&'a str>,
    pub sensitivity: Option<UserContextSensitivity>,
    pub valid_from_epoch: Option<i64>,
    pub valid_to_epoch: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaimEditResult {
    pub previous: UserContextClaim,
    pub current: UserContextClaim,
}

pub fn create_manual_claim(
    conn: &Connection,
    req: &ManualClaimRequest<'_>,
) -> Result<UserContextClaim> {
    let text = normalize_required("claim text", req.text)?;
    validate_validity(req.valid_from_epoch, req.valid_to_epoch)?;
    let (owner_scope, owner_key) = normalized_owner(req.owner_scope, req.owner_key)?;
    let claim_type = req.claim_type.db_value();
    validate_confidence(req.confidence)?;
    let claim_key = normalized_claim_key(req.claim_key, claim_type, text);
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO user_context_claims
         (user_key, owner_scope, owner_key, claim_type, claim_key, claim_text,
          confidence, sensitivity, source_kind, source_refs_json, status,
          valid_from_epoch, valid_to_epoch, last_confirmed_at_epoch,
          supersedes_claim_id, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'active',
                 ?11, ?12, ?13, NULL, ?14, ?14)",
        params![
            DEFAULT_USER_KEY,
            owner_scope,
            owner_key,
            claim_type,
            claim_key,
            text,
            req.confidence,
            req.sensitivity.db_value(),
            DEFAULT_SOURCE_KIND,
            DEFAULT_SOURCE_REFS_JSON,
            req.valid_from_epoch,
            req.valid_to_epoch,
            now,
            now,
        ],
    )
    .context("insert manual user-context claim")?;
    load_claim(conn, conn.last_insert_rowid())
}

pub fn list_claims(conn: &Connection, req: &ClaimListRequest<'_>) -> Result<Vec<UserContextClaim>> {
    let mut conditions = Vec::new();
    let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut idx = 1;

    if !req.include_inactive {
        conditions.push("status = ?1".to_string());
        values.push(Box::new("active".to_string()));
        conditions.push("sensitivity <> ?2".to_string());
        values.push(Box::new("restricted".to_string()));
        let now = chrono::Utc::now().timestamp();
        conditions.push("(valid_from_epoch IS NULL OR valid_from_epoch <= ?3)".to_string());
        values.push(Box::new(now));
        conditions.push("(valid_to_epoch IS NULL OR valid_to_epoch > ?4)".to_string());
        values.push(Box::new(now));
        idx = 5;
    }

    if let Some(owner_scope) = normalized_optional(req.owner_scope) {
        validate_owner_scope(owner_scope)?;
        conditions.push(format!("owner_scope = ?{idx}"));
        values.push(Box::new(owner_scope.to_string()));
        idx += 1;
    }

    if let Some(owner_key) = normalized_optional(req.owner_key) {
        conditions.push(format!("owner_key = ?{idx}"));
        values.push(Box::new(owner_key.to_string()));
        idx += 1;
    }

    let mut sql = "SELECT id, user_key, owner_scope, owner_key, claim_type, claim_key,
                          claim_text, confidence, sensitivity, source_kind,
                          source_refs_json, status, valid_from_epoch,
                          valid_to_epoch, last_confirmed_at_epoch,
                          supersedes_claim_id, created_at_epoch, updated_at_epoch
                   FROM user_context_claims"
        .to_string();
    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }
    sql.push_str(&format!(
        " ORDER BY updated_at_epoch DESC, id DESC LIMIT ?{idx}"
    ));
    values.push(Box::new(req.limit.clamp(1, 500)));

    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&values);
    let rows = stmt.query_map(refs.as_slice(), claim_from_row)?;
    crate::db::query::collect_rows(rows)
}

pub fn load_claim(conn: &Connection, id: i64) -> Result<UserContextClaim> {
    conn.query_row(
        "SELECT id, user_key, owner_scope, owner_key, claim_type, claim_key,
                claim_text, confidence, sensitivity, source_kind,
                source_refs_json, status, valid_from_epoch, valid_to_epoch,
                last_confirmed_at_epoch, supersedes_claim_id,
                created_at_epoch, updated_at_epoch
         FROM user_context_claims
         WHERE id = ?1",
        [id],
        claim_from_row,
    )
    .optional()?
    .ok_or_else(|| anyhow!("user-context claim {id} not found"))
}

pub fn edit_claim(
    conn: &Connection,
    id: i64,
    req: &ClaimEditRequest<'_>,
) -> Result<ClaimEditResult> {
    let text = normalize_required("claim text", req.text)?;
    validate_validity(req.valid_from_epoch, req.valid_to_epoch)?;
    let tx = conn.unchecked_transaction()?;
    let previous = load_claim(&tx, id)?;
    if previous.status != "active" {
        bail!(
            "only active user-context claims can be edited; claim {id} is {}",
            previous.status
        );
    }
    let now = chrono::Utc::now().timestamp();
    let claim_type = req
        .claim_type
        .map(UserContextClaimType::db_value)
        .unwrap_or(previous.claim_type.as_str());
    let claim_key = normalized_claim_key(req.claim_key, claim_type, text);
    let sensitivity = req
        .sensitivity
        .map(UserContextSensitivity::db_value)
        .unwrap_or(previous.sensitivity.as_str());
    let valid_from = req.valid_from_epoch.or(previous.valid_from_epoch);
    let valid_to = req.valid_to_epoch.or(previous.valid_to_epoch);
    validate_validity(valid_from, valid_to)?;

    tx.execute(
        "UPDATE user_context_claims
         SET status = 'superseded', updated_at_epoch = ?1
         WHERE id = ?2",
        params![now, id],
    )?;
    tx.execute(
        "INSERT INTO user_context_claims
         (user_key, owner_scope, owner_key, claim_type, claim_key, claim_text,
          confidence, sensitivity, source_kind, source_refs_json, status,
          valid_from_epoch, valid_to_epoch, last_confirmed_at_epoch,
          supersedes_claim_id, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'active',
                 ?11, ?12, ?13, ?14, ?15, ?15)",
        params![
            previous.user_key,
            previous.owner_scope,
            previous.owner_key,
            claim_type,
            claim_key,
            text,
            previous.confidence,
            sensitivity,
            previous.source_kind,
            previous.source_refs_json,
            valid_from,
            valid_to,
            now,
            previous.id,
            now,
        ],
    )?;
    let current = load_claim(&tx, tx.last_insert_rowid())?;
    tx.commit()?;
    Ok(ClaimEditResult { previous, current })
}

pub fn suppress_claim(conn: &Connection, id: i64) -> Result<UserContextClaim> {
    let claim = load_claim(conn, id)?;
    if claim.status == "suppressed" {
        return Ok(claim);
    }
    if claim.status != "active" {
        bail!(
            "only active user-context claims can be suppressed; claim {id} is {}",
            claim.status
        );
    }
    transition_status(conn, id, "suppressed")
}

pub fn unsuppress_claim(conn: &Connection, id: i64) -> Result<UserContextClaim> {
    let claim = load_claim(conn, id)?;
    if claim.status != "suppressed" {
        bail!(
            "only suppressed user-context claims can be unsuppressed; claim {id} is {}",
            claim.status
        );
    }
    transition_status(conn, id, "active")
}

pub fn delete_claim(conn: &Connection, id: i64) -> Result<UserContextClaim> {
    transition_status(conn, id, "deleted")
}

fn transition_status(conn: &Connection, id: i64, status: &str) -> Result<UserContextClaim> {
    let updated = conn.execute(
        "UPDATE user_context_claims
         SET status = ?1, updated_at_epoch = ?2
         WHERE id = ?3",
        params![status, chrono::Utc::now().timestamp(), id],
    )?;
    if updated != 1 {
        bail!("failed to update user-context claim {id}");
    }
    load_claim(conn, id)
}

fn claim_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<UserContextClaim> {
    Ok(UserContextClaim {
        id: row.get(0)?,
        user_key: row.get(1)?,
        owner_scope: row.get(2)?,
        owner_key: row.get(3)?,
        claim_type: row.get(4)?,
        claim_key: row.get(5)?,
        claim_text: row.get(6)?,
        confidence: row.get(7)?,
        sensitivity: row.get(8)?,
        source_kind: row.get(9)?,
        source_refs_json: row.get(10)?,
        status: row.get(11)?,
        valid_from_epoch: row.get(12)?,
        valid_to_epoch: row.get(13)?,
        last_confirmed_at_epoch: row.get(14)?,
        supersedes_claim_id: row.get(15)?,
        created_at_epoch: row.get(16)?,
        updated_at_epoch: row.get(17)?,
    })
}

fn normalized_owner<'a>(
    owner_scope: Option<&'a str>,
    owner_key: Option<&'a str>,
) -> Result<(&'a str, &'a str)> {
    let owner_scope = normalized_optional(owner_scope).unwrap_or(DEFAULT_OWNER_SCOPE);
    validate_owner_scope(owner_scope)?;
    let owner_key = normalized_optional(owner_key);
    match (owner_scope, owner_key) {
        ("user", None) => Ok((owner_scope, DEFAULT_OWNER_KEY)),
        ("user", Some(owner_key)) => Ok((owner_scope, owner_key)),
        (_, Some(owner_key)) => Ok((owner_scope, owner_key)),
        _ => bail!("--owner-key is required when --scope is not user"),
    }
}

fn validate_owner_scope(owner_scope: &str) -> Result<()> {
    if matches!(owner_scope, "user" | "workspace" | "repo" | "session") {
        return Ok(());
    }
    bail!("unsupported user-context owner scope: {owner_scope}");
}

fn normalized_claim_key(provided: Option<&str>, claim_type: &str, text: &str) -> String {
    if let Some(value) = normalized_optional(provided) {
        return value.to_string();
    }
    let mut hasher = Sha256::new();
    hasher.update(claim_type.as_bytes());
    hasher.update(b"\0");
    hasher.update(text.as_bytes());
    let digest = hasher.finalize();
    format!(
        "{claim_type}:manual:{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7]
    )
}

fn normalize_required<'a>(label: &str, value: &'a str) -> Result<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        bail!("{label} cannot be empty");
    }
    Ok(value)
}

fn normalized_optional(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn validate_confidence(confidence: f64) -> Result<()> {
    if (0.0..=1.0).contains(&confidence) {
        return Ok(());
    }
    bail!("confidence must be between 0.0 and 1.0");
}

fn validate_validity(valid_from_epoch: Option<i64>, valid_to_epoch: Option<i64>) -> Result<()> {
    if let (Some(from), Some(to)) = (valid_from_epoch, valid_to_epoch) {
        if to <= from {
            bail!("valid_to_epoch must be greater than valid_from_epoch");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn migrated_conn() -> Result<Connection> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        Ok(conn)
    }

    #[test]
    fn manual_claim_defaults_to_active_user_scope_with_source_metadata() -> Result<()> {
        let conn = migrated_conn()?;
        let claim = create_manual_claim(
            &conn,
            &ManualClaimRequest {
                text: "Prefer concise architecture review",
                owner_scope: None,
                owner_key: None,
                claim_type: UserContextClaimType::Preference,
                claim_key: None,
                confidence: 1.0,
                sensitivity: UserContextSensitivity::Normal,
                valid_from_epoch: None,
                valid_to_epoch: None,
            },
        )?;

        assert_eq!(claim.user_key, DEFAULT_USER_KEY);
        assert_eq!(claim.owner_scope, DEFAULT_OWNER_SCOPE);
        assert_eq!(claim.owner_key, DEFAULT_OWNER_KEY);
        assert_eq!(claim.source_kind, "manual");
        assert_eq!(claim.status, "active");
        assert!(claim.source_refs_json.contains("manual_cli"));
        assert!(claim.claim_key.starts_with("preference:manual:"));
        Ok(())
    }

    #[test]
    fn non_user_scope_requires_owner_key() -> Result<()> {
        let conn = migrated_conn()?;
        let err = create_manual_claim(
            &conn,
            &ManualClaimRequest {
                text: "Repo scoped preference",
                owner_scope: Some("repo"),
                owner_key: None,
                claim_type: UserContextClaimType::Preference,
                claim_key: None,
                confidence: 1.0,
                sensitivity: UserContextSensitivity::Normal,
                valid_from_epoch: None,
                valid_to_epoch: None,
            },
        )
        .expect_err("repo scope requires owner-key");
        assert!(err.to_string().contains("--owner-key is required"));
        Ok(())
    }

    #[test]
    fn default_list_excludes_suppressed_deleted_expired_future_and_restricted() -> Result<()> {
        let conn = migrated_conn()?;
        let active =
            create_manual_claim(&conn, &request("active", UserContextSensitivity::Normal))?;
        let suppressed = create_manual_claim(
            &conn,
            &request("suppressed", UserContextSensitivity::Normal),
        )?;
        suppress_claim(&conn, suppressed.id)?;
        let deleted =
            create_manual_claim(&conn, &request("deleted", UserContextSensitivity::Normal))?;
        delete_claim(&conn, deleted.id)?;
        create_manual_claim(
            &conn,
            &ManualClaimRequest {
                text: "expired",
                valid_to_epoch: Some(1),
                ..request("expired", UserContextSensitivity::Normal)
            },
        )?;
        create_manual_claim(
            &conn,
            &ManualClaimRequest {
                text: "future",
                valid_from_epoch: Some(chrono::Utc::now().timestamp() + 86_400),
                ..request("future", UserContextSensitivity::Normal)
            },
        )?;
        create_manual_claim(
            &conn,
            &request("restricted", UserContextSensitivity::Restricted),
        )?;

        let visible = list_claims(
            &conn,
            &ClaimListRequest {
                owner_scope: None,
                owner_key: None,
                include_inactive: false,
                limit: 100,
            },
        )?;
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, active.id);

        let all = list_claims(
            &conn,
            &ClaimListRequest {
                owner_scope: None,
                owner_key: None,
                include_inactive: true,
                limit: 100,
            },
        )?;
        assert_eq!(all.len(), 6);
        Ok(())
    }

    #[test]
    fn suppress_and_unsuppress_only_cross_active_suppressed_boundary() -> Result<()> {
        let conn = migrated_conn()?;
        let active = create_manual_claim(
            &conn,
            &request("governance boundary", UserContextSensitivity::Normal),
        )?;

        assert_eq!(suppress_claim(&conn, active.id)?.status, "suppressed");
        assert_eq!(suppress_claim(&conn, active.id)?.status, "suppressed");
        let err = edit_claim(
            &conn,
            active.id,
            &ClaimEditRequest {
                text: "restored through edit",
                claim_type: None,
                claim_key: None,
                sensitivity: None,
                valid_from_epoch: None,
                valid_to_epoch: None,
            },
        )
        .expect_err("suppressed claims must not be restored through edit");
        assert!(err
            .to_string()
            .contains("only active user-context claims can be edited"));
        assert_eq!(unsuppress_claim(&conn, active.id)?.status, "active");

        let deleted = delete_claim(&conn, active.id)?;
        assert_eq!(deleted.status, "deleted");
        let err = unsuppress_claim(&conn, active.id)
            .expect_err("deleted claims must not be restored through unsuppress");
        assert!(err
            .to_string()
            .contains("only suppressed user-context claims can be unsuppressed"));
        Ok(())
    }

    #[test]
    fn edit_supersedes_old_claim_and_inserts_new_active_claim() -> Result<()> {
        let conn = migrated_conn()?;
        let original =
            create_manual_claim(&conn, &request("old text", UserContextSensitivity::Normal))?;

        let edited = edit_claim(
            &conn,
            original.id,
            &ClaimEditRequest {
                text: "new text",
                claim_type: Some(UserContextClaimType::Goal),
                claim_key: Some("goal:remem"),
                sensitivity: Some(UserContextSensitivity::Personal),
                valid_from_epoch: None,
                valid_to_epoch: None,
            },
        )?;

        assert_eq!(edited.previous.id, original.id);
        assert_eq!(load_claim(&conn, original.id)?.status, "superseded");
        assert_eq!(edited.current.supersedes_claim_id, Some(original.id));
        assert_eq!(edited.current.claim_text, "new text");
        assert_eq!(edited.current.claim_type, "goal");
        assert_eq!(edited.current.claim_key, "goal:remem");
        assert_eq!(edited.current.sensitivity, "personal");
        assert_eq!(edited.current.status, "active");
        Ok(())
    }

    fn request(text: &str, sensitivity: UserContextSensitivity) -> ManualClaimRequest<'_> {
        ManualClaimRequest {
            text,
            owner_scope: None,
            owner_key: None,
            claim_type: UserContextClaimType::Preference,
            claim_key: None,
            confidence: 1.0,
            sensitivity,
            valid_from_epoch: None,
            valid_to_epoch: None,
        }
    }
}
