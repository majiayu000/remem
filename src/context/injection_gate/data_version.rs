use anyhow::Result;
use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};
use std::path::Path;

use super::super::invocation::ContextInvocation;
use super::super::policy::{ContextPolicy, SectionKind};
use super::super::render_inputs::ContextRenderInputs;
use super::super::types::ContextRequest;

pub(super) fn context_injections_has_data_version(conn: &rusqlite::Connection) -> Result<bool> {
    conn.query_row(
        "SELECT 1
         FROM pragma_table_info('context_injections')
         WHERE name = 'data_version'
         LIMIT 1",
        [],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .map(|value| value.is_some())
    .map_err(Into::into)
}

pub(in crate::context) fn compute_data_version_from_render_inputs(
    request: &ContextRequest,
    invocation: &ContextInvocation,
    policy: &ContextPolicy,
    inputs: &ContextRenderInputs,
) -> Result<String> {
    let mut version = DataVersionBuilder::new();
    version.push("version", "3");
    version.push("project", &request.project);
    version.push("cwd", &request.cwd);
    version.push("branch", request.current_branch.as_deref().unwrap_or(""));
    version.push("session", request.session_id.as_deref().unwrap_or(""));
    version.push("source", request.hook_source.as_deref().unwrap_or(""));
    version.push("host", request.host.as_env_value());
    version.push("colors", if request.use_colors { "1" } else { "0" });
    version.push("gate_mode", invocation.gate_mode.as_deref().unwrap_or(""));
    version.push("package", env!("CARGO_PKG_VERSION"));
    version.push(
        "schema",
        &crate::migrate::latest_schema_version().to_string(),
    );
    version.push("policy", &format!("{:?}", policy));
    version.push("claude_md", &claude_md_fingerprint(&request.cwd)?);

    let now = chrono::Utc::now().timestamp();
    version.push("day_bucket", &(now / 86_400).to_string());
    push_render_input_signal(inputs, policy, &mut version);

    Ok(version.finish())
}

fn push_render_input_signal(
    inputs: &ContextRenderInputs,
    policy: &ContextPolicy,
    version: &mut DataVersionBuilder,
) {
    let loaded = &inputs.loaded;
    version.push(
        "memory_abstained",
        if loaded.memory_abstained { "1" } else { "0" },
    );
    version.push("errors_count", &loaded.errors.len().to_string());
    for error in &loaded.errors {
        version.push_row(
            "load_error",
            &[error.section.to_string(), error.message.clone()],
        );
    }
    for memory in &loaded.memories {
        push_memory(version, "memory_render_input", memory);
    }
    version.push(
        "memory_render_input_count",
        &loaded.memories.len().to_string(),
    );

    let mut staleness = loaded
        .staleness_labels
        .iter()
        .map(|(id, label)| (*id, label.label.clone()))
        .collect::<Vec<_>>();
    staleness.sort_by_key(|(id, _)| *id);
    for (id, label) in &staleness {
        version.push_row("staleness_label", &[id.to_string(), label.clone()]);
    }
    version.push("staleness_label_count", &staleness.len().to_string());

    for lesson in &loaded.lessons {
        push_memory(version, "lesson_render_input", &lesson.memory);
        version.push_row(
            "lesson_render_meta",
            &[
                lesson.metadata.memory_id.to_string(),
                format!("{:.6}", lesson.metadata.confidence),
                lesson.metadata.reinforcement_count.to_string(),
                lesson.metadata.source_evidence.clone().unwrap_or_default(),
                lesson.metadata.last_reinforced_at_epoch.to_string(),
                lesson
                    .metadata
                    .stale_after_epoch
                    .unwrap_or_default()
                    .to_string(),
            ],
        );
    }
    version.push(
        "lesson_render_input_count",
        &loaded.lessons.len().to_string(),
    );

    for memory in &inputs.preference_details.rendered_memories {
        push_memory(version, "preference_render_input", memory);
    }
    version.push(
        "preference_render_input_count",
        &inputs
            .preference_details
            .rendered_memories
            .len()
            .to_string(),
    );
    version.push(
        "preferences_project_count",
        &inputs
            .preference_details
            .summary
            .project_rendered
            .to_string(),
    );
    version.push(
        "preferences_global_count",
        &inputs
            .preference_details
            .summary
            .global_rendered
            .to_string(),
    );

    for summary in &loaded.summaries {
        version.push_row(
            "session_render_input",
            &[
                summary.created_at_epoch.to_string(),
                summary.request.clone(),
                summary.completed.clone().unwrap_or_default(),
            ],
        );
    }
    version.push(
        "session_render_input_count",
        &loaded.summaries.len().to_string(),
    );

    let workstream_limit = policy.section_item_limit(SectionKind::Workstreams, 5);
    for workstream in loaded.workstreams.iter().take(workstream_limit) {
        version.push_row(
            "workstream_render_input",
            &[
                workstream.id.to_string(),
                workstream.project.clone(),
                workstream.title.clone(),
                workstream.description.clone().unwrap_or_default(),
                workstream.status.as_str().to_string(),
                workstream.progress.clone().unwrap_or_default(),
                workstream.next_action.clone().unwrap_or_default(),
                workstream.blockers.clone().unwrap_or_default(),
                workstream.created_at_epoch.to_string(),
                workstream.updated_at_epoch.to_string(),
                workstream
                    .completed_at_epoch
                    .unwrap_or_default()
                    .to_string(),
            ],
        );
    }
    version.push(
        "workstream_render_input_count",
        &loaded.workstreams.len().min(workstream_limit).to_string(),
    );

    version.push("owner_repo", &loaded.owner_counts.repo.to_string());
    version.push("owner_user", &loaded.owner_counts.user.to_string());
    version.push(
        "owner_workspace",
        &loaded.owner_counts.workspace.to_string(),
    );
    version.push("owner_tool", &loaded.owner_counts.tool.to_string());
    version.push("owner_domain", &loaded.owner_counts.domain.to_string());
    version.push(
        "owner_workstream",
        &loaded.owner_counts.workstream.to_string(),
    );
    version.push("owner_session", &loaded.owner_counts.session.to_string());
    version.push("owner_legacy", &loaded.owner_counts.legacy.to_string());
    version.push("owner_unknown", &loaded.owner_counts.unknown.to_string());
}

fn claude_md_fingerprint(cwd: &str) -> Result<String> {
    let path = Path::new(cwd).join("CLAUDE.md");
    match std::fs::read(&path) {
        Ok(bytes) => Ok(super::sha256_hex_bytes(&bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok("missing".to_string()),
        Err(error) => Err(error.into()),
    }
}

fn push_memory(
    version: &mut DataVersionBuilder,
    label: &'static str,
    memory: &crate::memory::Memory,
) {
    version.push_row(
        label,
        &[
            memory.id.to_string(),
            memory.session_id.clone().unwrap_or_default(),
            memory.project.clone(),
            memory.topic_key.clone().unwrap_or_default(),
            memory.title.clone(),
            memory.text.clone(),
            memory.memory_type.clone(),
            memory.files.clone().unwrap_or_default(),
            memory.created_at_epoch.to_string(),
            memory.updated_at_epoch.to_string(),
            memory.status.clone(),
            memory.branch.clone().unwrap_or_default(),
            memory.scope.clone(),
        ],
    );
}

struct DataVersionBuilder {
    hasher: Sha256,
}

impl DataVersionBuilder {
    fn new() -> Self {
        Self {
            hasher: Sha256::new(),
        }
    }

    fn push(&mut self, key: &str, value: &str) {
        self.hasher.update(key.as_bytes());
        self.hasher.update([0]);
        self.hasher.update(value.as_bytes());
        self.hasher.update([0xff]);
    }

    fn push_row(&mut self, label: &str, fields: &[String]) {
        self.push(label, &fields.len().to_string());
        for field in fields {
            self.push("field", field);
        }
        self.push("row_end", label);
    }

    fn finish(self) -> String {
        format!("{:x}", self.hasher.finalize())
    }
}
