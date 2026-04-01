pub fn extract_field(content: &str, field: &str) -> Option<String> {
    let open = format!("<{}>", field);
    let close = format!("</{}>", field);
    let start = content.find(&open)? + open.len();
    let end_rel = content[start..].find(&close)?;
    let end = start + end_rel;
    if start >= end {
        return None;
    }
    let value = content[start..end].trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}
