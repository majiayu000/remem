use std::collections::HashSet;

use serde_json::Value;

use super::{build_schema, OutputSchema};

const JSON_OUTPUTS: [OutputSchema; 13] = [
    OutputSchema::CurrentState,
    OutputSchema::Search,
    OutputSchema::RecallUserContext,
    OutputSchema::Timeline,
    OutputSchema::GetObservations,
    OutputSchema::LookupCommit,
    OutputSchema::CommitsForSession,
    OutputSchema::SaveMemory,
    OutputSchema::GovernMemory,
    OutputSchema::Workstreams,
    OutputSchema::UpdateWorkstream,
    OutputSchema::SearchRaw,
    OutputSchema::ListRawSessions,
];

fn schema_value(kind: OutputSchema) -> anyhow::Result<Value> {
    Ok(Value::Object(build_schema(kind)?.as_ref().clone()))
}

fn assert_no_nullable(value: &Value, path: &str) {
    match value {
        Value::Object(object) => {
            assert!(
                !object.contains_key("nullable"),
                "non-standard nullable at {path}: {value}"
            );
            for (key, child) in object {
                assert_no_nullable(child, &format!("{path}.{key}"));
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                assert_no_nullable(child, &format!("{path}[{index}]"));
            }
        }
        _ => {}
    }
}

fn resolve_local_ref<'a>(root: &'a Value, schema: &'a Value) -> &'a Value {
    schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|reference| reference.strip_prefix('#'))
        .and_then(|pointer| root.pointer(pointer))
        .unwrap_or(schema)
}

fn explicitly_allows_null(root: &Value, schema: &Value) -> bool {
    let schema = resolve_local_ref(root, schema);
    match schema.get("type") {
        Some(Value::String(value)) if value == "null" => return true,
        Some(Value::Array(values)) if values.iter().any(|value| value.as_str() == Some("null")) => {
            return true;
        }
        _ => {}
    }
    ["anyOf", "oneOf"]
        .into_iter()
        .filter_map(|keyword| schema.get(keyword).and_then(Value::as_array))
        .flatten()
        .any(|candidate| explicitly_allows_null(root, candidate))
}

fn has_property(root: &Value, schema: &Value, property: &str) -> bool {
    fn visit(
        root: &Value,
        schema: &Value,
        property: &str,
        visited_refs: &mut HashSet<String>,
    ) -> bool {
        if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
            if !visited_refs.insert(reference.to_string()) {
                return false;
            }
            if let Some(pointer) = reference.strip_prefix('#') {
                if let Some(target) = root.pointer(pointer) {
                    return visit(root, target, property, visited_refs);
                }
            }
        }
        if schema
            .get("properties")
            .and_then(Value::as_object)
            .is_some_and(|properties| properties.contains_key(property))
        {
            return true;
        }
        if schema
            .get("items")
            .is_some_and(|items| visit(root, items, property, visited_refs))
        {
            return true;
        }
        ["allOf", "anyOf", "oneOf"]
            .into_iter()
            .filter_map(|keyword| schema.get(keyword).and_then(Value::as_array))
            .flatten()
            .any(|candidate| visit(root, candidate, property, visited_refs))
    }

    visit(root, schema, property, &mut HashSet::new())
}

fn property<'a>(schema: &'a Value, name: &str) -> &'a Value {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .and_then(|properties| properties.get(name))
        .unwrap_or_else(|| panic!("schema should contain root property {name}"))
}

fn array_items<'a>(root: &'a Value, schema: &'a Value) -> Option<&'a Value> {
    let schema = resolve_local_ref(root, schema);
    if let Some(items) = schema.get("items") {
        return Some(items);
    }
    ["anyOf", "oneOf"]
        .into_iter()
        .filter_map(|keyword| schema.get(keyword).and_then(Value::as_array))
        .flatten()
        .find_map(|candidate| array_items(root, candidate))
}

#[test]
fn all_published_output_schemas_use_draft_2020_nullable_unions() -> anyhow::Result<()> {
    for kind in JSON_OUTPUTS {
        let schema = schema_value(kind)?;
        assert_no_nullable(&schema, &format!("{kind:?}"));
    }
    Ok(())
}

