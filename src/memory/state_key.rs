use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

#[derive(Debug, Clone, PartialEq)]
pub struct StateKeyDecision {
    pub state_key: String,
    pub confidence: f64,
    pub reason: String,
}

pub fn derive_state_key(
    memory_type: &str,
    topic_key: Option<&str>,
    title: &str,
    content: &str,
) -> Option<StateKeyDecision> {
    if let Some(topic_key) = stable_state_topic_key(topic_key) {
        return Some(StateKeyDecision {
            state_key: topic_key,
            confidence: 1.0,
            reason: "stable_topic_key".to_string(),
        });
    }

    if memory_type == "preference" {
        return derive_preference_state_key(title, content);
    }

    None
}

pub fn current_memory_id(
    conn: &Connection,
    owner_scope: &str,
    owner_key: &str,
    memory_type: &str,
    state_key: &str,
) -> Result<Option<i64>> {
    conn.query_row(
        "SELECT m.id
         FROM memory_state_keys sk
         JOIN memories m ON m.id = sk.current_memory_id
         WHERE sk.owner_scope = ?1
           AND sk.owner_key = ?2
           AND sk.memory_type = ?3
           AND sk.state_key = ?4
           AND sk.state_status = 'active'
           AND m.status = 'active'
         LIMIT 1",
        params![owner_scope, owner_key, memory_type, state_key],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

pub fn active_memory_ids(
    conn: &Connection,
    owner_scope: &str,
    owner_key: &str,
    memory_type: &str,
    state_key: &str,
    now_epoch: i64,
    require_unexpired: bool,
) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT m.id
         FROM memories m
         JOIN memory_state_keys sk ON sk.id = m.state_key_id
         WHERE sk.owner_scope = ?1
           AND sk.owner_key = ?2
           AND sk.memory_type = ?3
           AND sk.state_key = ?4
           AND sk.state_status = 'active'
           AND m.status = 'active'
           AND (
                ?5 = 0
                OR m.expires_at_epoch IS NULL
                OR m.expires_at_epoch > ?6
           )
         ORDER BY m.updated_at_epoch DESC, m.id DESC",
    )?;
    let rows = stmt.query_map(
        params![
            owner_scope,
            owner_key,
            memory_type,
            state_key,
            if require_unexpired { 1_i64 } else { 0_i64 },
            now_epoch
        ],
        |row| row.get(0),
    )?;
    crate::db::query::collect_rows(rows)
}

pub fn attach_current_memory(
    conn: &Connection,
    memory_id: i64,
    owner_scope: &str,
    owner_key: &str,
    memory_type: &str,
    decision: &StateKeyDecision,
    now_epoch: i64,
) -> Result<i64> {
    let state_key_id = upsert_state_key(
        conn,
        owner_scope,
        owner_key,
        memory_type,
        decision,
        Some(memory_id),
        now_epoch,
    )?;
    conn.execute(
        "UPDATE memories SET state_key_id = ?1 WHERE id = ?2",
        params![state_key_id, memory_id],
    )?;
    Ok(state_key_id)
}

