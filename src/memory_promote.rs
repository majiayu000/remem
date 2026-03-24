use anyhow::Result;
use rusqlite::Connection;

use crate::memory::{insert_memory, insert_memory_full};

/// Minimum content length to be worth promoting.
const MIN_DECISION_LEN: usize = 30;
const MIN_LEARNED_LEN: usize = 30;
const MIN_PREFERENCE_LEN: usize = 10;

/// Generate a stable topic_key from text for UPSERT dedup.
pub fn slugify_for_topic(text: &str, max_len: usize) -> String {
    slugify(text, max_len)
}

fn slugify(text: &str, max_len: usize) -> String {
    let slug: String = text
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c
            } else if c == '-' || c == '_' || c == ' ' {
                '-'
            } else if !c.is_ascii() {
                c
            } else {
                '-'
            }
        })
        .collect();
    let mut result = String::with_capacity(slug.len());
    let mut last_dash = false;
    for c in slug.chars() {
        if c == '-' {
            if !last_dash && !result.is_empty() {
                result.push('-');
            }
            last_dash = true;
        } else {
            result.push(c);
            last_dash = false;
        }
    }
    let trimmed = result.trim_end_matches('-');
    if trimmed.len() <= max_len {
        trimmed.to_string()
    } else {
        trimmed.chars().take(max_len).collect()
    }
}

/// Split a multi-line text block into individual items.
/// Recognizes bullet points, numbered lists, and semicolons.
fn split_into_items(text: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let is_new_item = trimmed.starts_with("• ")
            || trimmed.starts_with("- ")
            || trimmed.starts_with("* ")
            || trimmed.starts_with("· ")
            || trimmed
                .chars()
                .next()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
                && trimmed.contains(". ");

        if is_new_item {
            if !current.trim().is_empty() {
                items.push(current.trim().to_string());
            }
            let content = trimmed
                .trim_start_matches(|c: char| c == '•' || c == '-' || c == '*' || c == '·')
                .trim_start();
            let content = if content
                .chars()
                .next()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
            {
                content
                    .find(". ")
                    .map(|pos| &content[pos + 2..])
                    .unwrap_or(content)
            } else {
                content
            };
            current = content.to_string();
        } else {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(trimmed);
        }
    }
    if !current.trim().is_empty() {
        items.push(current.trim().to_string());
    }

    if items.len() <= 1 {
        let original = text.trim();
        let semi_split: Vec<String> = original
            .split('；')
            .flat_map(|s| s.split(';'))
            .map(|s| s.trim().to_string())
            .filter(|s| s.len() >= MIN_DECISION_LEN)
            .collect();
        if semi_split.len() > 1 {
            return semi_split;
        }
    }

    items
}