#[test]
fn common_optional_shapes_explicitly_accept_null() -> anyhow::Result<()> {
    let raw = schema_value(OutputSchema::SearchRaw)?;
    assert!(explicitly_allows_null(&raw, property(&raw, "project")));

    let current = schema_value(OutputSchema::CurrentState)?;
    assert!(explicitly_allows_null(
        &current,
        property(&current, "state")
    ));

    let govern = schema_value(OutputSchema::GovernMemory)?;
    assert!(explicitly_allows_null(&govern, property(&govern, "reason")));
    let required = govern["required"]
        .as_array()
        .expect("govern required must be an array");
    assert_eq!(
        required
            .iter()
            .filter(|value| value.as_str() == Some("reason"))
            .count(),
        1
    );
    Ok(())
}

#[test]
fn production_null_paths_are_valid_in_every_nullable_output_family() -> anyhow::Result<()> {
    let search = schema_value(OutputSchema::Search)?;
    let pagination = resolve_local_ref(&search, property(&search, "pagination"));
    assert!(explicitly_allows_null(
        &search,
        property(pagination, "next_offset")
    ));

    let recall = schema_value(OutputSchema::RecallUserContext)?;
    assert!(explicitly_allows_null(
        &recall,
        property(&recall, "usage_policy")
    ));

    let timeline = schema_value(OutputSchema::Timeline)?;
    let observation = array_items(&timeline, property(&timeline, "observations"))
        .map(|items| resolve_local_ref(&timeline, items))
        .expect("timeline must declare observation items");
    assert!(explicitly_allows_null(
        &timeline,
        property(observation, "title")
    ));

    let lookup = schema_value(OutputSchema::LookupCommit)?;
    let commit = array_items(&lookup, property(&lookup, "commits"))
        .map(|items| resolve_local_ref(&lookup, items))
        .expect("lookup_commit must declare commit items");
    let git = resolve_local_ref(&lookup, property(commit, "git"));
    assert!(explicitly_allows_null(&lookup, property(git, "branch")));

    let save = schema_value(OutputSchema::SaveMemory)?;
    assert!(explicitly_allows_null(&save, property(&save, "claim_id")));
    let local_copy = resolve_local_ref(&save, property(&save, "local_copy"));
    assert!(explicitly_allows_null(&save, property(local_copy, "path")));

    let workstreams = schema_value(OutputSchema::Workstreams)?;
    let workstream = array_items(&workstreams, property(&workstreams, "workstreams"))
        .map(|items| resolve_local_ref(&workstreams, items))
        .expect("workstreams must declare workstream items");
    assert!(explicitly_allows_null(
        &workstreams,
        property(workstream, "description")
    ));

    let raw_sessions = schema_value(OutputSchema::ListRawSessions)?;
    assert!(explicitly_allows_null(
        &raw_sessions,
        property(&raw_sessions, "since_epoch")
    ));
    Ok(())
}

#[test]
fn detail_items_are_a_typed_memory_or_observation_union() -> anyhow::Result<()> {
    let schema = schema_value(OutputSchema::GetObservations)?;
    let items = array_items(&schema, property(&schema, "details"))
        .expect("details must declare array items");
    let union = resolve_local_ref(&schema, items);
    let alternatives = union
        .get("anyOf")
        .or_else(|| union.get("oneOf"))
        .and_then(Value::as_array)
        .expect("detail items must declare a union");
    assert_eq!(alternatives.len(), 2);

    assert!(alternatives.iter().any(|candidate| {
        has_property(&schema, candidate, "memory_type")
            && has_property(&schema, candidate, "temporal_facts")
            && has_property(&schema, candidate, "topic_trace")
    }));
    assert!(alternatives.iter().any(|candidate| {
        has_property(&schema, candidate, "memory_session_id")
            && has_property(&schema, candidate, "compressed_sources")
    }));
    Ok(())
}

#[test]
fn current_state_and_search_use_typed_nested_contracts() -> anyhow::Result<()> {
    let current = schema_value(OutputSchema::CurrentState)?;
    assert!(has_property(
        &current,
        property(&current, "state"),
        "owner_scope"
    ));
    assert!(has_property(
        &current,
        property(&current, "current"),
        "staleness"
    ));
    assert!(has_property(
        &current,
        property(&current, "why"),
        "edge_type"
    ));

    let search = schema_value(OutputSchema::Search)?;
    let results = array_items(&search, property(&search, "results"))
        .expect("search results must declare array items");
    let result = resolve_local_ref(&search, results);
    let staleness = result["properties"]
        .get("staleness")
        .expect("search result must declare staleness");
    assert!(has_property(&search, staleness, "source_anchor"));
    Ok(())
}
