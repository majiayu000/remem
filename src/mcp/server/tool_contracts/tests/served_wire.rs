use std::collections::{BTreeMap, BTreeSet};

use rmcp::ServiceExt;
use rusqlite::params;
use serde_json::{json, Map, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use super::{next_message, send_message, send_tool_call};
use crate::db::test_support::ScopedTestDataDir;
use crate::memory::raw_archive::{insert_raw_message, ROLE_USER, SOURCE_HOOK};

#[tokio::test]
async fn every_schema_bearing_served_route_validates_a_real_non_empty_success() -> anyhow::Result<()>
{
    let _dir = ScopedTestDataDir::new("mcp-all-structured-wire");
    let fixture = seed_wire_fixture()?;
    let (server_transport, client_transport) = tokio::io::duplex(512 * 1024);
    let server = super::super::super::MemoryServer::new()?;
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
                "clientInfo": { "name": "remem-all-contracts-test", "version": "1" }
            }
        }),
    )
    .await?;
    let initialize = next_message(&mut messages).await?;
    assert_eq!(initialize["id"], 1);
    send_message(
        &mut client_writer,
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    )
    .await?;

    let calls = [
        WireCall::object(
            "current_state",
            "current_state",
            json!({ "state_key": "wire-state", "project": "/repo" }),
        ),
        WireCall::object(
            "search",
            "search",
            json!({
                "query": "wire contract",
                "project": "/repo",
                "limit": 5,
                "include_stale": true
            }),
        ),
        WireCall::object(
            "recall_user_context",
            "recall_user_context",
            json!({ "query": "wire recall", "project": "/repo", "limit": 5 }),
        ),
        WireCall::object(
            "context_bundle",
            "context_bundle",
            json!({
                "schema_version": 1,
                "task": "resume the wire contract work",
                "project": "/repo",
                "cwd": "/repo"
            }),
        ),
        WireCall::array(
            "timeline",
            "timeline",
            json!({ "anchor": fixture.compressed_observation_id, "project": "/repo" }),
            "observations",
        ),
        WireCall::array(
            "get_observations_observation",
            "get_observations",
            json!({
                "ids": [fixture.compressed_observation_id],
                "project": "/repo",
                "source": "observation"
            }),
            "details",
        ),
        WireCall::array(
            "get_observations_memory",
            "get_observations",
            json!({
                "ids": [fixture.memory_id],
                "project": "/repo",
                "source": "memory"
            }),
            "details",
        ),
        WireCall::array(
            "get_observations_session_summary",
            "get_observations",
            json!({
                "ids": [fixture.summary_id],
                "project": "/repo",
                "source": "session_summary"
            }),
            "details",
        ),
        WireCall::array(
            "lookup_commit",
            "lookup_commit",
            json!({ "sha": "abcdef1", "project": "/repo" }),
            "commits",
        ),
        WireCall::array(
            "commits_for_session",
            "commits_for_session",
            json!({ "session_id": "mem-session-wire", "project": "/repo" }),
            "commits",
        ),
        WireCall::object(
            "save_memory",
            "save_memory",
            json!({
                "text": "A second durable wire contract memory.",
                "title": "Wire contract save",
                "project": "/repo",
                "local_copy_enabled": false,
                "claim_enabled": false
            }),
        ),
        WireCall::object(
            "govern_memory",
            "govern_memory",
            json!({
                "ids": [fixture.memory_id],
                "project": "/repo",
                "action": "stale",
                "dry_run": true
            }),
        ),
        WireCall::array(
            "workstreams",
            "workstreams",
            json!({ "project": "/repo" }),
            "workstreams",
        ),
        WireCall::object(
            "update_workstream",
            "update_workstream",
            json!({ "id": fixture.workstream_id, "status": "paused" }),
        ),
        WireCall::object(
            "search_raw",
            "search_raw",
            json!({ "query": "wire-raw-contract", "project": "/repo" }),
        ),
        WireCall::object(
            "list_raw_sessions",
            "list_raw_sessions",
            json!({ "project": "/repo", "sample": 1 }),
        ),
    ];
    assert_eq!(
        calls
            .iter()
            .map(|call| call.tool)
            .collect::<BTreeSet<_>>()
            .len(),
        14
    );

    let mut results = BTreeMap::new();
    for (index, call) in calls.into_iter().enumerate() {
        let id = i64::try_from(index)? + 2;
        send_tool_call(&mut client_writer, id, call.tool, call.arguments.clone()).await?;
        let response = next_message(&mut messages).await?;
        assert!(
            response.get("error").is_none(),
            "{} returned a protocol error: {response}",
            call.label
        );
        let result = &response["result"];
        assert_ne!(
            result.get("isError").and_then(Value::as_bool),
            Some(true),
            "{} returned a tool error: {response}",
            call.label
        );
        let text = result["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("{} must preserve legacy text", call.label));
        let legacy: Value = serde_json::from_str(text)?;
        let expected = match call.envelope {
            Some(envelope) => Value::Object(Map::from_iter([(envelope.to_string(), legacy)])),
            None => legacy,
        };
        let structured = result["structuredContent"].clone();
        assert_eq!(
            structured, expected,
            "{} structuredContent must exactly mirror legacy JSON",
            call.label
        );
        assert_non_empty(&call, &structured);
        results.insert(call.label, structured);
    }

    assert_eq!(results["current_state"]["current"]["id"], fixture.memory_id);
    assert_eq!(results["search"]["results"][0]["id"], fixture.memory_id);
    assert_eq!(results["context_bundle"]["schema_version"], 1);
    assert_eq!(
        results["context_bundle"]["plan_hash"],
        results["context_bundle"]["audit"]["plan_hash"]
    );
    assert!(results["search"]["pagination"]["next_offset"].is_null());
    assert_eq!(
        results["get_observations_observation"]["details"][0]["compressed_sources"][0]
            ["source_observation_id"],
        fixture.source_observation_id
    );
    assert_eq!(
        results["get_observations_memory"]["details"][0]["memory_type"],
        "decision"
    );
    assert_eq!(
        results["get_observations_session_summary"]["details"][0]["id"],
        fixture.summary_id
    );
    assert!(results["govern_memory"]["reason"].is_null());
    assert!(results["workstreams"]["workstreams"][0]["description"].is_null());
    assert_eq!(results["update_workstream"]["updated"], true);
    assert_eq!(results["search_raw"]["results"][0]["source"], SOURCE_HOOK);
    assert_eq!(
        results["list_raw_sessions"]["sessions"][0]["message_count"],
        1
    );

    client_writer.shutdown().await?;
    drop(messages);
    tokio::time::timeout(std::time::Duration::from_secs(5), server_task).await???;
    Ok(())
}

