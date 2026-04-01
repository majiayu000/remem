pub(super) fn parse_native_memory_frontmatter(content: &str) -> (String, String, &str) {
    let default_title = "Untitled memory".to_string();
    let default_type = "discovery".to_string();

    if !content.starts_with("---") {
        return (default_title, default_type, content);
    }

    let after_first = &content[3..];
    let Some(end_pos) = after_first.find("\n---") else {
        return (default_title, default_type, content);
    };

    let frontmatter = &after_first[..end_pos];
    let body_start = 3 + end_pos + 4;
    let body = if body_start < content.len() {
        &content[body_start..]
    } else {
        ""
    };

    let mut name = None;
    let mut mem_type = None;

    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("name:") {
            name = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("type:") {
            let raw = value.trim();
            mem_type = Some(match raw {
                "user" | "feedback" => "preference".to_string(),
                "project" | "reference" => "discovery".to_string(),
                other => other.to_string(),
            });
        }
    }

    (
        name.unwrap_or(default_title),
        mem_type.unwrap_or(default_type),
        body,
    )
}
