pub fn slugify_for_topic(text: &str, max_len: usize) -> String {
    slugify(text, max_len)
}

fn slugify(text: &str, max_len: usize) -> String {
    let slug: String = text
        .to_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch
            } else if ch == '-' || ch == '_' || ch == ' ' {
                '-'
            } else if !ch.is_ascii() {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let mut result = String::with_capacity(slug.len());
    let mut last_dash = false;
    for ch in slug.chars() {
        if ch == '-' {
            if !last_dash && !result.is_empty() {
                result.push('-');
            }
            last_dash = true;
        } else {
            result.push(ch);
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

pub(crate) fn content_hash(text: &str) -> String {
    use std::hash::{Hash, Hasher};

    let normalized: String = text
        .to_lowercase()
        .chars()
        .filter(|ch| ch.is_alphanumeric() || *ch == ' ')
        .collect();
    let trimmed = crate::db::truncate_str(&normalized, 200);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    trimmed.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}