fn upsert_state_key(
    conn: &Connection,
    owner_scope: &str,
    owner_key: &str,
    memory_type: &str,
    decision: &StateKeyDecision,
    current_memory_id: Option<i64>,
    now_epoch: i64,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO memory_state_keys
         (owner_scope, owner_key, memory_type, state_key, state_label, state_status,
          current_memory_id, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?7, ?7)
         ON CONFLICT(owner_scope, owner_key, memory_type, state_key)
         DO UPDATE SET
             state_label = COALESCE(excluded.state_label, memory_state_keys.state_label),
             state_status = 'active',
             current_memory_id = COALESCE(excluded.current_memory_id, memory_state_keys.current_memory_id),
             updated_at_epoch = excluded.updated_at_epoch",
        params![
            owner_scope,
            owner_key,
            memory_type,
            decision.state_key,
            decision.state_key.replace('-', " "),
            current_memory_id,
            now_epoch
        ],
    )?;
    conn.query_row(
        "SELECT id FROM memory_state_keys
         WHERE owner_scope = ?1
           AND owner_key = ?2
           AND memory_type = ?3
           AND state_key = ?4",
        params![owner_scope, owner_key, memory_type, decision.state_key],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

fn stable_state_topic_key(topic_key: Option<&str>) -> Option<String> {
    let topic_key = topic_key?.trim();
    if topic_key.is_empty() || is_hash_like_topic_key(topic_key) {
        return None;
    }
    let slug = crate::memory::promote::slugify_for_topic(topic_key, 120);
    if slug.is_empty() {
        None
    } else {
        Some(slug)
    }
}

fn derive_preference_state_key(title: &str, content: &str) -> Option<StateKeyDecision> {
    let combined = format!("{title}\n{content}");
    if mentions_verification_status(&combined) && mentions_data_code_separation(&combined) {
        return Some(StateKeyDecision {
            state_key: "verification-status-separation".to_string(),
            confidence: 0.95,
            reason: "preference_domain_verification_status_separation".to_string(),
        });
    }
    if mentions_data_code_separation(&combined) {
        return Some(StateKeyDecision {
            state_key: "data-code-change-separation".to_string(),
            confidence: 0.90,
            reason: "preference_domain_data_code_separation".to_string(),
        });
    }
    if mentions_codesign_binary(&combined) {
        return Some(StateKeyDecision {
            state_key: "local-rust-binary-codesign-after-cp".to_string(),
            confidence: 0.90,
            reason: "preference_domain_codesign_binary".to_string(),
        });
    }

    None
}

fn is_hash_like_topic_key(topic_key: &str) -> bool {
    let lower = topic_key.to_ascii_lowercase();
    let mut parts = lower.rsplitn(2, ['-', '_']);
    let tail = parts.next().unwrap_or_default();
    let prefix = parts.next().unwrap_or_default();
    tail.len() >= 8
        && tail.chars().all(|ch| ch.is_ascii_hexdigit())
        && matches!(
            prefix,
            "decision"
                | "discovery"
                | "preference"
                | "bugfix"
                | "lesson"
                | "procedure"
                | "architecture"
        )
}

fn mentions_verification_status(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("verification status")
        || lower.contains("verify status")
        || (text.contains("验证") && text.contains("状态"))
}

fn mentions_data_code_separation(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let has_data_code = (lower.contains("data") && lower.contains("code"))
        || (text.contains("数据") && text.contains("代码"));
    let has_separation = lower.contains("separat")
        || lower.contains("distinct")
        || text.contains("分开")
        || text.contains("分离")
        || text.contains("隔离");
    has_data_code && has_separation
}

fn mentions_codesign_binary(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("codesign")
        && (lower.contains("binary")
            || lower.contains("bin/")
            || lower.contains("target/release")
            || lower.contains("cp "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_topic_key_is_preserved_as_state_key() {
        let decision = derive_state_key(
            "decision",
            Some("deploy-target"),
            "Deploy target",
            "Deploy to staging.",
        )
        .expect("stable topic key should derive");
        assert_eq!(decision.state_key, "deploy-target");
        assert_eq!(decision.reason, "stable_topic_key");
    }

    #[test]
    fn hash_like_topic_key_uses_ascii_preference_domain() {
        let decision = derive_state_key(
            "preference",
            Some("preference-1234abcd"),
            "Preference",
            "Keep verification status separate from data and code changes.",
        )
        .expect("semantic preference should derive");
        assert_eq!(decision.state_key, "verification-status-separation");
    }

    #[test]
    fn hash_like_topic_key_uses_cjk_preference_domain() {
        let decision = derive_state_key(
            "preference",
            Some("preference-deadbeef"),
            "Preference",
            "验证状态必须和数据、代码变更分开说明。",
        )
        .expect("CJK semantic preference should derive");
        assert_eq!(decision.state_key, "verification-status-separation");
    }

    #[test]
    fn ambiguous_hash_like_non_preference_is_not_invented() {
        assert!(derive_state_key(
            "decision",
            Some("decision-deadbeef"),
            "Decision",
            "A short ambiguous note.",
        )
        .is_none());
    }
}
