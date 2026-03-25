use anyhow::Result;
use rusqlite::{params, Connection};

/// Extract simple entities from text (project names, tools, concepts).
/// No LLM needed — rule-based extraction from title + content.
pub fn extract_entities(title: &str, content: &str) -> Vec<String> {
    let mut entities = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let combined = format!("{} {}", title, content);

    // Extract capitalized words/phrases (tools, frameworks, proper nouns)
    for word in combined.split_whitespace() {
        let clean = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_');
        if clean.len() < 2 {
            continue;
        }
        // Capitalized English words (FTS5, SQLCipher, Rust, Claude, etc.)
        if clean.chars().next().map(|c| c.is_uppercase()).unwrap_or(false)
            && clean.chars().any(|c| c.is_lowercase())
            && clean.len() >= 3
        {
            let lower = clean.to_lowercase();
            if !is_stop_word(&lower) && seen.insert(lower.clone()) {
                entities.push(clean.to_string());
            }
        }
        // ALL-CAPS acronyms (FTS5, API, MCP, RRF, etc.)
        if clean.len() >= 2
            && clean.len() <= 8
            && clean.chars().all(|c| c.is_uppercase() || c.is_ascii_digit())
        {
            let lower = clean.to_lowercase();
            if seen.insert(lower) {
                entities.push(clean.to_string());
            }
        }
    }

    // Extract known technical terms
    let tech_terms = [
        "remem", "sqlite", "sqlcipher", "fts5", "trigram", "axum", "tokio",
        "claude", "codex", "cursor", "aider", "mem0", "zep", "letta", "engram",
        "hindsight", "mcp", "hook", "ToolAdapter", "REST", "API",
    ];
    let lower_combined = combined.to_lowercase();
    for term in &tech_terms {
        if lower_combined.contains(&term.to_lowercase()) && seen.insert(term.to_lowercase()) {
            entities.push(term.to_string());
        }
    }

    entities.truncate(10); // Cap at 10 entities per memory
    entities
}

/// Link entities to a memory. Creates entities if they don't exist.
pub fn link_entities(conn: &Connection, memory_id: i64, entities: &[String]) -> Result<()> {
    for name in entities {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        // UPSERT entity
        conn.execute(
            "INSERT INTO entities (canonical_name, entity_type, mention_count)
             VALUES (?1, NULL, 1)
             ON CONFLICT(canonical_name) DO UPDATE SET mention_count = mention_count + 1",
            params![name],
        )?;
        let entity_id: i64 = conn.query_row(
            "SELECT id FROM entities WHERE canonical_name = ?1 COLLATE NOCASE",
            params![name],
            |row| row.get(0),
        )?;
        // Link memory ↔ entity
        conn.execute(
            "INSERT OR IGNORE INTO memory_entities (memory_id, entity_id) VALUES (?1, ?2)",
            params![memory_id, entity_id],
        )?;
    }
    Ok(())
}

