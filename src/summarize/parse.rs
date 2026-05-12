use crate::memory::format;

pub struct ParsedSummary {
    pub request: Option<String>,
    pub completed: Option<String>,
    pub decisions: Option<String>,
    pub learned: Option<String>,
    pub next_steps: Option<String>,
    pub preferences: Option<String>,
    pub workstream: Option<String>,
    pub workstream_progress: Option<String>,
    pub workstream_next: Option<String>,
    pub workstream_blockers: Option<String>,
}

pub fn parse_summary(text: &str) -> Option<ParsedSummary> {
    if text.contains("<skip_summary") {
        return None;
    }

    // Return None when <summary> is absent or malformed (missing closing tag).
    // An empty <summary></summary> must still produce Some so that
    // finalize_summarize records cooldown/duplicate metadata correctly.
    if !text.contains("<summary>") || !text.contains("</summary>") {
        return None;
    }
    let content = format::extract_field(text, "summary").unwrap_or_default();

    Some(ParsedSummary {
        request: format::extract_field(&content, "request"),
        completed: format::extract_field(&content, "completed"),
        decisions: format::extract_field(&content, "decisions"),
        learned: format::extract_field(&content, "learned"),
        next_steps: format::extract_field(&content, "next_steps"),
        preferences: format::extract_field(&content, "preferences"),
        workstream: format::extract_field(&content, "workstream"),
        workstream_progress: format::extract_field(&content, "workstream_progress"),
        workstream_next: format::extract_field(&content, "workstream_next"),
        workstream_blockers: format::extract_field(&content, "workstream_blockers"),
    })
}
