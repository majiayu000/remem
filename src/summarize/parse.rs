use crate::memory_format;

pub struct ParsedSummary {
    pub request: Option<String>,
    pub completed: Option<String>,
    pub decisions: Option<String>,
    pub learned: Option<String>,
    pub next_steps: Option<String>,
    pub preferences: Option<String>,
}

pub fn parse_summary(text: &str) -> Option<ParsedSummary> {
    if text.contains("<skip_summary") {
        return None;
    }

    let content = memory_format::extract_field(text, "summary")?;

    Some(ParsedSummary {
        request: memory_format::extract_field(&content, "request"),
        completed: memory_format::extract_field(&content, "completed"),
        decisions: memory_format::extract_field(&content, "decisions"),
        learned: memory_format::extract_field(&content, "learned"),
        next_steps: memory_format::extract_field(&content, "next_steps"),
        preferences: memory_format::extract_field(&content, "preferences"),
    })
}
