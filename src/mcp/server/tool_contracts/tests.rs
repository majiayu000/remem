use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content};
use rmcp::ServiceExt;
use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader, Lines};

use super::{add_structured_content, LegacyShape, CONTRACTS};
use crate::db::test_support::ScopedTestDataDir;
use crate::mcp::types::{CurrentStateParams, TimelineReportParams};

#[test]
fn contract_names_are_unique() {
    let names = CONTRACTS
        .iter()
        .map(|contract| contract.name)
        .collect::<std::collections::BTreeSet<_>>();

    assert_eq!(names.len(), CONTRACTS.len());
}

#[test]
fn object_adapter_preserves_legacy_text_exactly() {
    let legacy = "{\n  \"status\": \"ok\"\n}";
    let result = CallToolResult::success(vec![Content::text(legacy)]);

    let adapted = add_structured_content("test", LegacyShape::Object, result)
        .expect("object response should adapt");

    assert_eq!(result_text(&adapted), legacy);
    assert_eq!(adapted.structured_content, Some(json!({ "status": "ok" })));
    let wire = serde_json::to_value(&adapted).expect("result should serialize");
    assert_eq!(wire["structuredContent"], json!({ "status": "ok" }));
    assert!(wire.get("structured_content").is_none());
}

#[test]
fn array_adapter_preserves_text_and_adds_named_envelope() {
    let legacy = "[\n  {\"id\": 7}\n]";
    let result = CallToolResult::success(vec![Content::text(legacy)]);

    let adapted = add_structured_content(
        "test",
        LegacyShape::Array {
            envelope: "details",
        },
        result,
    )
    .expect("array response should adapt");

    assert_eq!(result_text(&adapted), legacy);
    assert_eq!(
        adapted.structured_content,
        Some(json!({ "details": [{ "id": 7 }] }))
    );
    let wire = serde_json::to_value(&adapted).expect("result should serialize");
    assert_eq!(
        wire["structuredContent"],
        json!({ "details": [{ "id": 7 }] })
    );
}

#[test]
fn error_result_is_not_adapted() {
    let original = CallToolResult::error(vec![Content::text("{\"error\":{}}")]);

    let adapted = add_structured_content("test", LegacyShape::Object, original.clone())
        .expect("tool errors should pass through");

    assert_eq!(adapted, original);
    assert!(adapted.structured_content.is_none());
}

#[test]
fn malformed_or_wrong_shape_success_fails_loudly() {
    let malformed = CallToolResult::success(vec![Content::text("not json")]);
    let wrong_shape = CallToolResult::success(vec![Content::text("[]")]);

    let malformed_error = add_structured_content("test", LegacyShape::Object, malformed)
        .expect_err("malformed JSON must fail");
    let shape_error = add_structured_content("test", LegacyShape::Object, wrong_shape)
        .expect_err("wrong root shape must fail");

    assert!(malformed_error.message.contains("output contract"));
    assert!(shape_error.message.contains("output contract"));
}

#[test]
fn adapter_rejects_values_that_drift_from_the_advertised_schema() {
    let cases = [
        (
            "undeclared root field",
            "update_workstream",
            LegacyShape::Object,
            json!({ "id": 7, "updated": true, "schema_drift_probe": true }),
        ),
        (
            "missing required field",
            "update_workstream",
            LegacyShape::Object,
            json!({ "id": 7 }),
        ),
        (
            "null in a non-null field",
            "update_workstream",
            LegacyShape::Object,
            json!({ "id": null, "updated": true }),
        ),
        (
            "wrong nested field type",
            "workstreams",
            LegacyShape::Array {
                envelope: "workstreams",
            },
            json!([{
                "id": "not-an-integer",
                "project": "/repo",
                "title": "Release",
                "status": "active",
                "created_at_epoch": 1,
                "updated_at_epoch": 1
            }]),
        ),
        (
            "object mixes mutually exclusive union branches",
            "get_observations",
            LegacyShape::Array {
                envelope: "details",
            },
            json!([{
                "id": 7,
                "project": "/repo",
                "title": "Memory",
                "text": "body",
                "memory_type": "decision",
                "created_at_epoch": 1,
                "updated_at_epoch": 1,
                "status": "active",
                "scope": "project",
                "memory_session_id": "session-1",
                "type": "discovery",
                "created_at": "1970-01-01T00:00:01Z"
            }]),
        ),
    ];

    for (label, tool, shape, value) in cases {
        let result = CallToolResult::success(vec![Content::text(value.to_string())]);
        let error = add_structured_content(tool, shape, result)
            .expect_err(&format!("{label} should violate {tool}'s outputSchema"));
        assert!(error.message.contains("output contract"));
    }
}

