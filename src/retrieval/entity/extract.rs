/// Extract simple entities from text (project names, tools, concepts).
/// No LLM needed — rule-based extraction from title + content.
pub fn extract_entities(title: &str, content: &str) -> Vec<String> {
    let mut entities = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let combined = format!("{} {}", title, content);

    for word in combined.split_whitespace() {
        let clean = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_');
        if clean.len() < 2 {
            continue;
        }
        if clean
            .chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false)
            && clean.chars().any(|c| c.is_lowercase())
            && clean.len() >= 3
        {
            let lower = clean.to_lowercase();
            if !is_stop_word(&lower) && seen.insert(lower.clone()) {
                entities.push(clean.to_string());
            }
        }
        if clean.len() >= 2
            && clean.len() <= 8
            && clean
                .chars()
                .all(|c| c.is_uppercase() || c.is_ascii_digit())
        {
            let lower = clean.to_lowercase();
            if seen.insert(lower) {
                entities.push(clean.to_string());
            }
        }
    }

    let lower_combined = combined.to_lowercase();
    for term in technical_terms() {
        if lower_combined.contains(&term.to_lowercase()) && seen.insert(term.to_lowercase()) {
            entities.push(term.to_string());
        }
    }

    entities.truncate(10);
    entities
}

fn technical_terms() -> &'static [&'static str] {
    &[
        "remem",
        "sqlite",
        "sqlcipher",
        "fts5",
        "trigram",
        "axum",
        "tokio",
        "claude",
        "codex",
        "cursor",
        "aider",
        "mem0",
        "zep",
        "letta",
        "engram",
        "hindsight",
        "mcp",
        "hook",
        "ToolAdapter",
        "REST",
        "API",
    ]
}

fn is_stop_word(word: &str) -> bool {
    matches!(
        word,
        "the"
            | "and"
            | "for"
            | "with"
            | "from"
            | "that"
            | "this"
            | "into"
            | "when"
            | "what"
            | "how"
            | "not"
            | "are"
            | "was"
            | "has"
            | "had"
            | "will"
            | "can"
            | "all"
            | "but"
            | "use"
            | "new"
            | "add"
            | "set"
            | "run"
            | "get"
            | "let"
            | "some"
            | "none"
            | "used"
            | "using"
            | "session"
            | "request"
            | "context"
            | "decisions"
            | "learned"
    )
}
