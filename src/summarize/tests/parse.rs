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
fn parse_summary_tolerates_missing_summary_close_tag() {
    let xml = r#"
<summary>
<request>Harden summary parsing</request>
<completed>Keep parsing when wrapper is truncated</completed>
"#;
    let parsed = parse_summary(xml).expect("truncated summary wrapper should still parse");
    assert_eq!(parsed.request.as_deref(), Some("Harden summary parsing"));
    assert_eq!(
        parsed.completed.as_deref(),
        Some("Keep parsing when wrapper is truncated")
    );
}

#[test]
fn parse_summary_tolerates_missing_field_close_tag() {
    let xml = r#"
<summary>
<request>Improve robustness<completed>Recover partial output</completed>
</summary>
"#;
    let parsed = parse_summary(xml).expect("malformed field should still parse other content");
    assert_eq!(parsed.request.as_deref(), Some("Improve robustness"));
    assert_eq!(parsed.completed.as_deref(), Some("Recover partial output"));
}

#[test]
fn parse_summary_tolerates_summary_open_tag_attributes() {
    let xml = r#"
<summary source="hook" model="gpt-5.4">
<request>Parse attribute wrapper</request>
<completed>Should not fail on metadata attributes</completed>
</summary>
"#;
    let parsed = parse_summary(xml).expect("summary wrapper with attributes should parse");
    assert_eq!(parsed.request.as_deref(), Some("Parse attribute wrapper"));
    assert_eq!(
        parsed.completed.as_deref(),
        Some("Should not fail on metadata attributes")
    );
}

#[test]
fn parse_summary_tolerates_field_tag_attributes() {
    let xml = r#"
<summary>
<request priority="high">Capture request despite field attrs</request>
<completed source="llm">Capture completion despite field attrs</completed>
</summary>
"#;
    let parsed = parse_summary(xml).expect("field tags with attributes should parse");
    assert_eq!(
        parsed.request.as_deref(),
        Some("Capture request despite field attrs")
    );
    assert_eq!(
        parsed.completed.as_deref(),
        Some("Capture completion despite field attrs")
    );
}

#[test]
fn parse_summary_prefers_last_summary_block() {
    let xml = r#"
<summary>
<request>Example request from scratchpad</request>
<completed>Example completion from scratchpad</completed>
</summary>

<summary>
<request>Final request from actual session</request>
<completed>Final completion from actual session</completed>
</summary>
"#;
    let parsed = parse_summary(xml).expect("multiple summary blocks should parse");
    assert_eq!(
        parsed.request.as_deref(),
        Some("Final request from actual session")
    );
    assert_eq!(
        parsed.completed.as_deref(),
        Some("Final completion from actual session")
    );
}

#[test]
fn parse_summary_keeps_literal_angle_brackets_in_well_formed_fields() {
    let xml = r#"
<summary>
<request>Handle x < y safely</request>
<completed>Preserve literal angle brackets in closed fields</completed>
</summary>
"#;
    let parsed =
        parse_summary(xml).expect("well-formed field with literal angle bracket should parse");
    assert_eq!(parsed.request.as_deref(), Some("Handle x < y safely"));
    assert_eq!(
        parsed.completed.as_deref(),
        Some("Preserve literal angle brackets in closed fields")
    );
}

#[test]
fn parse_summary_ignores_embedded_summary_tag_in_field() {
    // P2 regression: a literal `<summary>` token inside a field must not cause
    // the parser to anchor to that inner token instead of the outer wrapper.
    let xml = r#"
<summary>
<request>Explain the &lt;summary&gt; tag format — use <summary> sparingly</request>
<completed>Documented</completed>
</summary>
"#;
    let parsed = parse_summary(xml).expect("should parse");
    // request must be non-None (truncated at the inner tag is acceptable; None is data loss)
    assert!(
        parsed.request.is_some(),
        "request field must not be lost when outer wrapper anchoring is correct; got None"
    );
    // completed must be captured from the outer wrapper, not lost
    assert_eq!(parsed.completed.as_deref(), Some("Documented"));
}
