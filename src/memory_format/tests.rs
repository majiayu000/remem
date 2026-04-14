use super::*;

#[test]
fn parse_observations_title_case_tags() {
    let xml = r#"
<Observation>
  <type>decision</type>
  <title>Title case tag</title>
</Observation>
"#;
    let parsed = parse_observations(xml);
    assert_eq!(
        parsed.len(),
        1,
        "Observation with title-case tags must be parsed"
    );
    assert_eq!(parsed[0].title.as_deref(), Some("Title case tag"));
}

#[test]
fn parse_observations_upper_case_tags() {
    let xml = r#"
<OBSERVATION>
  <type>decision</type>
  <title>Upper case tag</title>
</OBSERVATION>
"#;
    let parsed = parse_observations(xml);
    assert_eq!(
        parsed.len(),
        1,
        "OBSERVATION with all-caps tags must be parsed"
    );
    assert_eq!(parsed[0].title.as_deref(), Some("Upper case tag"));
}

#[test]
fn parse_observations_mixed_case_open_and_close() {
    let xml = r#"<Observation><type>bugfix</type><title>Mixed close</title></observation>"#;
    let parsed = parse_observations(xml);
    assert_eq!(parsed.len(), 1, "Mixed-case open/close tags must be parsed");
    assert_eq!(parsed[0].obs_type, "bugfix");
}

#[test]
fn parse_observations_multiple_mixed_case() {
    let xml = r#"
<OBSERVATION>
  <type>decision</type>
  <title>First</title>
</OBSERVATION>
<observation>
  <type>bugfix</type>
  <title>Second</title>
</observation>
<Observation>
  <type>discovery</type>
  <title>Third</title>
</Observation>
"#;
    let parsed = parse_observations(xml);
    assert_eq!(
        parsed.len(),
        3,
        "All three mixed-case observation blocks must be parsed"
    );
    assert_eq!(parsed[0].title.as_deref(), Some("First"));
    assert_eq!(parsed[1].title.as_deref(), Some("Second"));
    assert_eq!(parsed[2].title.as_deref(), Some("Third"));
}

#[test]
fn extract_field_scans_from_open_tag() {
    let body = "</title><title>ok</title>";
    assert_eq!(extract_field(body, "title").as_deref(), Some("ok"));
}

#[test]
fn extract_field_accepts_tag_attributes() {
    let body = r#"<title source="llm" score="0.9">ok</title>"#;
    assert_eq!(extract_field(body, "title").as_deref(), Some("ok"));
}

#[test]
fn xml_escape_escapes_angle_and_amp() {
    assert_eq!(xml_escape_text(r#"a<&>"'"#), "a&lt;&amp;&gt;&quot;&apos;");
}

#[test]
fn parse_observations_defaults_invalid_type_and_filters_type_concept() {
    let xml = r#"
<observation>
  <type>unknown</type>
  <title>  Planning note  </title>
  <facts>
    <fact>first</fact>
  </facts>
  <concepts>
    <concept>discovery</concept>
    <concept>rust</concept>
  </concepts>
  <files_read>
    <file>src/lib.rs</file>
  </files_read>
  <files_modified>
    <file>src/main.rs</file>
  </files_modified>
</observation>
"#;

    let parsed = parse_observations(xml);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].obs_type, "discovery");
    assert_eq!(parsed[0].title.as_deref(), Some("Planning note"));
    assert_eq!(parsed[0].facts, vec!["first"]);
    assert_eq!(parsed[0].concepts, vec!["rust"]);
    assert_eq!(parsed[0].files_read, vec!["src/lib.rs"]);
    assert_eq!(parsed[0].files_modified, vec!["src/main.rs"]);
}

#[test]
fn parse_observations_tolerates_tag_attributes() {
    let xml = r#"
<observation source="hook">
  <type confidence="0.8">decision</type>
  <title lang="en">  Keep parser tolerant  </title>
  <facts quality="high">
    <fact order="1">first</fact>
  </facts>
  <concepts source="entity">
    <concept score="0.7">parser</concept>
  </concepts>
  <files_read>
    <file path="src/summarize/parse.rs">src/summarize/parse.rs</file>
  </files_read>
</observation>
"#;

    let parsed = parse_observations(xml);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].obs_type, "decision");
    assert_eq!(parsed[0].title.as_deref(), Some("Keep parser tolerant"));
    assert_eq!(parsed[0].facts, vec!["first"]);
    assert_eq!(parsed[0].concepts, vec!["parser"]);
    assert_eq!(parsed[0].files_read, vec!["src/summarize/parse.rs"]);
}

#[test]
fn parse_observations_tolerates_missing_field_close_tag() {
    let xml = r#"
<observation>
  <type>discovery</type>
  <title>Recover malformed title<narrative>Narrative remains parseable</narrative>
</observation>
"#;

    let parsed = parse_observations(xml);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].title.as_deref(), Some("Recover malformed title"));
    assert_eq!(
        parsed[0].narrative.as_deref(),
        Some("Narrative remains parseable")
    );
}

#[test]
fn parse_observations_rejects_plural_tag_as_observation() {
    // <observations> (plural) must NOT be treated as a real <observation> block.
    // Without the tag-name boundary check the outer container would be parsed as
    // an observation, consuming content up to the first </observation> and
    // potentially mangling or duplicating the real inner item.
    let xml = r#"
<observations>
  <observation>
    <type>decision</type>
    <title>Inner item</title>
  </observation>
</observations>
"#;
    let parsed = parse_observations(xml);
    assert_eq!(
        parsed.len(),
        1,
        "only the inner <observation> must be parsed, not the <observations> wrapper"
    );
    assert_eq!(parsed[0].title.as_deref(), Some("Inner item"));
}

#[test]
fn parse_observations_tolerates_missing_observation_close_tag() {
    let xml = r#"
<observation>
  <type>decision</type>
  <title>Recover truncated observation wrapper</title>
  <narrative>Should still capture final block even without close tag</narrative>
"#;

    let parsed = parse_observations(xml);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].obs_type, "decision");
    assert_eq!(
        parsed[0].title.as_deref(),
        Some("Recover truncated observation wrapper")
    );
}
