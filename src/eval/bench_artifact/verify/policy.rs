use std::path::Path;

use serde_json::Value;

use super::{read_json, read_json_artifact, resolve_public_path, VerifyState};
use crate::eval::bench_artifact::types::ClaimRegistryPolicy;

const REQUIRED_SCHEMA_FILES: [&str; 6] = [
    "schemas/benchmark-manifest.schema.json",
    "schemas/memory-run.schema.json",
    "schemas/coding-run.schema.json",
    "schemas/memory-report.schema.json",
    "schemas/coding-report.schema.json",
    "schemas/reproduction-metadata.schema.json",
];

pub(super) fn load_claim_registry(path: &Path, state: &mut VerifyState) {
    if let Some(registry) =
        read_json_artifact::<ClaimRegistryPolicy>(path, state, "claim registry policy")
    {
        state.verified_artifacts.claim_registry = Some(registry);
    }
}

pub(super) fn validate_required_schemas(state: &mut VerifyState) {
    for relative in REQUIRED_SCHEMA_FILES {
        let Some(path) = resolve_public_path(state, relative, relative) else {
            continue;
        };
        if !path.exists() {
            state.fail(relative.to_string(), "required schema file is missing");
            continue;
        }
        let Some(value) = read_json::<Value>(&path, state, "schema") else {
            continue;
        };
        if value.get("$schema").and_then(Value::as_str).is_none() {
            state.fail(relative.to_string(), "schema is missing $schema");
        }
        if value
            .pointer("/properties/schema_version/const")
            .and_then(Value::as_u64)
            != Some(1)
        {
            state.fail(
                relative.to_string(),
                "schema must pin schema_version const 1",
            );
        }
    }
}
