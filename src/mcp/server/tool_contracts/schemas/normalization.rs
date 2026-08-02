use anyhow::{bail, Context};
use rmcp::model::JsonObject;
use serde_json::{json, Map, Value};

/// Convert schemars' OpenAPI-style `nullable` extension into Draft 2020-12.
pub(super) fn normalize_nullable(mut schema: JsonObject) -> anyhow::Result<JsonObject> {
    normalize_object(&mut schema, "$".to_string())?;
    Ok(schema)
}

/// Reject undeclared fields for every typed object emitted by the output DTOs.
/// Unconstrained `serde_json::Value` extension points have no declared
/// `properties`, so they intentionally remain open.
pub(super) fn close_declared_objects(mut schema: JsonObject) -> anyhow::Result<JsonObject> {
    close_object(&mut schema, "$".to_string())?;
    Ok(schema)
}

fn close_value(value: &mut Value, path: String) -> anyhow::Result<()> {
    match value {
        Value::Object(object) => close_object(object, path),
        Value::Array(items) => {
            for (index, item) in items.iter_mut().enumerate() {
                close_value(item, format!("{path}[{index}]"))?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn close_object(object: &mut Map<String, Value>, path: String) -> anyhow::Result<()> {
    for (key, value) in object.iter_mut() {
        close_value(value, format!("{path}.{key}"))?;
    }

    if !object.get("properties").is_some_and(Value::is_object) {
        return Ok(());
    }

    match object.get("additionalProperties") {
        None => {
            object.insert("additionalProperties".to_string(), Value::Bool(false));
            Ok(())
        }
        Some(Value::Bool(false)) => Ok(()),
        Some(other) => bail!(
            "{path}.additionalProperties must be false for a declared output object, got {other}"
        ),
    }
}

fn normalize_value(value: &mut Value, path: String) -> anyhow::Result<()> {
    match value {
        Value::Object(object) => normalize_object(object, path),
        Value::Array(items) => {
            for (index, item) in items.iter_mut().enumerate() {
                normalize_value(item, format!("{path}[{index}]"))?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn normalize_object(object: &mut Map<String, Value>, path: String) -> anyhow::Result<()> {
    let nullable = object.remove("nullable");
    for (key, value) in object.iter_mut() {
        normalize_value(value, format!("{path}.{key}"))?;
    }

    match nullable {
        None | Some(Value::Bool(false)) => Ok(()),
        Some(Value::Bool(true)) => make_nullable(object, &path),
        Some(other) => bail!("{path}.nullable must be a boolean, got {other}"),
    }
}

fn make_nullable(object: &mut Map<String, Value>, path: &str) -> anyhow::Result<()> {
    if object.contains_key("$ref") || !object.contains_key("type") {
        let original = std::mem::take(object);
        object.insert(
            "anyOf".to_string(),
            Value::Array(vec![Value::Object(original), json!({ "type": "null" })]),
        );
        return Ok(());
    }

    let schema_type = object
        .get_mut("type")
        .with_context(|| format!("{path}.type disappeared while normalizing nullable"))?;
    match schema_type {
        Value::String(value) if value == "null" => Ok(()),
        Value::String(value) => {
            let value = std::mem::take(value);
            *schema_type = json!([value, "null"]);
            Ok(())
        }
        Value::Array(types) => {
            if types.iter().any(|value| !value.is_string()) {
                bail!("{path}.type array must contain only strings");
            }
            if !types.iter().any(|value| value.as_str() == Some("null")) {
                types.push(Value::String("null".to_string()));
            }
            Ok(())
        }
        other => bail!("{path}.type must be a string or string array, got {other}"),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{close_declared_objects, normalize_nullable};

    #[test]
    fn nullable_type_becomes_draft_2020_type_union() -> anyhow::Result<()> {
        let schema = normalize_nullable(
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "nullable": true },
                    "enabled": { "type": "boolean", "nullable": false }
                }
            })
            .as_object()
            .expect("fixture is an object")
            .clone(),
        )?;

        assert_eq!(
            schema["properties"]["name"]["type"],
            json!(["string", "null"])
        );
        assert_eq!(schema["properties"]["enabled"]["type"], "boolean");
        assert!(!serde_json::to_string(&schema)?.contains("nullable"));
        Ok(())
    }

    #[test]
    fn nullable_ref_becomes_any_of_with_explicit_null() -> anyhow::Result<()> {
        let schema = normalize_nullable(
            json!({
                "type": "object",
                "properties": {
                    "state": { "$ref": "#/$defs/State", "nullable": true }
                }
            })
            .as_object()
            .expect("fixture is an object")
            .clone(),
        )?;

        assert_eq!(
            schema["properties"]["state"]["anyOf"],
            json!([
                { "$ref": "#/$defs/State" },
                { "type": "null" }
            ])
        );
        Ok(())
    }

    #[test]
    fn declared_objects_are_closed_but_untyped_extensions_remain_open() -> anyhow::Result<()> {
        let schema = close_declared_objects(
            json!({
                "type": "object",
                "properties": {
                    "nested": {
                        "type": "object",
                        "properties": { "id": { "type": "integer" } }
                    },
                    "extension": {}
                }
            })
            .as_object()
            .expect("fixture is an object")
            .clone(),
        )?;

        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(
            schema["properties"]["nested"]["additionalProperties"],
            false
        );
        assert!(schema["properties"]["extension"]
            .get("additionalProperties")
            .is_none());
        Ok(())
    }
}
