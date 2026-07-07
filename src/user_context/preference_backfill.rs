use anyhow::{ensure, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use super::claims::{
    self, PreferenceBackfillClaimRequest, DEFAULT_OWNER_KEY, DEFAULT_OWNER_SCOPE,
    PREFERENCE_BACKFILL_SOURCE_KIND,
};

const MAX_BACKFILL_CLAIM_TEXT_CHARS: usize = 16_384;

#[derive(Debug, Clone, Copy)]
pub struct UserBackfillRequest {
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UserBackfillReport {
    pub applied: bool,
    pub limit: Option<i64>,
    pub candidates: Vec<UserBackfillCandidate>,
    pub converted: Vec<UserBackfillConverted>,
    pub skipped: Vec<UserBackfillSkipped>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UserBackfillCandidate {
    pub memory_id: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct UserBackfillConverted {
    pub memory_id: i64,
    pub claim_id: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct UserBackfillSkipped {
    pub memory_id: i64,
    pub reason: String,
}

#[derive(Debug, Clone)]
struct PreferenceMemory {
    id: i64,
    title: String,
    text: String,
}

#[derive(Debug, Clone)]
struct ExistingClaimMatch {
    status: String,
}

pub fn preview_backfill(
    conn: &Connection,
    req: &UserBackfillRequest,
) -> Result<UserBackfillReport> {
    validate_limit(req.limit)?;
    build_report(conn, false, req.limit)
}

pub fn apply_backfill(
    conn: &mut Connection,
    req: &UserBackfillRequest,
) -> Result<UserBackfillReport> {
    validate_limit(req.limit)?;
    let tx = conn.unchecked_transaction()?;
    let report = build_report(&tx, true, req.limit)?;
    tx.commit()?;
    Ok(report)
}

fn validate_limit(limit: Option<i64>) -> Result<()> {
    if let Some(limit) = limit {
        ensure!(limit > 0, "backfill limit must be positive");
    }
    Ok(())
}

fn build_report(conn: &Connection, apply: bool, limit: Option<i64>) -> Result<UserBackfillReport> {
    let sources = load_visible_user_preference_memories(conn, limit)?;
    let mut report = UserBackfillReport {
        applied: apply,
        limit,
        candidates: Vec::new(),
        converted: Vec::new(),
        skipped: Vec::new(),
        message: if apply {
            "User preference backfill applied.".to_string()
        } else {
            "Dry-run only; rerun with --apply to convert candidates.".to_string()
        },
    };

    for source in sources {
        let Some(decision) = evaluate_source(conn, &source)? else {
            if apply {
                let claim = claims::create_preference_backfill_claim(
                    conn,
                    &PreferenceBackfillClaimRequest {
                        memory_id: source.id,
                        text: &source.text,
                    },
                )?;
                report.converted.push(UserBackfillConverted {
                    memory_id: source.id,
                    claim_id: claim.id,
                });
            } else {
                report.candidates.push(UserBackfillCandidate {
                    memory_id: source.id,
                });
            }
            continue;
        };
        report.skipped.push(UserBackfillSkipped {
            memory_id: source.id,
            reason: decision,
        });
    }

    Ok(report)
}

fn load_visible_user_preference_memories(
    conn: &Connection,
    limit: Option<i64>,
) -> Result<Vec<PreferenceMemory>> {
    let policy_filter = crate::memory::suppression::memory_policy_filter_sql("memories");
    let current_filter =
        crate::memory::memory_current_filter_sql("status", "expires_at_epoch", false);
    let state_key_filter = crate::memory::memory_state_key_current_filter_sql("memories");
    let mut sql = format!(
        "SELECT id, title, content
         FROM memories
         WHERE memory_type = 'preference'
           AND owner_scope = ?1
           AND owner_key = ?2
           AND {current_filter}
           AND {state_key_filter}
           AND {policy_filter}
         ORDER BY updated_at_epoch DESC, id DESC"
    );
    if limit.is_some() {
        sql.push_str(" LIMIT ?3");
    }
    let mut stmt = conn.prepare(&sql)?;
    let rows = if let Some(limit) = limit {
        stmt.query_map(
            params![DEFAULT_OWNER_SCOPE, DEFAULT_OWNER_KEY, limit],
            preference_memory_from_row,
        )?
    } else {
        stmt.query_map(
            params![DEFAULT_OWNER_SCOPE, DEFAULT_OWNER_KEY],
            preference_memory_from_row,
        )?
    };
    crate::db::query::collect_rows(rows)
}

fn preference_memory_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PreferenceMemory> {
    Ok(PreferenceMemory {
        id: row.get(0)?,
        title: row.get(1)?,
        text: row.get(2)?,
    })
}

fn evaluate_source(conn: &Connection, source: &PreferenceMemory) -> Result<Option<String>> {
    if source.text.trim().is_empty() {
        return Ok(Some("empty_text".to_string()));
    }
    if source.text.chars().count() > MAX_BACKFILL_CLAIM_TEXT_CHARS {
        return Ok(Some("text_too_long".to_string()));
    }
    if let Some(reason) = super::non_retention::block_reason(
        &source.text,
        Some(&source.title),
        PREFERENCE_BACKFILL_SOURCE_KIND,
    ) {
        return Ok(Some(reason.to_string()));
    }
    if sensitivity_guard_blocks(&source.text) {
        return Ok(Some("sensitivity_uncertain".to_string()));
    }
    if let Some(existing) = existing_claim_for_source_memory(conn, source.id)? {
        return Ok(Some(duplicate_reason(&existing)));
    }
    let claim_key = claims::preference_claim_key(&source.text)?;
    if let Some(existing) = existing_claim_for_key(conn, &claim_key)? {
        return Ok(Some(duplicate_reason(&existing)));
    }
    Ok(None)
}

fn duplicate_reason(existing: &ExistingClaimMatch) -> String {
    if existing.status == "active" {
        "duplicate".to_string()
    } else {
        "governed_duplicate".to_string()
    }
}

fn existing_claim_for_key(
    conn: &Connection,
    claim_key: &str,
) -> Result<Option<ExistingClaimMatch>> {
    conn.query_row(
        "SELECT status
         FROM user_context_claims
         WHERE owner_scope = ?1
           AND owner_key = ?2
           AND claim_type = 'preference'
           AND claim_key = ?3
         ORDER BY CASE status WHEN 'active' THEN 0 ELSE 1 END,
                  updated_at_epoch DESC,
                  id DESC
        LIMIT 1",
        params![DEFAULT_OWNER_SCOPE, DEFAULT_OWNER_KEY, claim_key],
        |row| {
            Ok(ExistingClaimMatch {
                status: row.get(0)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

fn existing_claim_for_source_memory(
    conn: &Connection,
    memory_id: i64,
) -> Result<Option<ExistingClaimMatch>> {
    conn.query_row(
        "SELECT status
         FROM user_context_claims
         WHERE EXISTS (
             SELECT 1
             FROM json_each(
                 CASE
                     WHEN json_valid(user_context_claims.source_refs_json)
                     THEN user_context_claims.source_refs_json
                     ELSE '[]'
                 END
             ) ref
             WHERE json_extract(ref.value, '$.kind') = 'memory'
               AND json_extract(ref.value, '$.id') = ?1
         )
         ORDER BY CASE status WHEN 'active' THEN 0 ELSE 1 END,
                  updated_at_epoch DESC,
                  id DESC
        LIMIT 1",
        [memory_id],
        |row| {
            Ok(ExistingClaimMatch {
                status: row.get(0)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

fn sensitivity_guard_blocks(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    let sensitive_terms = [
        "address",
        "birthday",
        "credit card",
        "diagnosis",
        "email",
        "health",
        "home address",
        "medical",
        "passport",
        "personal",
        "phone",
        "private",
        "restricted",
        "sensitive",
        "ssn",
    ];
    sensitive_terms.iter().any(|term| text.contains(term))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::suppression::{create_suppression, parse_target, SuppressRequest};
    use rusqlite::{params, Connection};

    fn migrated_conn() -> Result<Connection> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        Ok(conn)
    }

    fn insert_memory_row(
        conn: &Connection,
        id: i64,
        text: &str,
        owner_scope: &str,
        owner_key: &str,
        memory_type: &str,
        status: &str,
        expires_at_epoch: Option<i64>,
    ) -> Result<()> {
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch,
              updated_at_epoch, status, scope, source_project, target_project,
              owner_scope, owner_key, expires_at_epoch)
             VALUES (?1, '/repo', 'Preference', ?2, ?3, 10, ?4, ?5, 'global',
                     '/repo', NULL, ?6, ?7, ?8)",
            params![
                id,
                text,
                memory_type,
                id * 10,
                status,
                owner_scope,
                owner_key,
                expires_at_epoch
            ],
        )?;
        Ok(())
    }

    fn insert_user_preference(conn: &Connection, id: i64, text: &str) -> Result<()> {
        insert_memory_row(
            conn,
            id,
            text,
            DEFAULT_OWNER_SCOPE,
            DEFAULT_OWNER_KEY,
            "preference",
            "active",
            None,
        )
    }

    #[test]
    fn dry_run_selects_visible_user_preferences_only() -> Result<()> {
        let conn = migrated_conn()?;
        insert_user_preference(&conn, 1, "Prefer concise review notes")?;
        insert_memory_row(
            &conn,
            2,
            "Repo preference",
            "repo",
            "/repo",
            "preference",
            "active",
            None,
        )?;
        insert_memory_row(
            &conn,
            3,
            "User decision",
            "user",
            "user:default",
            "decision",
            "active",
            None,
        )?;
        insert_memory_row(
            &conn,
            4,
            "Archived preference",
            "user",
            "user:default",
            "preference",
            "archived",
            None,
        )?;
        insert_memory_row(
            &conn,
            5,
            "Expired preference",
            "user",
            "user:default",
            "preference",
            "active",
            Some(1),
        )?;
        insert_user_preference(&conn, 6, "Suppressed preference")?;
        create_suppression(
            &conn,
            &SuppressRequest {
                target: parse_target("memory:6")?,
                reason: Some("test"),
                actor: Some("test"),
            },
        )?;

        let report = preview_backfill(&conn, &UserBackfillRequest { limit: None })?;

        assert!(!report.applied);
        assert_eq!(report.candidates.len(), 1);
        assert_eq!(report.candidates[0].memory_id, 1);
        assert!(report.converted.is_empty());
        assert!(report.skipped.is_empty());
        Ok(())
    }

    #[test]
    fn apply_converts_claim_with_source_ref_and_leaves_source_memory_unchanged() -> Result<()> {
        let mut conn = migrated_conn()?;
        insert_user_preference(&conn, 11, "Prefer architecture-first reviews")?;

        let report = apply_backfill(&mut conn, &UserBackfillRequest { limit: None })?;

        assert!(report.applied);
        assert!(report.candidates.is_empty());
        assert_eq!(report.converted.len(), 1);
        assert_eq!(report.converted[0].memory_id, 11);
        let claim = claims::load_claim(&conn, report.converted[0].claim_id)?;
        assert_eq!(claim.claim_type, "preference");
        assert_eq!(claim.source_kind, PREFERENCE_BACKFILL_SOURCE_KIND);
        assert_eq!(claim.sensitivity, "normal");
        assert_eq!(claim.status, "active");
        let refs: serde_json::Value = serde_json::from_str(&claim.source_refs_json)?;
        assert_eq!(refs[0]["kind"], "memory");
        assert_eq!(refs[0]["id"], 11);
        let source: (String, String) = conn.query_row(
            "SELECT content, status FROM memories WHERE id = 11",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(
            source,
            (
                "Prefer architecture-first reviews".to_string(),
                "active".to_string()
            )
        );
        Ok(())
    }

    #[test]
    fn repeated_apply_is_idempotent() -> Result<()> {
        let mut conn = migrated_conn()?;
        insert_user_preference(&conn, 21, "Prefer complete PR gate evidence")?;

        let first = apply_backfill(&mut conn, &UserBackfillRequest { limit: None })?;
        let second = apply_backfill(&mut conn, &UserBackfillRequest { limit: None })?;

        assert_eq!(first.converted.len(), 1);
        assert!(second.converted.is_empty());
        assert_eq!(second.skipped.len(), 1);
        assert_eq!(second.skipped[0].reason, "duplicate");
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM user_context_claims WHERE source_kind = ?1",
            [PREFERENCE_BACKFILL_SOURCE_KIND],
            |row| row.get(0),
        )?;
        assert_eq!(count, 1);
        Ok(())
    }

    #[test]
    fn governed_duplicate_claim_key_blocks_reactivation() -> Result<()> {
        let mut conn = migrated_conn()?;
        let text = "Prefer no hidden refactors";
        insert_user_preference(&conn, 31, text)?;
        let claim_key = claims::preference_claim_key(text)?;
        let existing = claims::create_manual_claim(
            &conn,
            &claims::ManualClaimRequest {
                text,
                owner_scope: None,
                owner_key: None,
                claim_type: claims::UserContextClaimType::Preference,
                claim_key: Some(&claim_key),
                confidence: 1.0,
                sensitivity: claims::UserContextSensitivity::Normal,
                valid_from_epoch: None,
                valid_to_epoch: None,
            },
        )?;
        claims::suppress_claim(&conn, existing.id)?;

        let report = apply_backfill(&mut conn, &UserBackfillRequest { limit: None })?;

        assert!(report.converted.is_empty());
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].reason, "governed_duplicate");
        Ok(())
    }

    #[test]
    fn governed_duplicate_source_ref_blocks_reactivation() -> Result<()> {
        let mut conn = migrated_conn()?;
        insert_user_preference(&conn, 32, "Prefer source refs over text matches")?;
        let existing = claims::create_manual_claim(
            &conn,
            &claims::ManualClaimRequest {
                text: "Different governed preference text",
                owner_scope: None,
                owner_key: None,
                claim_type: claims::UserContextClaimType::Preference,
                claim_key: Some("pref:different-governed"),
                confidence: 1.0,
                sensitivity: claims::UserContextSensitivity::Normal,
                valid_from_epoch: None,
                valid_to_epoch: None,
            },
        )?;
        conn.execute(
            "UPDATE user_context_claims
             SET source_kind = ?1,
                 source_refs_json = ?2
             WHERE id = ?3",
            params![
                PREFERENCE_BACKFILL_SOURCE_KIND,
                r#"[{"kind":"memory","id":32}]"#,
                existing.id
            ],
        )?;
        claims::suppress_claim(&conn, existing.id)?;

        let report = apply_backfill(&mut conn, &UserBackfillRequest { limit: None })?;

        assert!(report.converted.is_empty());
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].reason, "governed_duplicate");
        Ok(())
    }

    #[test]
    fn skips_non_retention_and_uncertain_sensitivity() -> Result<()> {
        let conn = migrated_conn()?;
        insert_user_preference(&conn, 41, "User's API key is sk-testsecret123456.")?;
        insert_user_preference(&conn, 42, "Private medical preference")?;
        let too_long = format!("Prefer {}", "x".repeat(MAX_BACKFILL_CLAIM_TEXT_CHARS));
        insert_user_preference(&conn, 43, &too_long)?;

        let report = preview_backfill(&conn, &UserBackfillRequest { limit: None })?;

        assert!(report.candidates.is_empty());
        assert_eq!(report.skipped.len(), 3);
        assert_eq!(report.skipped[0].memory_id, 43);
        assert_eq!(report.skipped[0].reason, "text_too_long");
        assert_eq!(report.skipped[1].memory_id, 42);
        assert_eq!(report.skipped[1].reason, "sensitivity_uncertain");
        assert_eq!(report.skipped[2].memory_id, 41);
        assert_eq!(report.skipped[2].reason, "secret_like_content");
        Ok(())
    }

    #[test]
    fn limit_bounds_processed_source_rows() -> Result<()> {
        let conn = migrated_conn()?;
        insert_user_preference(&conn, 51, "Prefer first")?;
        insert_user_preference(&conn, 52, "Prefer second")?;

        let report = preview_backfill(&conn, &UserBackfillRequest { limit: Some(1) })?;

        assert_eq!(report.candidates.len() + report.skipped.len(), 1);
        assert_eq!(report.candidates[0].memory_id, 52);
        Ok(())
    }
}
