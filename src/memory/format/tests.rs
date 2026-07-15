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
fn xml_escape_escapes_angle_and_amp() {
    assert_eq!(xml_escape_text(r#"a<&>"'"#), "a&lt;&amp;&gt;&quot;&apos;");
}

#[test]
fn parse_observations_reads_and_clamps_confidence() {
    let xml = r#"
<observation>
  <type>decision</type>
  <title>Valid</title>
  <confidence>0.42</confidence>
</observation>
<observation>
  <type>decision</type>
  <title>Above range</title>
  <confidence>3.5</confidence>
</observation>
<observation>
  <type>decision</type>
  <title>Below range</title>
  <confidence>-0.2</confidence>
</observation>
<observation>
  <type>decision</type>
  <title>Invalid</title>
  <confidence>high</confidence>
</observation>
<observation>
  <type>decision</type>
  <title>Missing</title>
</observation>
"#;
    let parsed = parse_observations(xml);
    assert_eq!(parsed.len(), 5);
    assert_eq!(parsed[0].confidence, Some(0.42));
    assert_eq!(parsed[1].confidence, Some(1.0));
    assert_eq!(parsed[2].confidence, Some(0.0));
    assert_eq!(parsed[3].confidence, None);
    assert_eq!(parsed[4].confidence, None);
}

#[test]
fn parse_observations_drops_missing_and_unknown_types_with_error_reasons() {
    let log_dir = std::env::temp_dir().join(format!(
        "remem-observation-type-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should follow Unix epoch")
            .as_nanos()
    ));
    std::fs::create_dir_all(&log_dir).expect("test log directory should be created");

    let xml = r#"
<observation>
  <type>unknown</type>
  <title>Unknown</title>
</observation>
<observation>
  <title>Missing</title>
</observation>
<observation>
  <type>decision</type>
  <title>Valid</title>
</observation>
"#;

    let parsed = crate::log::with_log_dir(&log_dir, || parse_observations(xml));
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].obs_type, "decision");
    assert_eq!(parsed[0].title.as_deref(), Some("Valid"));

    let log = std::fs::read_to_string(log_dir.join("remem.log"))
        .expect("observation parser error log should be readable");
    assert!(log.contains("[ERROR] [observation-parse]"));
    assert!(log.contains("drop_reason=unknown_type raw_type=\"unknown\""));
    assert!(log.contains("drop_reason=missing_type raw_type=\"\""));

    std::fs::remove_dir_all(log_dir).expect("test log directory should be removed");
}

#[test]
fn parse_observations_preserves_legal_types_and_filters_type_concept() {
    for observation_type in OBSERVATION_TYPES {
        let xml = format!(
            r#"
<observation>
  <type>{observation_type}</type>
  <title>  Planning note  </title>
  <facts>
    <fact>first</fact>
  </facts>
  <concepts>
    <concept>{observation_type}</concept>
    <concept>rust</concept>
  </concepts>
  <files_read>
    <file>src/lib.rs</file>
  </files_read>
  <files_modified>
    <file>src/main.rs</file>
  </files_modified>
</observation>
"#
        );

        let parsed = parse_observations(&xml);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].obs_type, *observation_type);
        assert_eq!(parsed[0].title.as_deref(), Some("Planning note"));
        assert_eq!(parsed[0].facts, vec!["first"]);
        assert_eq!(parsed[0].concepts, vec!["rust"]);
        assert_eq!(parsed[0].files_read, vec!["src/lib.rs"]);
        assert_eq!(parsed[0].files_modified, vec!["src/main.rs"]);
    }
}
