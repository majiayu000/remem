use anyhow::Result;
#[cfg(test)]
use rusqlite::params;

use super::audit::{record_context_injection_items, ContextAuditItem};
use super::host::resolve_host_kind;
#[cfg(test)]
use super::injection_gate::injection_key_for_audit;
use super::injection_gate::{ContextGateAction, ContextGateDecision};
use super::invocation::ContextInvocation;
use super::policy::{ContextLimits, ContextPolicy, SectionKind};

const PROMPT_SUBMIT_MEMORY_LIMIT: i64 = 4;
#[cfg(test)]
const PROMPT_SUBMIT_LATENCY_BUDGET_MS: u128 = 250;

mod candidates;
use candidates::{
    memory_detail_read_tokens, prompt_submit_staleness_labels, render_prompt_submit_context,
};
mod audit_identity;

#[cfg(test)]
mod regression_tests;
#[cfg(test)]
mod review_regression_tests;

pub(crate) fn prompt_submit_additional_context(
    conn: &rusqlite::Connection,
    cwd: &str,
    project: &str,
    session_id: &str,
    prompt: &str,
    host_arg: Option<&str>,
) -> Result<Option<String>> {
    prompt_submit_additional_context_for_event(
        conn, cwd, project, session_id, prompt, host_arg, None,
    )
}

pub(crate) fn prompt_submit_additional_context_for_event(
    conn: &rusqlite::Connection,
    cwd: &str,
    project: &str,
    session_id: &str,
    prompt: &str,
    host_arg: Option<&str>,
    prompt_event_id: Option<&str>,
) -> Result<Option<String>> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Ok(None);
    }

    let host = resolve_host_kind(host_arg);
    let invocation = ContextInvocation {
        cwd: cwd.to_string(),
        project: project.to_string(),
        session_id: Some(session_id.to_string()),
        transcript_path: None,
        source: Some("UserPromptSubmit".to_string()),
        host,
        use_colors: false,
        debug: false,
        force: false,
        gate_mode: None,
    };
    let policy = ContextPolicy::from_limits(ContextLimits::default());
    let excluded_types = policy
        .section(SectionKind::MemoryIndex)
        .map(|section| section.exclude_types.as_slice())
        .unwrap_or(&[]);
    let current_branch = crate::db::detect_git_branch(cwd);
    let as_of_epoch = chrono::Utc::now().timestamp();
    let prompt_injection_key = audit_identity::prompt_injection_key(&invocation, prompt_event_id);
    let already_injected = audit_identity::previously_injected_memory_ids(
        conn,
        &invocation,
        prompt_injection_key.as_deref(),
    )?;
    let (mut retrieved, poisoning_safe_ids) = super::prompt_submit_retrieval::retrieve(
        conn,
        project,
        prompt,
        current_branch.as_deref(),
        excluded_types,
        PROMPT_SUBMIT_MEMORY_LIMIT,
        as_of_epoch,
        &already_injected,
    )?;
    let mut g2_drops = Vec::new();
    let mut g2_errors = Vec::new();
    super::query::exclude_non_current_context_memories(
        conn,
        &mut retrieved,
        &mut g2_drops,
        as_of_epoch,
        &mut g2_errors,
    );
    if let Some(error) = g2_errors.first() {
        anyhow::bail!("prompt-submit memory visibility classification failed: {error:?}");
    }
    let mut rendered = Vec::new();
    let mut audit_items = Vec::new();
    for drop in g2_drops {
        if let super::types::ContextPreselectionItem::Memory(memory) = drop.item {
            audit_items.push(ContextAuditItem::dropped_memory(
                &memory,
                "prompt_submit",
                drop.reason,
            ));
        }
    }
    for memory in retrieved {
        if already_injected.contains(&memory.id) {
            audit_items.push(ContextAuditItem::dropped_memory(
                &memory,
                "prompt_submit",
                "already_injected",
            ));
        } else if !poisoning_safe_ids.contains(&memory.id) {
            audit_items.push(ContextAuditItem::dropped_memory(
                &memory,
                "prompt_submit",
                "prompt_submit_poisoning_gate",
            ));
        } else if rendered.len() < PROMPT_SUBMIT_MEMORY_LIMIT as usize {
            rendered.push(memory);
        } else {
            audit_items.push(ContextAuditItem::dropped_memory(
                &memory,
                "prompt_submit",
                "prompt_submit_memory_limit",
            ));
        }
    }

    let continuity = candidates::load_first_turn_continuity(conn, project, &invocation)?;
    audit_items.extend(continuity.audit_items().iter().cloned());

    if rendered.is_empty() && continuity.is_empty() {
        audit_items.push(prompt_submit_abstained_item(
            "prompt_submit_no_relevant_context",
        ));
        let decision = empty_prompt_submit_decision(prompt_injection_key);
        record_context_injection_items(conn, &invocation, &decision, &audit_items)?;
        return Ok(None);
    }

    let render_reference_epoch = chrono::Utc::now().timestamp();
    let staleness_labels = prompt_submit_staleness_labels(conn, &rendered, render_reference_epoch);
    let memory_read_tokens = memory_detail_read_tokens(conn, &rendered)?;
    let rendered_context = render_prompt_submit_context(
        &continuity,
        &rendered,
        &memory_read_tokens,
        &staleness_labels,
        render_reference_epoch,
    )?;
    audit_items.extend(rendered_context.audit_items);
    if !rendered_context.has_candidates {
        audit_items.push(prompt_submit_abstained_item(
            "prompt_submit_no_rendered_candidates",
        ));
        let decision = empty_prompt_submit_decision(prompt_injection_key);
        record_context_injection_items(conn, &invocation, &decision, &audit_items)?;
        return Ok(None);
    }
    let decision = prompt_submit_decision(rendered_context.output, prompt_injection_key);
    record_context_injection_items(conn, &invocation, &decision, &audit_items)?;
    Ok(Some(decision.output))
}

