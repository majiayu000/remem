use crate::memory::Memory;

pub(in crate::context) fn is_memory_self_diagnostic(memory: &Memory) -> bool {
    let haystack = format!("{} {}", memory.title, memory.text).to_ascii_lowercase();
    is_self_diagnostic_text(&haystack)
}

pub(in crate::context) fn is_self_diagnostic_text(text: &str) -> bool {
    let haystack = text.to_ascii_lowercase();
    [
        "sessionstart",
        "memory injection",
        "memories loaded",
        "remem context",
        "loaded memories",
    ]
    .iter()
    .any(|needle| haystack.contains(needle))
}
