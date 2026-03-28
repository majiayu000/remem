use anyhow::Result;
use rusqlite::Connection;

use crate::memory::{insert_memory, insert_memory_full};

/// Minimum content length to be worth promoting.
const MIN_DECISION_LEN: usize = 30;
const MIN_LEARNED_LEN: usize = 30;
const MIN_PREFERENCE_LEN: usize = 10;

/// Max title length — leaves room for FTS matching.
const MAX_TITLE_LEN: usize = 120;

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

/// Generate a stable content-based hash for topic_key dedup.
/// Uses first 200 chars of content to produce a short hex prefix,
/// so the same decision across different sessions gets the same key.
fn content_hash(text: &str) -> String {
    use std::hash::{Hash, Hasher};
    let normalized: String = text
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ')
        .collect();
    let trimmed = if normalized.len() > 200 {
        &normalized[..200]
    } else {
        &normalized
    };
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    trimmed.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Build a keyword-rich title from the content itself.
/// Falls back to request prefix only if content is too short.
fn build_title(content: &str, request: &str, label: &str) -> String {
    let source = if content.len() >= 20 {
        content
    } else {
        request
    };
    if source.is_empty() {
        return format!("Session {label}");
    }
    let truncated = truncate_at_boundary(source, MAX_TITLE_LEN - label.len() - 5);
    format!("{truncated} — {label}")
}

/// Build a keyword-rich title for a single item in a multi-item list.
fn build_item_title(item: &str, label: &str, _index: usize) -> String {
    let truncated = truncate_at_boundary(item, MAX_TITLE_LEN - label.len() - 5);
    format!("{truncated} — {label}")
}

/// Truncate text at a word or sentence boundary, preserving keywords.
fn truncate_at_boundary(text: &str, max_len: usize) -> String {
    let text = text.trim();
    if text.len() <= max_len {
        return text.to_string();
    }
    let slice = &text[..max_len];
    for sep in ['。', '；', ';', '.', '，', ','] {
        if let Some(pos) = slice.rfind(sep) {
            if pos > max_len / 2 {
                return text[..pos + sep.len_utf8()].trim().to_string();
            }
        }
    }
    if let Some(pos) = slice.rfind(' ') {
        if pos > max_len / 2 {
            return text[..pos].to_string();
        }
    }
    text.chars().take(max_len).collect()
}

/// Build content with request as lightweight context.
fn build_content(body: &str, request: &str) -> String {
    if request.is_empty() {
        body.to_string()
    } else {
        format!(
            "[Context: {}]\n\n{}",
            truncate_at_boundary(request, 150),
            body
        )
    }
}

/// Split a multi-line text block into individual items.
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
                .trim_start_matches(['•', '-', '*', '·'])
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
///
/// topic_key is now based on content hash (not request prefix),
/// so the same decision across different sessions deduplicates correctly.
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
                    let title = build_item_title(item, "decision", i);
                    let content = build_content(item, request_text);
                    // Content-based hash: same decision text → same topic_key
                    let topic_key = format!("decision-{}", content_hash(item));
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
            } else {
                let title = build_title(text, request_text, "decisions");
                let content = build_content(text, request_text);
                let topic_key = format!("decision-{}", content_hash(text));
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
                    let title = build_item_title(item, "learned", i);
                    let content = build_content(item, request_text);
                    let topic_key = format!("discovery-{}", content_hash(item));
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
            } else {
                let title = build_title(text, request_text, "learned");
                let content = build_content(text, request_text);
                let topic_key = format!("discovery-{}", content_hash(text));
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
            let title = build_title(text, "", "preference");
            let topic_key = format!("preference-{}", content_hash(text));
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
                None,
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
        let text =
            "- First decision about architecture\n- Second decision about testing\n- Third one";
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
    fn test_build_title_from_content() {
        let title = build_title(
            "Use RwLock instead of Mutex for concurrent read support",
            "Optimize search and concurrency",
            "decision",
        );
        assert!(title.contains("RwLock"));
        assert!(title.contains("— decision"));
    }

    #[test]
    fn test_build_title_fallback_to_request() {
        let title = build_title("short", "Optimize search and concurrency", "decision");
        assert!(title.contains("Optimize"));
    }

    #[test]
    fn test_build_content_no_boilerplate() {
        let content = build_content(
            "Use RwLock instead of Mutex for concurrent read support",
            "Optimize search",
        );
        assert!(!content.contains("**Request**"));
        assert!(!content.contains("**Decisions**"));
        assert!(content.contains("[Context:"));
        assert!(content.contains("RwLock"));
    }

    #[test]
    fn test_truncate_at_boundary() {
        let text = "Use RwLock instead of Mutex for concurrent read support in the database layer";
        let truncated = truncate_at_boundary(text, 40);
        assert!(truncated.len() <= 45);
        assert!(!truncated.ends_with(' '));
    }

    #[test]
    fn test_truncate_cjk() {
        let text = "使用 RwLock 替代 Mutex 实现并发读支持。数据库层需要高并发";
        let truncated = truncate_at_boundary(text, 30);
        assert!(truncated.contains("。") || truncated.len() <= 35);
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

        let memories = crate::memory::get_recent_memories(&conn, "test/proj", 10).unwrap();
        let titles: Vec<&str> = memories.iter().map(|m| m.title.as_str()).collect();
        assert!(
            titles.iter().any(|t| t.contains("RwLock")),
            "title should contain keyword from content: {:?}",
            titles
        );
        assert!(
            titles.iter().any(|t| t.contains("trigram")),
            "title should contain keyword from content: {:?}",
            titles
        );
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

    #[test]
    fn test_promote_content_format() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        setup_memory_schema(&conn);

        let decisions = "Switched from unicode61 to trigram tokenizer for better CJK support";
        promote_summary_to_memories(
            &conn,
            "session-1",
            "test/proj",
            Some("Fix search"),
            Some(decisions),
            None,
            None,
        )
        .unwrap();

        let memories = crate::memory::get_recent_memories(&conn, "test/proj", 10).unwrap();
        assert_eq!(memories.len(), 1);
        assert!(
            !memories[0].text.contains("**Request**"),
            "content should not have boilerplate: {}",
            memories[0].text
        );
        assert!(
            memories[0].text.contains("[Context:"),
            "content should have compact context: {}",
            memories[0].text
        );
    }

    #[test]
    fn test_content_hash_dedup() {
        // Same decision text from different requests should get the same topic_key
        let hash1 = content_hash("Use FTS5 trigram tokenizer for CJK support");
        let hash2 = content_hash("Use FTS5 trigram tokenizer for CJK support");
        assert_eq!(hash1, hash2);

        // Different content should get different keys
        let hash3 = content_hash("Switch to WAL mode for concurrent reads");
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_cross_session_dedup() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        setup_memory_schema(&conn);

        let decision = "Use FTS5 trigram tokenizer for CJK text search support";

        // Session 1: different request text
        promote_summary_to_memories(
            &conn,
            "session-1",
            "test/proj",
            Some("Optimize search"),
            Some(decision),
            None,
            None,
        )
        .unwrap();

        // Session 2: same decision, different request
        promote_summary_to_memories(
            &conn,
            "session-2",
            "test/proj",
            Some("Database performance tuning"),
            Some(decision),
            None,
            None,
        )
        .unwrap();

        // Should have only 1 memory (upserted, not duplicated)
        let memories = crate::memory::get_recent_memories(&conn, "test/proj", 10).unwrap();
        assert_eq!(
            memories.len(),
            1,
            "same decision should dedup across sessions"
        );
    }
}
