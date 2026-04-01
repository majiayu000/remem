use std::fs;

use crate::summarize::input::extract_last_assistant_message;

#[test]
fn extract_last_assistant_message_skips_malformed_lines() {
    let path = std::env::temp_dir().join(format!(
        "remem-summarize-transcript-{}-{}.jsonl",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let transcript = concat!(
        "not-json\n",
        "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"ignore\"}]}}\n",
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"first\"}]}}\n",
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"text\":\"skip\"},{\"type\":\"text\",\"text\":\"second\"}]}}\n"
    );
    fs::write(&path, transcript).expect("transcript should be written");

    let extracted = extract_last_assistant_message(path.to_str().expect("utf8 path"));
    assert_eq!(extracted.as_deref(), Some("second"));

    let _ = fs::remove_file(path);
}
