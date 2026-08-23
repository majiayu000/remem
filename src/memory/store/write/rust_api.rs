use anyhow::{bail, ensure, Context, Result};
use rusqlite::Connection;

use crate::memory::activation::{
    ActivationActorKind, ActivationPoisoningVerdict, ActivationProvenanceKind, ActivationRouteKind,
    ActiveMemoryRoute, ActiveMemoryWriteRequest, ExpectedActiveMemory,
};
use crate::memory::lifecycle::MemoryLifecycleOp;
use crate::memory::poisoning::SourceTrustClass;

#[allow(clippy::too_many_arguments)]
pub fn insert_memory_full_with_reference_time(
    conn: &Connection,
    session_id: Option<&str>,
    project: &str,
    topic_key: Option<&str>,
    title: &str,
    content: &str,
    memory_type: &str,
    files: Option<&str>,
    branch: Option<&str>,
    scope: &str,
    created_at_override: Option<i64>,
    reference_time_override: Option<i64>,
) -> Result<i64> {
    if let Some(matched) =
        crate::memory::poisoning::scan_instruction_pattern(&format!("{title}\n{content}"))
    {
        bail!(
            "Rust memory API payload matched instruction-pattern {}@{}",
            matched.pattern_id,
            matched.pattern_set_version
        );
    }
    let (_, operation_plan) = crate::memory::operation::plan_direct_save(
        conn,
        "rust_api",
        "rust_api",
        project,
        scope,
        memory_type,
        topic_key,
        title,
        content,
        files,
        branch,
        None,
        None,
    )?;
    let target_memory_id = match operation_plan.op {
        MemoryLifecycleOp::Update | MemoryLifecycleOp::Noop => Some(
            operation_plan
                .target_memory_id
                .context("Rust memory write plan omitted its existing target")?,
        ),
        _ => None,
    };
    let existing = target_memory_id
        .map(|memory_id| ExpectedActiveMemory::from_existing(conn, memory_id))
        .transpose()?;
    let preserves_provenance = existing.as_ref().is_some_and(|memory| {
        memory.title == title
            && memory.memory_type == memory_type
            && memory.topic_key.as_deref() == topic_key
            && memory.files.as_deref() == files
            && (crate::memory::operation::same_memory_text(&memory.content, content)
                || (memory_type == "preference"
                && crate::memory::preference::reinforcement::cleanup_preserves_candidate_provenance(
                    &memory.content,
                    content,
                )))
    });

    let created_at = created_at_override.map(|value| value.to_string());
    let reference_time = reference_time_override.map(|value| value.to_string());
    let payload_sha256 = crate::memory::activation::payload_sha256(&[
        if session_id.is_some() { "1" } else { "0" },
        session_id.unwrap_or(""),
        project,
        if topic_key.is_some() { "1" } else { "0" },
        topic_key.unwrap_or(""),
        title,
        content,
        memory_type,
        if files.is_some() { "1" } else { "0" },
        files.unwrap_or(""),
        if branch.is_some() { "1" } else { "0" },
        branch.unwrap_or(""),
        scope,
        if created_at.is_some() { "1" } else { "0" },
        created_at.as_deref().unwrap_or(""),
        if reference_time.is_some() { "1" } else { "0" },
        reference_time.as_deref().unwrap_or(""),
    ]);
    let mut request = ActiveMemoryWriteRequest {
        activation_id: crate::memory::activation::ephemeral_activation_id(
            "rust-api",
            &payload_sha256,
        ),
        route_kind: ActivationRouteKind::RustApi,
        actor_kind: ActivationActorKind::RustApi,
        source_operation: "insert_memory_full_with_reference_time".to_string(),
        source_trust: SourceTrustClass::LocalToolOutput,
        result_source_trust: SourceTrustClass::LocalToolOutput,
        source_project: project.to_string(),
        route: ActiveMemoryRoute::default_for(project, branch, scope),
        provenance_kind: ActivationProvenanceKind::RustApi,
        provenance_ref: "rust-api:insert-memory:v1".to_string(),
        payload_sha256,
        expected_memory: ExpectedActiveMemory::new(title, content, memory_type)
            .with_topic_key(topic_key)
            .with_files(files),
        poisoning_verdict: ActivationPoisoningVerdict::Clean,
        superseded_ids: Vec::new(),
    };
    if let Some(memory_id) = target_memory_id {
        let existing_route = crate::memory::activation::load_existing_route(conn, memory_id)?;
        request.route = existing_route.route;
        request.source_project = existing_route.source_project;
        if preserves_provenance {
            let existing = existing
                .as_ref()
                .context("Rust memory write target payload was not loaded")?;
            request.result_source_trust = existing_route.result_source_trust;
            request.expected_memory.evidence_event_ids = existing.evidence_event_ids.clone();
            request.expected_memory.source_candidate_id = existing.source_candidate_id;
        }
    }

    Ok(
        crate::memory::activation::execute_one(conn, &request, |permit| {
            let memory_id = super::insert_memory_full_activated(
                conn,
                permit,
                session_id,
                project,
                topic_key,
                title,
                content,
                memory_type,
                files,
                branch,
                scope,
                created_at_override,
                reference_time_override,
            )?;
            if let Some(target_memory_id) = target_memory_id {
                ensure!(
                    memory_id == target_memory_id,
                    "Rust memory write plan target changed before activation"
                );
            }
            if !preserves_provenance {
                conn.execute(
                    "UPDATE memories
                 SET source_trust_class = 'local_tool_output',
                     source_candidate_id = NULL,
                     evidence_event_ids = NULL
                 WHERE id = ?1",
                    [memory_id],
                )?;
            }
            Ok(memory_id)
        })?
        .memory_id,
    )
}
