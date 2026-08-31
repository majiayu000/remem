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

#[test]
fn adversarial_policy_v2_schema_requires_provenance_and_artifact_hashes() -> serde_json::Result<()>
{
    let schema: Value = serde_json::from_str(include_str!(
        "../../../../eval/public/schemas/memory-run.schema.json"
    ))?;
    let required = schema["allOf"][0]["then"]["required"]
        .as_array()
        .expect("v2 conditional required fields");
    for field in ["artifact_sha256", "suite_content_identity"] {
        assert!(required.iter().any(|value| value == field), "{field}");
    }
    let required_artifacts = [
        "reader_input",
        "retrieved_evidence",
        "answer",
        "score",
        "diagnosis",
        "remem_db_snapshot",
    ];
    let artifacts = &schema["allOf"][0]["then"]["properties"]["artifacts"];
    let artifact_required = artifacts["required"]
        .as_array()
        .expect("v2 required artifact paths");
    let artifact_hashes = &schema["allOf"][0]["then"]["properties"]["artifact_sha256"];
    let artifact_hash_required = artifact_hashes["required"]
        .as_array()
        .expect("v2 required artifact hashes");
    assert_eq!(artifact_hashes["minProperties"], required_artifacts.len());
    for field in required_artifacts {
        assert!(
            artifact_required.iter().any(|value| value == field),
            "{field}"
        );
        assert!(
            artifact_hash_required.iter().any(|value| value == field),
            "{field} hash"
        );
    }
    let environment = &schema["allOf"][0]["then"]["properties"]["environment"];
    let environment_required = environment["required"]
        .as_array()
        .expect("v2 environment required fields");
    for field in ["os", "arch", "source_dirty", "production_input_tree_sha256"] {
        assert!(
            environment_required.iter().any(|value| value == field),
            "{field}"
        );
    }
    assert_eq!(environment["properties"]["source_dirty"]["const"], false);
    assert_eq!(
        environment["properties"]["production_input_tree_sha256"]["type"],
        "string"
    );
    assert_eq!(
        environment["properties"]["production_input_tree_sha256"]["pattern"],
        "^[0-9a-f]{64}$"
    );
    Ok(())
}

#[test]
fn duplicate_coding_claim_contract_is_removed_from_runtime() {
    let duplicate_name = ["coding-claim", "-contract-v1.json"].concat();
    assert!(!std::path::Path::new("eval/public/claims")
        .join(&duplicate_name)
        .exists());
    assert!(
        !include_str!("../../../../scripts/ci/check_public_claims.py").contains(&duplicate_name)
    );
}
