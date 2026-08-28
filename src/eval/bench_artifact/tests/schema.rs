use serde_json::Value;

#[test]
fn memory_run_schema_has_one_strict_suite_content_identity_definition() -> serde_json::Result<()> {
    let raw = include_str!("../../../../eval/public/schemas/memory-run.schema.json");
    assert_eq!(raw.matches("\"suite_content_identity\": {").count(), 1);

    let schema: Value = serde_json::from_str(raw)?;
    let definition = &schema["properties"]["suite_content_identity"];
    assert_eq!(definition["type"], "string");
    assert_eq!(definition["pattern"], "^sha256-raw-suite-v1:[0-9a-f]{64}$");
    Ok(())
}