#[tokio::test]
async fn served_routes_preserve_text_and_publish_structured_successes() -> anyhow::Result<()> {
    let _dir = ScopedTestDataDir::new("mcp-structured-wire");
    let (server_transport, client_transport) = tokio::io::duplex(128 * 1024);
    let server = super::super::MemoryServer::new()?;
    let expected_object_text = server
        .current_state(Parameters(CurrentStateParams {
            state_key: "missing-key".to_string(),
            project: Some("/repo".to_string()),
            r#type: None,
            owner_scope: None,
            owner_key: None,
            as_of_epoch: None,
        }))
        .unwrap_or_else(|err| panic!("direct current_state baseline should succeed: {err:?}"));
    let expected_report_text = server
        .timeline_report(Parameters(TimelineReportParams {
            project: "/repo".to_string(),
            full: None,
        }))
        .unwrap_or_else(|err| panic!("direct timeline_report baseline should succeed: {err:?}"));
    let server_task = tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });
    let (client_reader, mut client_writer) = tokio::io::split(client_transport);
    let mut messages = BufReader::new(client_reader).lines();

    send_message(
        &mut client_writer,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": { "name": "remem-contract-test", "version": "1" }
            }
        }),
    )
    .await?;
    let initialize = next_message(&mut messages).await?;
    assert_eq!(initialize["id"], 1);
    assert_eq!(initialize["result"]["protocolVersion"], "2025-03-26");
    send_message(
        &mut client_writer,
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    )
    .await?;

    send_tool_call(
        &mut client_writer,
        2,
        "current_state",
        json!({ "state_key": "missing-key", "project": "/repo" }),
    )
    .await?;
    let object_response = next_message(&mut messages).await?;
    let object_text = object_response["result"]["content"][0]["text"]
        .as_str()
        .expect("object tool should preserve text content");
    assert_eq!(object_text, expected_object_text);
    let object_json: Value = serde_json::from_str(object_text)?;
    assert_eq!(object_json["status"], "not_found");
    assert_eq!(object_response["result"]["structuredContent"], object_json);

    send_tool_call(
        &mut client_writer,
        3,
        "workstreams",
        json!({ "project": "/repo" }),
    )
    .await?;
    let array_response = next_message(&mut messages).await?;
    assert_eq!(array_response["result"]["content"][0]["text"], "[]");
    assert_eq!(
        array_response["result"]["structuredContent"],
        json!({ "workstreams": [] })
    );

    send_tool_call(
        &mut client_writer,
        4,
        "current_state",
        json!({ "state_key": " " }),
    )
    .await?;
    let error_response = next_message(&mut messages).await?;
    assert_eq!(error_response["result"]["isError"], true);
    assert!(error_response["result"].get("structuredContent").is_none());

    send_tool_call(
        &mut client_writer,
        5,
        "timeline_report",
        json!({ "project": "/repo" }),
    )
    .await?;
    let report_response = next_message(&mut messages).await?;
    assert_eq!(
        report_response["result"]["content"][0]["text"],
        expected_report_text
    );
    assert!(expected_report_text.starts_with("# Journey Into /repo\n"));
    assert!(report_response["result"].get("structuredContent").is_none());

    client_writer.shutdown().await?;
    drop(messages);
    tokio::time::timeout(std::time::Duration::from_secs(5), server_task).await???;
    Ok(())
}

async fn send_tool_call<W: AsyncWrite + Unpin>(
    writer: &mut W,
    id: i64,
    name: &str,
    arguments: Value,
) -> anyhow::Result<()> {
    send_message(
        writer,
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        }),
    )
    .await
}

async fn send_message<W: AsyncWrite + Unpin>(writer: &mut W, message: Value) -> anyhow::Result<()> {
    writer.write_all(message.to_string().as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}

async fn next_message<R: AsyncBufRead + Unpin>(messages: &mut Lines<R>) -> anyhow::Result<Value> {
    let line = messages
        .next_line()
        .await?
        .ok_or_else(|| anyhow::anyhow!("MCP transport closed before its response"))?;
    Ok(serde_json::from_str(&line)?)
}

fn result_text(result: &CallToolResult) -> &str {
    result
        .content
        .first()
        .and_then(|content| content.raw.as_text())
        .map(|text| text.text.as_str())
        .expect("tool result should contain one text item")
}
