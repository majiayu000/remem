use crate::summarize::parse_summary;

#[test]
fn parse_summary_extracts_fields() {
    let xml = r#"
<summary>
<request>Fix search</request>
<completed>Improved ranking</completed>
<decisions>Use trigram tokenizer</decisions>
<learned>FTS5 handles CJK</learned>
<next_steps>Add benchmarks</next_steps>
<preferences>Prefer concise output</preferences>
</summary>
"#;
    let parsed = parse_summary(xml).expect("summary should parse");
    assert_eq!(parsed.request.as_deref(), Some("Fix search"));
    assert_eq!(parsed.completed.as_deref(), Some("Improved ranking"));
    assert_eq!(parsed.decisions.as_deref(), Some("Use trigram tokenizer"));
    assert_eq!(parsed.learned.as_deref(), Some("FTS5 handles CJK"));
    assert_eq!(parsed.next_steps.as_deref(), Some("Add benchmarks"));
    assert_eq!(parsed.preferences.as_deref(), Some("Prefer concise output"));
}

#[test]
fn parse_summary_returns_none_for_skip_marker() {
    assert!(parse_summary("<skip_summary />").is_none());
}

#[test]
fn parse_summary_returns_some_for_empty_summary_block() {
    // An empty <summary></summary> must not be treated as a skip.
    // finalize_summarize still needs to run to record cooldown/duplicate metadata.
    let parsed = parse_summary("<summary></summary>");
    assert!(
        parsed.is_some(),
        "empty summary block should produce Some, not None"
    );
    let parsed = parsed.unwrap();
    assert!(parsed.request.is_none());
    assert!(parsed.completed.is_none());
}

#[test]
fn parse_summary_returns_none_for_truncated_summary_block() {
    // A truncated response with no </summary> must return None, not Some with empty fields.
    assert!(parse_summary("<summary>truncated without closing tag").is_none());
}