#[derive(Clone)]
struct WireCall {
    label: &'static str,
    tool: &'static str,
    arguments: Value,
    envelope: Option<&'static str>,
}

impl WireCall {
    fn object(label: &'static str, tool: &'static str, arguments: Value) -> Self {
        Self {
            label,
            tool,
            arguments,
            envelope: None,
        }
    }

    fn array(
        label: &'static str,
        tool: &'static str,
        arguments: Value,
        envelope: &'static str,
    ) -> Self {
        Self {
            label,
            tool,
            arguments,
            envelope: Some(envelope),
        }
    }
}

fn assert_non_empty(call: &WireCall, structured: &Value) {
    let payload = call
        .envelope
        .map_or(structured, |envelope| &structured[envelope]);
    let non_empty = match payload {
        Value::Object(object) => !object.is_empty(),
        Value::Array(items) => !items.is_empty(),
        _ => false,
    };
    assert!(
        non_empty,
        "{} must exercise a non-empty success",
        call.label
    );
}

struct WireFixture {
    memory_id: i64,
    summary_id: i64,
    source_observation_id: i64,
    compressed_observation_id: i64,
    workstream_id: i64,
}

fn seed_wire_fixture() -> anyhow::Result<WireFixture> {
    let conn = crate::db::open_db()?;
    let memory_id = crate::memory::insert_memory(
        &conn,
        Some("memory-session-wire"),
        "/repo",
        Some("wire-state"),
        "Wire contract decision",
        "The wire contract keeps output schemas executable.",
        "decision",
        None,
    )?;
    let state_key_id: i64 = conn.query_row(
        "SELECT id FROM memory_state_keys
         WHERE owner_scope = 'repo' AND owner_key = '/repo'
           AND memory_type = 'decision' AND state_key = 'wire-state'",
        [],
        |row| row.get(0),
    )?;
    conn.execute(
        "UPDATE memory_state_keys
         SET current_memory_id = ?1, state_status = 'active'
         WHERE id = ?2",
        rusqlite::params![memory_id, state_key_id],
    )?;
    // source_trust_class must be set: this route asserts a real non-empty
    // current_state, and the G2 gate only admits a row with explicit writer
    // proof. A directly user-authored row is the spec's compatibility arm, so
    // the fixture does not need synthetic candidate or evidence rows.
    conn.execute(
        "UPDATE memories
         SET state_key_id = ?1, owner_scope = 'repo', owner_key = '/repo',
             context_class = 'startup_core', source_trust_class = 'user_prompt'
         WHERE id = ?2",
        rusqlite::params![state_key_id, memory_id],
    )?;

    crate::user_context::claims::create_manual_claim(
        &conn,
        &crate::user_context::claims::ManualClaimRequest {
            text: "Prefer the wire recall contract",
            owner_scope: None,
            owner_key: None,
            claim_type: crate::user_context::claims::UserContextClaimType::Preference,
            claim_key: None,
            confidence: 1.0,
            sensitivity: crate::user_context::claims::UserContextSensitivity::Normal,
            valid_from_epoch: None,
            valid_to_epoch: None,
        },
    )?;

    let source_observation_id = crate::db::insert_observation(
        &conn,
        "source-session-wire",
        "/repo",
        "discovery",
        Some("Source wire evidence"),
        None,
        Some("Original wire observation"),
        None,
        None,
        None,
        None,
        None,
        0,
    )?;
    let compressed_observation_id = crate::db::insert_observation(
        &conn,
        "compressed-session-wire",
        "/repo",
        "decision",
        Some("Compressed wire decision"),
        None,
        Some("Compressed wire narrative"),
        None,
        None,
        None,
        None,
        None,
        0,
    )?;
    let sources =
        crate::db::get_observations_by_ids(&conn, &[source_observation_id], Some("/repo"))?;
    crate::db::insert_compressed_observation_sources(
        &conn,
        &[compressed_observation_id],
        &sources,
        "compressed-session-wire",
    )?;

    let changed_files = vec!["src/mcp/server/tool_contracts.rs".to_string()];
    crate::git_trace::link_commit_to_session(
        &conn,
        &crate::git_trace::CommitLinkInput {
            metadata: crate::git_trace::CommitMetadataInput {
                project: "/repo",
                repo_path: Some("/repo"),
                sha: "abcdef1234567890abcdef1234567890abcdef12",
                short_sha: Some("abcdef1"),
                branch: Some("main"),
                message: Some("Enforce wire output contracts"),
                authored_at_epoch: Some(1_700_000_000),
                changed_files: &changed_files,
            },
            session_id: "content-session-wire",
            memory_session_id: Some("mem-session-wire"),
            source: "git_metadata",
        },
    )?;

    let raw = insert_raw_message(
        &conn,
        "raw-session-wire",
        "/repo",
        ROLE_USER,
        "wire-raw-contract",
        SOURCE_HOOK,
        Some("main"),
        Some("/repo"),
    )?
    .ok_or_else(|| anyhow::anyhow!("wire raw row was unexpectedly deduplicated"))?;
    conn.execute(
        "INSERT INTO raw_session_identities
         (source_root, transcript_path, host, fallback_session_id,
          canonical_session_id, project, legacy_project, status,
          contract_version, observed_mtime_ns, observed_size_bytes,
          first_seen_at_epoch, last_seen_at_epoch)
         VALUES ('local', '/tmp/.codex/sessions/raw-session-wire.jsonl',
                 'codex-cli', 'raw-session-wire', 'raw-session-wire',
                 '/repo', '/repo', 'active', 1, 1, 1, 1, 1)",
        [],
    )?;
    conn.execute(
        "UPDATE raw_messages
         SET transcript_identity_id = ?1, transcript_record_ordinal = 1
         WHERE id = ?2",
        params![conn.last_insert_rowid(), raw.id],
    )?;
    conn.execute(
        "INSERT INTO workstreams
         (project, title, status, created_at_epoch, updated_at_epoch)
         VALUES ('/repo', 'Wire contract', 'active', 1, 1)",
        [],
    )?;
    let workstream_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO session_summaries
         (memory_session_id, project, request, completed, next_steps, created_at,
          created_at_epoch)
         VALUES ('mem-session-summary-wire', '/repo', 'Resume wire contract',
                 'Verified summary detail', 'Finish served-wire coverage',
                 '2026-09-02 00:00:00', 1700000000)",
        [],
    )?;
    let summary_id = conn.last_insert_rowid();

    Ok(WireFixture {
        memory_id,
        summary_id,
        source_observation_id,
        compressed_observation_id,
        workstream_id,
    })
}
