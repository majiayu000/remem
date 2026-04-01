use super::*;

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