/// Auto-promote session summary fields to memories.
/// Splits multi-item decisions/learned into individual memories.
/// Returns number of memories created/updated.
pub fn promote_summary_to_memories(
    conn: &Connection,
    session_id: &str,
    project: &str,
    request: Option<&str>,
    decisions: Option<&str>,
    learned: Option<&str>,
    preferences: Option<&str>,
) -> Result<usize> {
    let request_text = request.unwrap_or("").trim();
    let mut count = 0;

    if let Some(text) = decisions {
        let text = text.trim();
        if text.len() >= MIN_DECISION_LEN {
            let items = split_into_items(text);
            if items.len() > 1 {
                for (i, item) in items.iter().enumerate() {
                    if item.len() < MIN_DECISION_LEN {
                        continue;
                    }
                    let title = if request_text.is_empty() {
                        format!("Decision: {}", &item[..item.len().min(70)])
                    } else {
                        let preview = &request_text[..request_text.len().min(60)];
                        format!("{} — decision {}", preview, i + 1)
                    };
                    let topic_key =
                        format!("auto-decision-{}-{}", slugify(request_text, 40), i + 1);
                    insert_memory(
                        conn,
                        Some(session_id),
                        project,
                        Some(&topic_key),
                        &title,
                        item,
                        "decision",
                        None,
                    )?;
                    count += 1;
                }
            } else {
                let title = if request_text.is_empty() {
                    "Session decisions".to_string()
                } else {
                    let preview = &request_text[..request_text.len().min(80)];
                    format!("{} — decisions", preview)
                };
                let content = if request_text.is_empty() {
                    text.to_string()
                } else {
                    format!("**Request**: {}\n\n**Decisions**: {}", request_text, text)
                };
                let topic_key = format!("auto-decision-{}", slugify(request_text, 50));
                insert_memory(
                    conn,
                    Some(session_id),
                    project,
                    Some(&topic_key),
                    &title,
                    &content,
                    "decision",
                    None,
                )?;
                count += 1;
            }
        }
    }

    if let Some(text) = learned {
        let text = text.trim();
        if text.len() >= MIN_LEARNED_LEN {
            let items = split_into_items(text);
            if items.len() > 1 {
                for (i, item) in items.iter().enumerate() {
                    if item.len() < MIN_LEARNED_LEN {
                        continue;
                    }
                    let title = if request_text.is_empty() {
                        format!("Discovery: {}", &item[..item.len().min(70)])
                    } else {
                        let preview = &request_text[..request_text.len().min(60)];
                        format!("{} — discovery {}", preview, i + 1)
                    };
                    let topic_key =
                        format!("auto-discovery-{}-{}", slugify(request_text, 40), i + 1);
                    insert_memory(
                        conn,
                        Some(session_id),
                        project,
                        Some(&topic_key),
                        &title,
                        item,
                        "discovery",
                        None,
                    )?;
                    count += 1;
                }
            } else {
                let title = if request_text.is_empty() {
                    "Session insights".to_string()
                } else {
                    let preview = &request_text[..request_text.len().min(80)];
                    format!("{} — learned", preview)
                };
                let content = if request_text.is_empty() {
                    text.to_string()
                } else {
                    format!("**Request**: {}\n\n**Learned**: {}", request_text, text)
                };
                let topic_key = format!("auto-discovery-{}", slugify(request_text, 50));
                insert_memory(
                    conn,
                    Some(session_id),
                    project,
                    Some(&topic_key),
                    &title,
                    &content,
                    "discovery",
                    None,
                )?;
                count += 1;
            }
        }
    }

    if let Some(text) = preferences {
        let text = text.trim();
        if text.len() >= MIN_PREFERENCE_LEN {
            let title = format!("Preference: {}", &text[..text.len().min(60)]);
            let topic_key = format!("auto-preference-{}", slugify(text, 50));
            insert_memory_full(
                conn,
                Some(session_id),
                project,
                Some(&topic_key),
                &title,
                text,
                "preference",
                None,
                None,
                "global",
            )?;
            count += 1;
        }
    }

    if count > 0 {
        crate::log::info(
            "promote",
            &format!(
                "promoted {} memories from summary project={}",
                count, project
            ),
        );
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::tests_helper::setup_memory_schema;

    #[test]
    fn test_split_into_items_bullets() {
        let text = "• Use RwLock for concurrent reads\n• Switch to trigram tokenizer\n• Set compression threshold=100";
        let items = split_into_items(text);
        assert_eq!(items.len(), 3);
        assert!(items[0].contains("RwLock"));
    }

    #[test]
    fn test_split_into_items_dashes() {
        let text = "- First decision about architecture\n- Second decision about testing\n- Third one";
        let items = split_into_items(text);
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn test_split_into_items_single_line() {
        let text = "Switched from unicode61 to trigram tokenizer for better CJK support";
        let items = split_into_items(text);
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn test_split_into_items_semicolons() {
        let text = "Use RwLock for concurrent reads; Switch to trigram tokenizer for CJK; Set compression threshold to 100 observations";
        let items = split_into_items(text);
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn test_promote_multi_decisions() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        setup_memory_schema(&conn);

        let decisions = "• Use RwLock instead of Mutex for concurrent read support\n\
                         • Switch to trigram tokenizer for CJK text search\n\
                         • Set compression threshold to 100 observations";
        let count = promote_summary_to_memories(
            &conn,
            "session-1",
            "test/proj",
            Some("Optimize search and concurrency"),
            Some(decisions),
            None,
            None,
        )
        .unwrap();
        assert_eq!(count, 3);
    }

    #[test]
    fn test_promote_multi_learned() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        setup_memory_schema(&conn);

        let learned = "- FTS5 trigram tokenizer handles CJK without word boundaries\n\
                       - WAL mode allows concurrent reads with single writer";
        let count = promote_summary_to_memories(
            &conn,
            "session-1",
            "test/proj",
            Some("Research storage"),
            None,
            Some(learned),
            None,
        )
        .unwrap();
        assert_eq!(count, 2);
    }
}
