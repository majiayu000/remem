mod schemas;

use std::collections::BTreeSet;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use rmcp::handler::server::router::tool::{ToolRoute, ToolRouter};
use rmcp::model::{CallToolResult, ToolAnnotations};
use serde_json::{json, Map, Value};

use super::MemoryServer;
use schemas::{build_schema, validate_output, OutputSchema};

#[derive(Clone, Copy, Debug)]
enum LegacyShape {
    Object,
    Array { envelope: &'static str },
}

#[derive(Clone, Copy, Debug)]
struct ToolContract {
    name: &'static str,
    title: &'static str,
    read_only: bool,
    destructive: bool,
    idempotent: bool,
    open_world: bool,
    output: Option<(OutputSchema, LegacyShape)>,
}

const fn json_object(
    name: &'static str,
    title: &'static str,
    read_only: bool,
    destructive: bool,
    idempotent: bool,
    open_world: bool,
    schema: OutputSchema,
) -> ToolContract {
    ToolContract {
        name,
        title,
        read_only,
        destructive,
        idempotent,
        open_world,
        output: Some((schema, LegacyShape::Object)),
    }
}

const fn json_array(
    name: &'static str,
    title: &'static str,
    read_only: bool,
    destructive: bool,
    idempotent: bool,
    open_world: bool,
    schema: OutputSchema,
    envelope: &'static str,
) -> ToolContract {
    ToolContract {
        name,
        title,
        read_only,
        destructive,
        idempotent,
        open_world,
        output: Some((schema, LegacyShape::Array { envelope })),
    }
}

const CONTRACTS: [ToolContract; 15] = [
    json_object(
        "current_state",
        "Current State",
        true,
        false,
        true,
        false,
        OutputSchema::CurrentState,
    ),
    json_object(
        "search",
        "Search Memories",
        true,
        false,
        true,
        true,
        OutputSchema::Search,
    ),
    json_object(
        "recall_user_context",
        "Recall User Context",
        false,
        true,
        false,
        true,
        OutputSchema::RecallUserContext,
    ),
    json_object(
        "context_bundle",
        "Compile Context Bundle (Experimental)",
        false,
        true,
        false,
        false,
        OutputSchema::ContextBundle,
    ),
    json_array(
        "timeline",
        "Memory Timeline",
        true,
        false,
        true,
        true,
        OutputSchema::Timeline,
        "observations",
    ),
    json_array(
        "get_observations",
        "Get Observation Details",
        false,
        true,
        false,
        false,
        OutputSchema::GetObservations,
        "details",
    ),
    json_array(
        "lookup_commit",
        "Lookup Commit",
        false,
        true,
        false,
        false,
        OutputSchema::LookupCommit,
        "commits",
    ),
    json_array(
        "commits_for_session",
        "List Session Commits",
        false,
        true,
        false,
        false,
        OutputSchema::CommitsForSession,
        "commits",
    ),
    json_object(
        "save_memory",
        "Save Memory",
        false,
        true,
        false,
        true,
        OutputSchema::SaveMemory,
    ),
    json_object(
        "govern_memory",
        "Govern Memory",
        false,
        true,
        false,
        false,
        OutputSchema::GovernMemory,
    ),
    ToolContract {
        name: "timeline_report",
        title: "Timeline Report",
        read_only: true,
        destructive: false,
        idempotent: true,
        open_world: false,
        output: None,
    },
    json_array(
        "workstreams",
        "List Workstreams",
        true,
        false,
        true,
        false,
        OutputSchema::Workstreams,
        "workstreams",
    ),
    json_object(
        "update_workstream",
        "Update Workstream",
        false,
        true,
        false,
        false,
        OutputSchema::UpdateWorkstream,
    ),
    json_object(
        "search_raw",
        "Search Raw Archive",
        true,
        false,
        true,
        false,
        OutputSchema::SearchRaw,
    ),
    json_object(
        "list_raw_sessions",
        "List Raw Sessions",
        true,
        false,
        true,
        false,
        OutputSchema::ListRawSessions,
    ),
];

pub(super) fn apply(router: &mut ToolRouter<MemoryServer>) -> Result<()> {
    verify_complete_registry(router)?;

    for contract in CONTRACTS {
        let route = router
            .map
            .get_mut(contract.name)
            .with_context(|| format!("MCP contract route disappeared: {}", contract.name))?;
        route.attr.title = Some(contract.title.to_string());
        route.attr.annotations = Some(
            ToolAnnotations::with_title(contract.title)
                .read_only(contract.read_only)
                .destructive(contract.destructive)
                .idempotent(contract.idempotent)
                .open_world(contract.open_world),
        );

        if let Some((schema, shape)) = contract.output {
            route.attr.output_schema = Some(
                build_schema(schema)
                    .with_context(|| format!("build output schema for {}", contract.name))?,
            );
            wrap_success(route, contract.name, shape, schema);
        } else {
            route.attr.output_schema = None;
        }
    }

    Ok(())
}

fn verify_complete_registry(router: &ToolRouter<MemoryServer>) -> Result<()> {
    let registered = router
        .map
        .keys()
        .map(|name| name.as_ref())
        .collect::<BTreeSet<_>>();
    let contracted = CONTRACTS
        .iter()
        .map(|contract| contract.name)
        .collect::<BTreeSet<_>>();

    if contracted.len() != CONTRACTS.len() {
        bail!("MCP tool contract registry contains duplicate tool names");
    }

    if registered == contracted {
        return Ok(());
    }

    let missing = registered
        .difference(&contracted)
        .copied()
        .collect::<Vec<_>>();
    let unexpected = contracted
        .difference(&registered)
        .copied()
        .collect::<Vec<_>>();
    bail!(
        "MCP tool contract registry mismatch; registered without contract: {missing:?}; contract without route: {unexpected:?}"
    );
}

fn wrap_success(
    route: &mut ToolRoute<MemoryServer>,
    tool: &'static str,
    shape: LegacyShape,
    schema: OutputSchema,
) {
    let original = Arc::clone(&route.call);
    route.call = Arc::new(move |context| {
        let original = Arc::clone(&original);
        Box::pin(async move {
            let result = original(context).await?;
            add_structured_content(tool, shape, schema, result)
        })
    });
}

fn add_structured_content(
    tool: &'static str,
    shape: LegacyShape,
    schema: OutputSchema,
    mut result: CallToolResult,
) -> std::result::Result<CallToolResult, rmcp::ErrorData> {
    if result.is_error == Some(true) {
        return Ok(result);
    }
    if result.structured_content.is_some() {
        return Err(contract_violation(
            tool,
            "handler unexpectedly returned structured content before adaptation",
        ));
    }
    if result.content.len() != 1 {
        return Err(contract_violation(
            tool,
            format!(
                "expected exactly one legacy text content item, got {}",
                result.content.len()
            ),
        ));
    }

    let text = result.content[0]
        .raw
        .as_text()
        .map(|content| content.text.as_str())
        .ok_or_else(|| contract_violation(tool, "legacy success content was not text"))?;
    let parsed: Value = serde_json::from_str(text).map_err(|error| {
        contract_violation(
            tool,
            format!("legacy success content was not JSON: {error}"),
        )
    })?;

    let structured = match shape {
        LegacyShape::Object if parsed.is_object() => parsed,
        LegacyShape::Array { envelope } if parsed.is_array() => {
            Value::Object(Map::from_iter([(envelope.to_string(), parsed)]))
        }
        LegacyShape::Object => {
            return Err(contract_violation(
                tool,
                "legacy success JSON root was not an object",
            ));
        }
        LegacyShape::Array { .. } => {
            return Err(contract_violation(
                tool,
                "legacy success JSON root was not an array",
            ));
        }
    };
    validate_output(schema, &structured).map_err(|error| {
        contract_violation(
            tool,
            format!("structured success did not match outputSchema: {error:#}"),
        )
    })?;
    result.structured_content = Some(structured);
    Ok(result)
}

fn contract_violation(tool: &'static str, reason: impl Into<String>) -> rmcp::ErrorData {
    rmcp::ErrorData::internal_error(
        format!("{tool} violated its declared MCP output contract"),
        Some(json!({
            "tool": tool,
            "reason": reason.into(),
        })),
    )
}

#[cfg(test)]
mod tests;
