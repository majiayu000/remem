use rusqlite::Connection;

const MAX_HINT_CHARS: usize = 180;
const MAX_CONTEXT_CHARS: usize = 4000;

pub fn build_search_context(
    memory_type: &str,
    topic_key: Option<&str>,
    content: &str,
    files: Option<&str>,
) -> String {
    let mut hints = Vec::new();
    push_hint(&mut hints, format!("type: {memory_type}"));

    if let Some(topic_key) = topic_key.and_then(non_empty) {
        push_hint(
            &mut hints,
            format!("topic: {}", topic_key.replace(['-', '_'], " ")),
        );
    }

    let file_hints = parse_file_hints(files);
    if !file_hints.is_empty() {
        push_hint(&mut hints, format!("files: {}", file_hints.join(" ")));
    }

    for (label, snippet) in extract_labeled_hints(content) {
        push_hint(&mut hints, format!("{label}: {snippet}"));
    }

    let commands = extract_commands(content);
    if !commands.is_empty() {
        push_hint(&mut hints, format!("commands: {}", commands.join(" ; ")));
    }

    truncate_context(&hints.join("\n"))
}

pub fn rebuild_all(conn: &Connection) -> anyhow::Result<usize> {
    let rows = {
        let mut stmt = conn.prepare(
            "SELECT id, topic_key, content, memory_type, files
             FROM memories",
        )?;
        let mapped = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?;
        mapped.collect::<Result<Vec<_>, _>>()?
    };

    let mut changed = 0usize;
    for (id, topic_key, content, memory_type, files) in rows {
        let search_context = build_search_context(
            &memory_type,
            topic_key.as_deref(),
            &content,
            files.as_deref(),
        );
        changed += conn.execute(
            "UPDATE memories SET search_context = ?1 WHERE id = ?2",
            rusqlite::params![search_context, id],
        )?;
    }
    Ok(changed)
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn parse_file_hints(files: Option<&str>) -> Vec<String> {
    let Some(raw) = files.and_then(non_empty) else {
        return vec![];
    };

    let paths = serde_json::from_str::<Vec<String>>(raw).unwrap_or_else(|_| vec![raw.to_string()]);
    let mut hints = Vec::new();
    for path in paths {
        let Some(path) = non_empty(&path) else {
            continue;
        };
        push_hint(&mut hints, path.to_string());
        if let Some(basename) = path.rsplit('/').next().and_then(non_empty) {
            push_hint(&mut hints, basename.to_string());
        }
    }
    hints
}

fn extract_labeled_hints(content: &str) -> Vec<(&'static str, String)> {
    let cues = [
        ("symptom", &["symptom:", "issue:", "problem:", "error:"][..]),
        ("root cause", &["root cause:", "cause:"][..]),
        (
            "fix",
            &["fix:", "fixed:", "resolved by", "resolution:", "solution:"][..],
        ),
        (
            "verification",
            &["verification:", "verified", "tests:", "test:"][..],
        ),
        ("outcome", &["outcome:", "result:"][..]),
    ];

    let lower = content.to_lowercase();
    let mut found = Vec::new();
    for (label, variants) in cues {
        if let Some((position, cue)) = variants
            .iter()
            .filter_map(|cue| lower.find(cue).map(|pos| (pos, *cue)))
            .min_by_key(|(pos, _)| *pos)
        {
            let start = position + cue.len();
            let snippet = snippet_after(content, start);
            push_labeled_hint(&mut found, label, snippet);
        }
    }
    found
}

fn snippet_after(content: &str, start: usize) -> String {
    let snippet = content
        .get(start..)
        .unwrap_or("")
        .split(['\n', '.', ';'])
        .next()
        .unwrap_or("")
        .trim();
    if snippet.is_empty() {
        return String::new();
    }
    truncate_context(snippet)
        .chars()
        .take(MAX_HINT_CHARS)
        .collect::<String>()
}

fn push_labeled_hint(
    hints: &mut Vec<(&'static str, String)>,
    label: &'static str,
    snippet: String,
) {
    if hints.iter().any(|(existing, _)| *existing == label) {
        return;
    }
    hints.push((label, snippet));
}

fn extract_commands(content: &str) -> Vec<String> {
    let mut commands = Vec::new();
    let mut in_tick = false;
    let mut current = String::new();
    for ch in content.chars() {
        if ch == '`' {
            if in_tick {
                if looks_like_command(&current) {
                    push_hint(&mut commands, current.trim().to_string());
                }
                current.clear();
            }
            in_tick = !in_tick;
        } else if in_tick {
            current.push(ch);
        }
    }

    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(command) = trimmed.strip_prefix("$ ").and_then(non_empty) {
            push_hint(&mut commands, command.to_string());
        }
    }
    commands
}

fn looks_like_command(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.len() < 3 || trimmed.len() > MAX_HINT_CHARS {
        return false;
    }
    let Some(first) = trimmed.split_whitespace().next() else {
        return false;
    };
    matches!(
        first,
        "cargo"
            | "go"
            | "pytest"
            | "python"
            | "python3"
            | "node"
            | "npm"
            | "npx"
            | "pnpm"
            | "yarn"
            | "bun"
            | "deno"
            | "git"
            | "gh"
            | "uv"
            | "make"
            | "just"
            | "sqlite3"
            | "remem"
    )
}

fn push_hint(hints: &mut Vec<String>, value: String) {
    let normalized = value.trim();
    if normalized.is_empty() || hints.iter().any(|hint| hint == normalized) {
        return;
    }
    hints.push(normalized.to_string());
}

fn truncate_context(value: &str) -> String {
    if value.len() <= MAX_CONTEXT_CHARS {
        return value.to_string();
    }
    crate::db::truncate_str(value, MAX_CONTEXT_CHARS).to_string()
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::{build_search_context, rebuild_all};

    #[test]
    fn search_context_includes_rebuildable_structured_hints() {
        let context = build_search_context(
            "bugfix",
            Some("cache-key-timeout"),
            "Issue: requests timed out. Cause: cache key drift. Resolved by invalidating \
             stale entries. Verified with `cargo test retrieval::memory_search`.",
            Some(r#"["src/retrieval/memory_search/fts.rs"]"#),
        );

        assert!(context.contains("type: bugfix"));
        assert!(context.contains("topic: cache key timeout"));
        assert!(context.contains("files: src/retrieval/memory_search/fts.rs fts.rs"));
        assert!(context.contains("symptom: requests timed out"));
        assert!(context.contains("root cause: cache key drift"));
        assert!(context.contains("fix: invalidating stale entries"));
        assert!(context.contains("verification: with `cargo test retrieval::memory_search`"));
        assert!(context.contains("commands: cargo test retrieval::memory_search"));
    }

    #[test]
    fn rebuild_all_regenerates_context_from_stored_metadata() -> anyhow::Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::memory::types::tests_helper::setup_memory_schema(&conn);
        conn.execute(
            "INSERT INTO memories
             (id, session_id, project, topic_key, title, content, memory_type, files,
              created_at_epoch, updated_at_epoch, status, branch, scope)
             VALUES (1, NULL, 'proj', 'search-context-rebuild', 'Title',
                     'Issue: miss. Resolved by adding context. Verified with `cargo test`.',
                     'bugfix', '[\"src/memory/search_context.rs\"]',
                     100, 100, 'active', NULL, 'project')",
            [],
        )?;

        let changed = rebuild_all(&conn)?;
        assert_eq!(changed, 1);
        let context: String = conn.query_row(
            "SELECT search_context FROM memories WHERE id = 1",
            [],
            |row| row.get(0),
        )?;
        assert!(context.contains("search context rebuild"));
        assert!(context.contains("src/memory/search_context.rs"));
        assert!(context.contains("commands: cargo test"));
        Ok(())
    }
}
