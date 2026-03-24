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