fn empty_prompt_submit_decision(key: Option<String>) -> ContextGateDecision {
    ContextGateDecision {
        output: String::new(),
        action: ContextGateAction::Bypassed,
        reason: "prompt_submit_empty",
        key,
        context_hash: None,
        output_mode: Some("prompt_submit"),
        retained_context_chars: None,
        output_truncated: false,
    }
}

fn prompt_submit_decision(output: String, key: Option<String>) -> ContextGateDecision {
    ContextGateDecision {
        output,
        action: ContextGateAction::Bypassed,
        reason: "prompt_submit",
        key,
        context_hash: None,
        output_mode: Some("prompt_submit"),
        retained_context_chars: None,
        output_truncated: false,
    }
}

fn prompt_submit_abstained_item(reason: &'static str) -> ContextAuditItem {
    ContextAuditItem {
        item_kind: "memory",
        item_id: None,
        memory_id: None,
        channel: "prompt_submit",
        score: None,
        render_order: None,
        status: "abstained",
        drop_reason: Some(reason),
        title: "prompt context abstained".to_string(),
        provenance: "src=memory".to_string(),
        staleness: "staleness=none".to_string(),
        render_end_chars: None,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use rusqlite::Connection;

    use super::*;
    use crate::context::host::HostKind;
    use crate::context::injection_gate::apply_context_gate_with_data_version;
    use crate::context::invocation::ContextInvocation;

    fn setup_prompt_submit_conn() -> Result<Connection> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        Ok(conn)
    }

    fn insert_prompt_submit_memory(
        conn: &Connection,
        project: &str,
        title: &str,
        content: &str,
    ) -> Result<i64> {
        let id = crate::memory::insert_memory(
            conn,
            Some("seed-session"),
            project,
            None,
            title,
            content,
            "decision",
            None,
        )?;
        conn.execute(
            "UPDATE memories SET source_trust_class = 'user_prompt' WHERE id = ?1",
            [id],
        )?;
        Ok(id)
    }

    fn record_prompt_event(
        conn: &Connection,
        project: &str,
        session_id: &str,
        content: &str,
    ) -> Result<()> {
        crate::db::record_captured_event(
            conn,
            &crate::db::CaptureEventInput {
                host: "claude-code",
                session_id,
                project,
                cwd: Some(project),
                event_type: "user_prompt_submit",
                role: Some("user"),
                tool_name: None,
                content,
                task_kind: Some(crate::db::ExtractionTaskKind::SessionRollup),
            },
        )?;
        Ok(())
    }

    #[test]
    fn prompt_submit_injects_relevant_memory() -> Result<()> {
        let conn = setup_prompt_submit_conn()?;
        let project = "/tmp/remem-prompt-submit";
        insert_prompt_submit_memory(
            &conn,
            project,
            "SQLCipher storage decision",
            "Persist private data with SQLCipher encryption at rest.",
        )?;

        let output = prompt_submit_additional_context(
            &conn,
            project,
            project,
            "sess-prompt-hit",
            "How should we protect private persisted data with SQLCipher?",
            Some("claude-code"),
        )?
        .expect("matching prompt should inject context");

        assert!(output.contains("SQLCipher storage decision"));
        assert!(output.contains("src=memory:#"));
        assert!(!output.contains("Persist private data with SQLCipher encryption at rest."));
        assert!(output.contains("open=get_observations"));
        assert!(output.contains("optional leads"));
        Ok(())
    }

    #[test]
    fn claude_repeated_continue_surfaces_continuity_only_once() -> Result<()> {
        let conn = setup_prompt_submit_conn()?;
        let project = "/tmp/remem-prompt-submit-continuity";
        crate::workstream::upsert_workstream(
            &conn,
            project,
            "memory-session-continuity",
            &crate::workstream::ParsedWorkStream {
                title: Some("Prompt-time recall rollout".to_string()),
                progress: Some("Codex hook contract verified".to_string()),
                next_action: Some("Implement compact candidate rendering".to_string()),
                blockers: None,
                is_completed: false,
            },
        )?;
        record_prompt_event(&conn, project, "sess-prompt-continuity", "continue")?;
        let first = prompt_submit_additional_context(
            &conn,
            project,
            project,
            "sess-prompt-continuity",
            "continue",
            Some("claude-code"),
        )?
        .ok_or_else(|| anyhow::anyhow!("first prompt should surface continuity"))?;
        assert!(first.contains("Prompt-time recall rollout"), "{first}");
        assert!(first.contains("first_turn_continuity"), "{first}");

        record_prompt_event(&conn, project, "sess-prompt-continuity", "continue")?;
        let later = prompt_submit_additional_context(
            &conn,
            project,
            project,
            "sess-prompt-continuity",
            "unrelated quantum telemetry",
            Some("claude-code"),
        )?;
        assert!(
            later.is_none(),
            "later prompt must omit continuity: {later:?}"
        );
        Ok(())
    }

    #[test]
    fn prompt_submit_indexes_fact_only_memory_without_preloading_fact_body() -> Result<()> {
        let conn = setup_prompt_submit_conn()?;
        let project = "/tmp/remem-prompt-submit-fact";
        let now = chrono::Utc::now().timestamp();
        let memory_id = insert_prompt_submit_memory(
            &conn,
            project,
            "Opaque signer source",
            "Structured fact only.",
        )?;
        conn.execute(
            "INSERT INTO memory_facts
             (project, subject, predicate, object, valid_from_epoch, valid_to_epoch,
              learned_at_epoch, source_memory_id, source_observation_id, source_event_ids,
              confidence, supersedes_fact_id, status, invalidated_at_epoch,
              created_at_epoch, updated_at_epoch)
             VALUES (?1, 'HarborMint', 'verified_by', 'Toma Reed', ?2, NULL, ?3, ?4,
                     NULL, '[]', 0.95, NULL, 'active', NULL, ?3, ?3)",
            params![project, now - 1_000, now - 900, memory_id],
        )?;

        let output = prompt_submit_additional_context(
            &conn,
            project,
            project,
            "sess-prompt-fact-hit",
            "Who signs HarborMint with Toma Reed?",
            Some("claude-code"),
        )?
        .ok_or_else(|| anyhow::anyhow!("fact-only prompt should inject context"))?;

        assert!(output.contains("Opaque signer source"), "{output}");
        assert!(!output.contains("Temporal facts:"), "{output}");
        assert!(
            !output.contains("HarborMint verified_by Toma Reed"),
            "{output}"
        );
        assert!(output.contains("open=get_observations"), "{output}");
        Ok(())
    }

    #[test]
    fn prompt_submit_marks_source_anchor_label_failures_as_errors() -> Result<()> {
        let conn = setup_prompt_submit_conn()?;
        let project = "/tmp/remem-prompt-submit-staleness-fallback";
        let memory_id = insert_prompt_submit_memory(
            &conn,
            project,
            "SQLCipher storage decision",
            "Persist private data with SQLCipher encryption at rest.",
        )?;
        conn.execute(
            "UPDATE memories SET files = '[not-json' WHERE id = ?1",
            [memory_id],
        )?;

        let output = prompt_submit_additional_context(
            &conn,
            project,
            project,
            "sess-prompt-staleness-fallback",
            "How should SQLCipher protect private persisted data?",
            Some("claude-code"),
        )?
        .ok_or_else(|| anyhow::anyhow!("prompt should still inject context"))?;

        assert!(output.contains("SQLCipher storage decision"), "{output}");
        assert!(output.contains("source_anchor=error"), "{output}");
        Ok(())
    }

    #[test]
    fn prompt_submit_abstains_for_unrelated_prompt() -> Result<()> {
        let conn = setup_prompt_submit_conn()?;
        let project = "/tmp/remem-prompt-submit-abstain";
        insert_prompt_submit_memory(
            &conn,
            project,
            "Legacy release checklist",
            "Legacy release checklist for cache warmup.",
        )?;

        let output = prompt_submit_additional_context(
            &conn,
            project,
            project,
            "sess-prompt-abstain",
            "Investigate quantum telemetry routing",
            Some("claude-code"),
        )?;

        assert!(output.is_none());
        let (channel, status): (String, String) = conn.query_row(
            "SELECT channel, status
             FROM context_injection_items
             WHERE session_id = 'sess-prompt-abstain'
             ORDER BY id DESC
             LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(channel, "prompt_submit");
        assert_eq!(status, "abstained");
        Ok(())
    }

    #[test]
    fn prompt_submit_does_not_resend_already_injected_memory() -> Result<()> {
        let conn = setup_prompt_submit_conn()?;
        let project = "/tmp/remem-prompt-submit-dedup";
        insert_prompt_submit_memory(
            &conn,
            project,
            "Migration locking fix",
            "Serialize startup migrations to prevent races.",
        )?;

        let first = prompt_submit_additional_context(
            &conn,
            project,
            project,
            "sess-prompt-dedup",
            "How do we fix startup migration races?",
            Some("claude-code"),
        )?;
        let second = prompt_submit_additional_context(
            &conn,
            project,
            project,
            "sess-prompt-dedup",
            "How do we fix startup migration races?",
            Some("claude-code"),
        )?;

        assert!(first.is_some());
        assert!(second.is_none());
        Ok(())
    }

    #[test]
    fn prompt_submit_does_not_resend_session_start_injected_memory() -> Result<()> {
        let conn = setup_prompt_submit_conn()?;
        let project = "/tmp/remem-prompt-submit-session-start-dedup";
        let memory_id = insert_prompt_submit_memory(
            &conn,
            project,
            "Migration locking fix",
            "Serialize startup migrations to prevent races.",
        )?;
        let memory = crate::memory::get_memories_by_ids(&conn, &[memory_id], Some(project))?
            .pop()
            .ok_or_else(|| anyhow::anyhow!("inserted memory should load"))?;
        let invocation = prompt_submit_test_invocation(project, "sess-session-start-dedup");
        let decision = ContextGateDecision {
            output: "# remem context\nMigration locking fix\n".into(),
            action: ContextGateAction::EmittedFull,
            reason: "first_or_forced",
            key: Some(injection_key_for_audit(&invocation)),
            context_hash: Some("seed-session-start-context".to_string()),
            output_mode: Some("full"),
            retained_context_chars: None,
            output_truncated: false,
        };
        record_context_injection_items(
            &conn,
            &invocation,
            &decision,
            &[ContextAuditItem::injected_memory(&memory, "core", 1)],
        )?;

        let output = prompt_submit_additional_context(
            &conn,
            project,
            project,
            "sess-session-start-dedup",
            "How do we fix startup migration races?",
            Some("claude-code"),
        )?;

        assert!(output.is_none());
        Ok(())
    }

    #[test]
    fn prompt_submit_ignores_output_level_context_gate_rows() -> Result<()> {
        let conn = setup_prompt_submit_conn()?;
        let project = "/tmp/remem-prompt-submit-gate-row";
        insert_prompt_submit_memory(
            &conn,
            project,
            "Migration locking fix",
            "Serialize startup migrations to prevent races.",
        )?;
        let invocation = prompt_submit_test_invocation(project, "sess-gate-row");
        let first = apply_context_gate_with_data_version(
            &conn,
            &invocation,
            "# remem context\nExisting SessionStart body\n".to_string(),
            None,
        );
        assert_eq!(first.action, ContextGateAction::EmittedFull);

        let output = prompt_submit_additional_context(
            &conn,
            project,
            project,
            "sess-gate-row",
            "How do we fix startup migration races?",
            Some("claude-code"),
        )?
        .ok_or_else(|| anyhow::anyhow!("prompt output should not be suppressed"))?;

        assert!(output.starts_with("# remem prompt candidate index"));
        assert!(output.contains("Migration locking fix"));
        assert!(!output.contains("# remem context delta"));
        Ok(())
    }

    #[test]
    fn prompt_submit_repeated_identical_inputs_are_byte_identical() -> Result<()> {
        let conn = setup_prompt_submit_conn()?;
        let project = "/tmp/remem-prompt-submit-deterministic";
        let memory_id = insert_prompt_submit_memory(
            &conn,
            project,
            "SQLCipher storage decision",
            "Persist private data with SQLCipher encryption at rest.",
        )?;
        conn.execute(
            "UPDATE memories SET updated_at_epoch = ?1 WHERE id = ?2",
            rusqlite::params![1_600_000_000_i64, memory_id],
        )?;

        let first = prompt_submit_additional_context(
            &conn,
            project,
            project,
            "sess-prompt-deterministic-1",
            "How should SQLCipher protect private persisted data?",
            Some("claude-code"),
        )?
        .ok_or_else(|| anyhow::anyhow!("first prompt should inject context"))?;
        let second = prompt_submit_additional_context(
            &conn,
            project,
            project,
            "sess-prompt-deterministic-2",
            "How should SQLCipher protect private persisted data?",
            Some("claude-code"),
        )?
        .ok_or_else(|| anyhow::anyhow!("second prompt should inject context"))?;

        assert_eq!(first, second);
        assert!(first.contains("staleness=old"), "{first}");
        Ok(())
    }

    #[test]
    fn prompt_submit_context_appends_without_rewriting_session_start_prefix() -> Result<()> {
        let conn = setup_prompt_submit_conn()?;
        let project = "/tmp/remem-prompt-submit-additive";
        insert_prompt_submit_memory(
            &conn,
            project,
            "SQLCipher storage decision",
            "Persist private data with SQLCipher encryption at rest.",
        )?;
        let session_start_prefix = "# remem context\n\n## Core\nStable startup prefix\n\n";

        let prompt_context = prompt_submit_additional_context(
            &conn,
            project,
            project,
            "sess-prompt-additive",
            "How should SQLCipher protect private persisted data?",
            Some("claude-code"),
        )?
        .ok_or_else(|| anyhow::anyhow!("prompt should inject context"))?;
        let combined = format!("{session_start_prefix}{prompt_context}");

        assert!(combined.starts_with(session_start_prefix));
        assert_eq!(
            &combined.as_bytes()[..session_start_prefix.len()],
            session_start_prefix.as_bytes()
        );
        assert!(prompt_context.starts_with("# remem prompt candidate index"));
        assert!(!prompt_context.contains("# remem context\n\n## Core"));
        Ok(())
    }

    #[test]
    fn prompt_submit_p95_latency_stays_under_budget() -> Result<()> {
        let _data_dir = crate::db::test_support::ScopedTestDataDir::new("prompt-submit-latency");
        let conn = setup_prompt_submit_conn()?;
        let project = "/tmp/remem-prompt-submit-latency";
        insert_prompt_submit_memory(
            &conn,
            project,
            "SQLCipher storage decision",
            "Persist private data with SQLCipher encryption at rest.",
        )?;
        let mut durations = Vec::new();
        for idx in 0..20 {
            let start = Instant::now();
            let output = prompt_submit_additional_context(
                &conn,
                project,
                project,
                &format!("sess-prompt-latency-{idx}"),
                "How should SQLCipher protect private persisted data?",
                Some("claude-code"),
            )?;
            assert!(output.is_some());
            durations.push(start.elapsed().as_millis());
        }
        durations.sort_unstable();
        let p95 = durations[(durations.len() * 95).div_ceil(100) - 1];
        assert!(
            p95 <= PROMPT_SUBMIT_LATENCY_BUDGET_MS,
            "p95 {p95}ms exceeded {PROMPT_SUBMIT_LATENCY_BUDGET_MS}ms"
        );
        Ok(())
    }

    #[test]
    fn prompt_submit_does_not_inject_legacy_unverified_memory() -> Result<()> {
        let conn = setup_prompt_submit_conn()?;
        let project = "/tmp/remem-prompt-submit-g2";
        let memory_id = crate::memory::insert_memory(
            &conn,
            Some("seed-session"),
            project,
            None,
            "SQLCipher storage decision",
            "Persist private data with SQLCipher encryption at rest.",
            "decision",
            None,
        )?;
        conn.execute(
            "UPDATE memories
             SET source_trust_class = 'local_tool_output',
                 source_candidate_id = NULL,
                 evidence_event_ids = NULL,
                 confidence = NULL,
                 valid_from_epoch = NULL
             WHERE id = ?1",
            [memory_id],
        )?;

        let output = prompt_submit_additional_context(
            &conn,
            project,
            project,
            "sess-prompt-g2",
            "How should we protect private persisted data with SQLCipher?",
            Some("claude-code"),
        )?;
        assert!(output.is_none());
        let drop_reason: String = conn.query_row(
            "SELECT drop_reason
             FROM context_injection_items
             WHERE session_id = 'sess-prompt-g2'
               AND status = 'dropped'
             ORDER BY id DESC
             LIMIT 1",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(drop_reason, "legacy_unverified_provenance_missing");
        Ok(())
    }

    fn prompt_submit_test_invocation(project: &str, session_id: &str) -> ContextInvocation {
        ContextInvocation {
            cwd: project.to_string(),
            project: project.to_string(),
            session_id: Some(session_id.to_string()),
            transcript_path: None,
            source: Some("SessionStart".to_string()),
            host: HostKind::ClaudeCode,
            use_colors: false,
            debug: false,
            force: false,
            gate_mode: None,
        }
    }
}
