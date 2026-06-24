use anyhow::Result;
use rusqlite::{params, Connection};

pub(super) const MATCH_REASON_SESSION_LINK: &str = "session_link";
pub(super) const MATCH_REASON_ALIAS_EXACT: &str = "alias_exact";
pub(super) const MATCH_REASON_TITLE_EXACT: &str = "title_exact";
pub(super) const MATCH_REASON_TITLE_CONTAINS: &str = "title_contains";
pub(super) const MATCH_REASON_INSERT: &str = "insert";

const BROAD_TOKENS: &[&str] = &[
    "task",
    "tasks",
    "issue",
    "issues",
    "feature",
    "features",
    "fix",
    "review",
    "reviews",
    "workflow",
    "workflows",
    "workstream",
    "workstreams",
    "skill",
    "skills",
    "implementation",
    "docs",
    "doc",
    "test",
    "tests",
    "pr",
];

pub(super) fn normalize_title(title: &str) -> String {
    let mut out = String::new();
    let mut pending_space = false;

    for ch in title.trim().chars() {
        let normalized = match ch {
            '/' | '\\' | '-' | '_' | ':' | ';' | ',' | '.' | '(' | ')' | '[' | ']' | '{' | '}'
            | '<' | '>' | '|' => ' ',
            _ => ch,
        };

        if normalized.is_whitespace() {
            pending_space = !out.is_empty();
            continue;
        }

        if pending_space && !out.is_empty() {
            out.push(' ');
        }
        pending_space = false;

        for lower in normalized.to_lowercase() {
            out.push(lower);
        }
    }

    out
}

pub(super) fn title_has_continuity(left: &str, right: &str) -> bool {
    let left = normalize_title(left);
    let right = normalize_title(right);
    if left.is_empty() || right.is_empty() {
        return false;
    }
    if left == right || left.contains(&right) || right.contains(&left) {
        return true;
    }

    let left_tokens = meaningful_tokens(&left);
    if left_tokens.is_empty() {
        return false;
    }
    let right_tokens = meaningful_tokens(&right);
    let shared = right_tokens
        .iter()
        .filter(|token| left_tokens.iter().any(|candidate| candidate == *token))
        .collect::<Vec<_>>();
    shared.len() >= 2
        || shared
            .iter()
            .any(|token| token_is_strong_continuity_anchor(token))
}

pub(super) fn workstream_identity_key(
    project: &str,
    memory_session_id: &str,
    created_at_epoch: i64,
    workstream_id: i64,
) -> String {
    let input = format!("{project}\0{memory_session_id}\0{created_at_epoch}\0{workstream_id}");
    let hash = crate::db::content_identity_hash(input.as_bytes());
    let suffix = hash.rsplit(':').next().unwrap_or(hash.as_str());
    format!("ws_{}", &suffix[..16])
}

pub(super) fn ensure_workstream_alias(
    conn: &Connection,
    workstream_id: i64,
    title: &str,
    source: &str,
    memory_session_id: Option<&str>,
    source_workstream_id: Option<i64>,
    now: i64,
) -> Result<()> {
    let normalized_title = normalize_title(title);
    if normalized_title.is_empty() {
        return Ok(());
    }

    conn.execute(
        "INSERT INTO workstream_aliases
         (workstream_id, title, normalized_title, first_seen_epoch, last_seen_epoch)
         VALUES (?1, ?2, ?3, ?4, ?4)
         ON CONFLICT(workstream_id, normalized_title) DO UPDATE SET
            last_seen_epoch = MAX(last_seen_epoch, excluded.last_seen_epoch)",
        params![workstream_id, title, normalized_title, now],
    )?;

    let alias_id: i64 = conn.query_row(
        "SELECT id FROM workstream_aliases
         WHERE workstream_id = ?1 AND normalized_title = ?2",
        params![workstream_id, normalized_title],
        |row| row.get(0),
    )?;

    conn.execute(
        "INSERT INTO workstream_alias_sources
         (alias_id, source, memory_session_id, source_workstream_id, observed_title,
          first_seen_epoch, last_seen_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
        params![
            alias_id,
            source,
            memory_session_id,
            source_workstream_id,
            title,
            now,
        ],
    )?;

    Ok(())
}

pub(super) fn has_continuity_alias(
    conn: &Connection,
    workstream_id: i64,
    incoming_title: &str,
) -> Result<bool> {
    let mut stmt = conn.prepare(
        "SELECT title FROM workstream_aliases
         WHERE workstream_id = ?1
         ORDER BY last_seen_epoch DESC",
    )?;
    let rows = stmt.query_map(params![workstream_id], |row| row.get::<_, String>(0))?;
    for row in rows {
        if title_has_continuity(&row?, incoming_title) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn meaningful_tokens(normalized: &str) -> Vec<&str> {
    normalized
        .split_whitespace()
        .filter(|token| !BROAD_TOKENS.contains(token))
        .filter(|token| !token.is_ascii() || token.len() >= 4)
        .collect()
}

fn token_is_strong_continuity_anchor(token: &str) -> bool {
    token.chars().any(|ch| !ch.is_ascii()) && token.chars().count() >= 4
}

#[cfg(test)]
mod tests {
    use super::{normalize_title, title_has_continuity};

    #[test]
    fn title_normalization_collapses_separator_noise() {
        assert_eq!(
            normalize_title(" flowguard / run-guard Skill "),
            "flowguard run guard skill"
        );
    }

    #[test]
    fn continuity_keeps_spellbook_rename_chain_but_not_unrelated_tasks() {
        assert!(title_has_continuity(
            "agent-workflow Skill 生命周期工作流",
            "flowguard Skill 生命周期工作流"
        ));
        assert!(title_has_continuity(
            "flowguard Skill 生命周期工作流",
            "flowguard / run-guard Skill 生命周期工作流"
        ));
        assert!(!title_has_continuity(
            "agent-workflow Skill 生命周期工作流",
            "release notes cleanup"
        ));
        assert!(!title_has_continuity(
            "billing import cleanup",
            "billing dashboard rollout"
        ));
    }
}