/// Search memories by entity name. Returns memory IDs sorted by relevance.
pub fn search_by_entity(conn: &Connection, query: &str, limit: i64) -> Result<Vec<i64>> {
    // Extract potential entity names from query
    let query_entities = extract_entities(query, "");

    if query_entities.is_empty() {
        // Fallback: try each query word as an entity
        let mut ids = Vec::new();
        for word in query.split_whitespace() {
            if word.len() < 2 {
                continue;
            }
            let pattern = format!("%{}%", word);
            let mut stmt = conn.prepare(
                "SELECT DISTINCT me.memory_id FROM memory_entities me
                 JOIN entities e ON e.id = me.entity_id
                 WHERE e.canonical_name LIKE ?1 COLLATE NOCASE
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![pattern, limit], |r| r.get::<_, i64>(0))?;
            for row in rows {
                let id = row?;
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        return Ok(ids);
    }

    let mut all_ids = Vec::new();
    for entity_name in &query_entities {
        let mut stmt = conn.prepare(
            "SELECT DISTINCT me.memory_id FROM memory_entities me
             JOIN entities e ON e.id = me.entity_id
             WHERE e.canonical_name = ?1 COLLATE NOCASE
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![entity_name, limit], |r| r.get::<_, i64>(0))?;
        for row in rows {
            let id = row?;
            if !all_ids.contains(&id) {
                all_ids.push(id);
            }
        }
    }
    Ok(all_ids)
}

/// Entity graph expansion for multi-hop retrieval.
/// Given a set of memory IDs (first-hop results), find co-occurring entities
/// in those memories, then find OTHER memories that mention those entities.
/// This enables "Melanie → Tom (her kid) → Tom likes dinosaurs" chains.
pub fn expand_via_entity_graph(
    conn: &Connection,
    seed_memory_ids: &[i64],
    exclude_ids: &[i64],
    limit: i64,
) -> Result<Vec<i64>> {
    if seed_memory_ids.is_empty() {
        return Ok(vec![]);
    }

    // Step 1: Get all entity IDs linked to seed memories
    let placeholders: Vec<String> = (1..=seed_memory_ids.len()).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "SELECT DISTINCT entity_id FROM memory_entities WHERE memory_id IN ({})",
        placeholders.join(", ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let param_values: Vec<Box<dyn rusqlite::types::ToSql>> = seed_memory_ids
        .iter()
        .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
        .collect();
    let refs: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|b| b.as_ref()).collect();
    let entity_rows = stmt.query_map(refs.as_slice(), |r| r.get::<_, i64>(0))?;
    let entity_ids: Vec<i64> = entity_rows.flatten().collect();

    if entity_ids.is_empty() {
        return Ok(vec![]);
    }

    // Step 2: Find memories linked to these entities, excluding already-seen ones
    let entity_placeholders: Vec<String> =
        (1..=entity_ids.len()).map(|i| format!("?{i}")).collect();

    let exclude_set: std::collections::HashSet<i64> = exclude_ids.iter().copied().collect();
    let seed_set: std::collections::HashSet<i64> = seed_memory_ids.iter().copied().collect();

    let sql2 = format!(
        "SELECT me.memory_id, COUNT(DISTINCT me.entity_id) as shared_count
         FROM memory_entities me
         WHERE me.entity_id IN ({})
         GROUP BY me.memory_id
         ORDER BY shared_count DESC
         LIMIT ?{}",
        entity_placeholders.join(", "),
        entity_ids.len() + 1
    );
    let mut stmt2 = conn.prepare(&sql2)?;
    let mut param_values2: Vec<Box<dyn rusqlite::types::ToSql>> = entity_ids
        .iter()
        .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
        .collect();
    // Over-fetch to account for filtering
    param_values2.push(Box::new(limit * 3));
    let refs2: Vec<&dyn rusqlite::types::ToSql> = param_values2.iter().map(|b| b.as_ref()).collect();

    let rows2 = stmt2.query_map(refs2.as_slice(), |r| r.get::<_, i64>(0))?;
    let mut expanded_ids = Vec::new();
    for row in rows2 {
        let id = row?;
        if !seed_set.contains(&id) && !exclude_set.contains(&id) {
            expanded_ids.push(id);
            if expanded_ids.len() >= limit as usize {
                break;
            }
        }
    }

    Ok(expanded_ids)
}

fn is_stop_word(word: &str) -> bool {
    matches!(
        word,
        "the" | "and" | "for" | "with" | "from" | "that" | "this" | "into"
            | "when" | "what" | "how" | "not" | "are" | "was" | "has" | "had"
            | "will" | "can" | "all" | "but" | "use" | "new" | "add" | "set"
            | "run" | "get" | "let" | "some" | "none" | "used" | "using"
            | "session" | "request" | "context" | "decisions" | "learned"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_tool_names() {
        let entities = extract_entities("FTS5 trigram tokenizer for SQLCipher", "Using Rust and Axum");
        assert!(entities.iter().any(|e| e.contains("FTS5")));
        assert!(entities.iter().any(|e| e.to_lowercase() == "sqlcipher"));
        assert!(entities.iter().any(|e| e.to_lowercase() == "axum"));
    }

    #[test]
    fn extract_from_chinese_mixed() {
        let entities = extract_entities("remem 竞品分析", "对比 Mem0 和 Letta 的设计");
        assert!(entities.iter().any(|e| e.to_lowercase() == "remem"));
        assert!(entities.iter().any(|e| e.to_lowercase() == "mem0"));
        assert!(entities.iter().any(|e| e.to_lowercase() == "letta"));
    }

    #[test]
    fn no_stop_words() {
        let entities = extract_entities("The new API for this", "");
        assert!(!entities.iter().any(|e| e.to_lowercase() == "the"));
        assert!(entities.iter().any(|e| e == "API"));
    }
}
