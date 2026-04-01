pub(crate) const MIN_DECISION_LEN: usize = 30;
const MAX_TITLE_LEN: usize = 120;

pub(crate) fn build_title(content: &str, request: &str, label: &str) -> String {
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

pub(crate) fn build_item_title(item: &str, label: &str, _index: usize) -> String {
    let truncated = truncate_at_boundary(item, MAX_TITLE_LEN - label.len() - 5);
    format!("{truncated} — {label}")
}

pub(crate) fn truncate_at_boundary(text: &str, max_len: usize) -> String {
    let text = text.trim();
    if text.len() <= max_len {
        return text.to_string();
    }
    let safe_end = crate::db::truncate_str(text, max_len).len();
    let slice = &text[..safe_end];
    for sep in ['。', '；', ';', '.', '，', ','] {
        if let Some(pos) = slice.rfind(sep) {
            if pos > safe_end / 2 {
                return text[..pos + sep.len_utf8()].trim().to_string();
            }
        }
    }
    if let Some(pos) = slice.rfind(' ') {
        if pos > safe_end / 2 {
            return text[..pos].to_string();
        }
    }
    text.chars().take(max_len).collect()
}

pub(crate) fn build_content(body: &str, request: &str) -> String {
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

pub(crate) fn split_into_items(text: &str) -> Vec<String> {
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
                .map(|ch| ch.is_ascii_digit())
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
                .map(|ch| ch.is_ascii_digit())
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
        let semi_split: Vec<String> = text
            .trim()
            .split('；')
            .flat_map(|segment| segment.split(';'))
            .map(|segment| segment.trim().to_string())
            .filter(|segment| segment.len() >= MIN_DECISION_LEN)
            .collect();
        if semi_split.len() > 1 {
            return semi_split;
        }
    }

    items
}
