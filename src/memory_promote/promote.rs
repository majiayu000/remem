use anyhow::Result;
use rusqlite::Connection;

use crate::memory::{insert_memory, insert_memory_full};

use super::format::{
    build_content, build_item_title, build_title, split_into_items, MIN_DECISION_LEN,
};
use super::slug::content_hash;

const MIN_LEARNED_LEN: usize = 30;
const MIN_PREFERENCE_LEN: usize = 10;

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
        count += promote_standard_items(
            conn,
            session_id,
            project,
            request_text,
            text,
            MIN_DECISION_LEN,
            "decision",
            "decisions",
            "decision",
            "decision",
        )?;
    }

    if let Some(text) = learned {
        count += promote_standard_items(
            conn,
            session_id,
            project,
            request_text,
            text,
            MIN_LEARNED_LEN,
            "learned",
            "learned",
            "discovery",
            "discovery",
        )?;
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

fn promote_standard_items(
    conn: &Connection,
    session_id: &str,
    project: &str,
    request_text: &str,
    text: &str,
    min_len: usize,
    item_label: &str,
    single_label: &str,
    topic_prefix: &str,
    memory_type: &str,
) -> Result<usize> {
    let text = text.trim();
    if text.len() < min_len {
        return Ok(0);
    }

    let items = split_into_items(text);
    if items.len() > 1 {
        let mut count = 0;
        for (index, item) in items.iter().enumerate() {
            if item.len() < min_len {
                continue;
            }
            let title = build_item_title(item, item_label, index);
            let content = build_content(item, request_text);
            let topic_key = format!("{}-{}", topic_prefix, content_hash(item));
            insert_memory(
                conn,
                Some(session_id),
                project,
                Some(&topic_key),
                &title,
                &content,
                memory_type,
                None,
            )?;
            count += 1;
        }
        Ok(count)
    } else {
        let title = build_title(text, request_text, single_label);
        let content = build_content(text, request_text);
        let topic_key = format!("{}-{}", topic_prefix, content_hash(text));
        insert_memory(
            conn,
            Some(session_id),
            project,
            Some(&topic_key),
            &title,
            &content,
            memory_type,
            None,
        )?;
        Ok(1)
    }
}
