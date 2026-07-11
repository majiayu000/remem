use std::collections::{HashMap, HashSet};

use anyhow::Result;
use rusqlite::params;

use crate::memory::Memory;

use super::audit::{
    memory_render_metadata_with_labels, record_context_injection_items, ContextAuditItem,
};
use super::fact_labels::annotate_memories_with_temporal_facts_for_query;
use super::format::{char_len, format_epoch_short, truncate_chars_with_ellipsis};
use super::host::resolve_host_kind;
use super::hybrid_context::query_hybrid_context_memories;
use super::injection_gate::{injection_key_for_audit, ContextGateAction, ContextGateDecision};
use super::invocation::ContextInvocation;
use super::policy::{ContextLimits, ContextPolicy, SectionKind};

const PROMPT_SUBMIT_MEMORY_LIMIT: i64 = 3;
const PROMPT_SUBMIT_CHAR_LIMIT: usize = 1_800;
const PROMPT_SUBMIT_PREVIEW_CHARS: usize = 240;
#[cfg(test)]
const PROMPT_SUBMIT_LATENCY_BUDGET_MS: u128 = 250;

pub(crate) fn prompt_submit_additional_context(
    conn: &rusqlite::Connection,
    cwd: &str,
    project: &str,
    session_id: &str,
    prompt: &str,
    host_arg: Option<&str>,
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
    let mut retrieved = query_hybrid_context_memories(
        conn,
        project,
        prompt,
        current_branch.as_deref(),
        excluded_types,
        PROMPT_SUBMIT_MEMORY_LIMIT,
    )?;
    annotate_memories_with_temporal_facts_for_query(
        conn,
        &mut retrieved,
        Some(prompt),
        Some(project),
    )?;
    let already_injected = query_previously_injected_memory_ids(conn, &invocation)?;
    let mut rendered = Vec::new();
    let mut audit_items = Vec::new();
    for memory in retrieved {
        if already_injected.contains(&memory.id) {
            audit_items.push(ContextAuditItem::dropped_memory(
                &memory,
                "prompt_submit",
                "already_injected",
            ));
        } else if !prompt_relevance_passes(prompt, &memory) {
            audit_items.push(ContextAuditItem::dropped_memory(
                &memory,
                "prompt_submit",
                "below_prompt_relevance_threshold",
            ));
        } else {
            rendered.push(memory);
        }
    }

    if rendered.is_empty() {
        if audit_items.is_empty() {
            audit_items.push(prompt_submit_abstained_item(
                "prompt_submit_no_relevant_context",
            ));
        }
        let decision = empty_prompt_submit_decision();
        record_context_injection_items(conn, &invocation, &decision, &audit_items)?;
        return Ok(None);
    }

    let render_reference_epoch = chrono::Utc::now().timestamp();
    let staleness_labels = prompt_submit_staleness_labels(conn, &rendered, render_reference_epoch);
    audit_items.extend(rendered.iter().enumerate().map(|(index, memory)| {
        ContextAuditItem::injected_memory_with_labels(
            memory,
            "prompt_submit",
            index as i64 + 1,
            &staleness_labels,
        )
    }));
    let output = render_prompt_submit_context(&rendered, &staleness_labels, render_reference_epoch);
    let decision = prompt_submit_decision(output);
    record_context_injection_items(conn, &invocation, &decision, &audit_items)?;
    Ok(Some(decision.output))
}

fn query_previously_injected_memory_ids(
    conn: &rusqlite::Connection,
    invocation: &ContextInvocation,
) -> Result<HashSet<i64>> {
    let key = injection_key_for_audit(invocation);
    let Some(session_id) = invocation.session_id.as_deref() else {
        return Ok(HashSet::new());
    };
    let mut stmt = conn.prepare(
        "SELECT DISTINCT memory_id
         FROM context_injection_items
         WHERE host = ?1
           AND project = ?2
           AND session_id = ?3
           AND injection_key = ?4
           AND status = 'injected'
           AND memory_id IS NOT NULL",
    )?;
    let rows = stmt.query_map(
        params![
            invocation.host.as_env_value(),
            invocation.project,
            session_id,
            key
        ],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(crate::db::query::collect_rows(rows)?.into_iter().collect())
}

fn prompt_relevance_passes(prompt: &str, memory: &Memory) -> bool {
    let prompt_tokens = significant_prompt_tokens(prompt);
    if prompt_tokens.is_empty() {
        return false;
    }
    let memory_text = format!("{} {}", memory.title, memory.text).to_lowercase();
    prompt_tokens
        .iter()
        .any(|token| memory_text.contains(token))
}

fn render_prompt_submit_context(
    memories: &[Memory],
    staleness_labels: &HashMap<i64, crate::memory::MemoryStalenessLabel>,
    render_reference_epoch: i64,
) -> String {
    let mut output = String::from("# remem prompt context\n\n## Relevant Memories\n");
    output.push_str(crate::memory::usage::citation_contract_line());
    output.push('\n');
    output.push_str(crate::user_context::usage_policy::USER_CONTEXT_USAGE_POLICY);
    output.push('\n');
    for memory in memories {
        let header = format!(
            "**#{} {}** ({}, {}; {})\n",
            memory.id,
            memory.title,
            memory.memory_type,
            format_epoch_short(memory.updated_at_epoch),
            memory_render_metadata_with_labels(memory, render_reference_epoch, staleness_labels)
        );
        if char_len(&output) + char_len(&header) >= PROMPT_SUBMIT_CHAR_LIMIT {
            break;
        }
        output.push_str(&header);
        let remaining = PROMPT_SUBMIT_CHAR_LIMIT.saturating_sub(char_len(&output) + 1);
        let preview =
            truncate_chars_with_ellipsis(&memory.text, remaining.min(PROMPT_SUBMIT_PREVIEW_CHARS));
        if !preview.is_empty() {
            output.push_str(&preview);
            output.push('\n');
        }
    }
    output
}

fn prompt_submit_staleness_labels(
    conn: &rusqlite::Connection,
    memories: &[Memory],
    render_reference_epoch: i64,
) -> HashMap<i64, crate::memory::MemoryStalenessLabel> {
    crate::memory::staleness::memory_staleness_labels_for_memories_lossy(
        conn,
        memories,
        render_reference_epoch,
        |id, error| {
            crate::log::error(
                "context",
                &format!("prompt-submit source-anchor label failed for memory {id}: {error}"),
            );
        },
    )
    .unwrap_or_else(|error| {
        crate::log::error(
            "context",
            &format!("prompt-submit staleness batch failed: {error}"),
        );
        memories
            .iter()
            .map(|memory| {
                (
                    memory.id,
                    crate::memory::memory_staleness_error_label(
                        memory,
                        render_reference_epoch,
                        &error,
                    ),
                )
            })
            .collect()
    })
}

fn empty_prompt_submit_decision() -> ContextGateDecision {
    ContextGateDecision {
        output: String::new(),
        action: ContextGateAction::Bypassed,
        reason: "prompt_submit_empty",
        key: None,
        context_hash: None,
        output_mode: Some("prompt_submit"),
    }
}

fn prompt_submit_decision(output: String) -> ContextGateDecision {
    ContextGateDecision {
        output,
        action: ContextGateAction::Bypassed,
        reason: "prompt_submit",
        key: None,
        context_hash: None,
        output_mode: Some("prompt_submit"),
    }
}

fn significant_prompt_tokens(text: &str) -> HashSet<String> {
    text.split_whitespace()
        .map(|token| {
            token
                .trim_matches(|ch: char| ch.is_ascii_punctuation())
                .to_lowercase()
        })
        .filter(|token| token.chars().count() >= 3 || !token.is_ascii())
        .filter(|token| !is_prompt_stop_token(token))
        .collect()
}

fn is_prompt_stop_token(token: &str) -> bool {
    matches!(
        token,
        "the"
            | "and"
            | "for"
            | "with"
            | "from"
            | "that"
            | "this"
            | "should"
            | "when"
            | "where"
            | "what"
            | "why"
            | "how"
            | "into"
            | "was"
            | "were"
            | "does"
            | "need"
            | "about"
    )
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
        crate::memory::insert_memory(
            conn,
            Some("seed-session"),
            project,
            None,
            title,
            content,
            "decision",
            None,
        )
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
        assert_eq!(
            output
                .matches(crate::user_context::usage_policy::USER_CONTEXT_USAGE_POLICY)
                .count(),
            1
        );
        Ok(())
    }

    #[test]
    fn prompt_submit_injects_fact_only_memory_with_temporal_label() -> Result<()> {
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
        assert!(output.contains("Temporal facts:"), "{output}");
        assert!(
            output.contains("HarborMint verified_by Toma Reed"),
            "{output}"
        );
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

        assert!(output.starts_with("# remem prompt context"));
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
    fn prompt_submit_renderer_uses_supplied_reference_epoch() {
        let memory = Memory {
            id: 1,
            session_id: None,
            project: "/tmp/remem-prompt-submit-reference".to_string(),
            topic_key: None,
            title: "Older memory".to_string(),
            text: "Body".to_string(),
            memory_type: "decision".to_string(),
            files: None,
            created_at_epoch: 1_500_000_000,
            updated_at_epoch: 1_500_000_000,
            status: "active".to_string(),
            branch: None,
            scope: "project".to_string(),
        };
        let staleness_labels = HashMap::new();

        let fresh = render_prompt_submit_context(
            &[memory.clone()],
            &staleness_labels,
            memory.updated_at_epoch,
        );
        let fresh_again = render_prompt_submit_context(
            &[memory.clone()],
            &staleness_labels,
            memory.updated_at_epoch,
        );
        let old =
            render_prompt_submit_context(&[memory], &staleness_labels, 1_500_000_000 + 91 * 86_400);

        assert_eq!(fresh, fresh_again);
        assert!(fresh.contains("staleness=fresh"), "{fresh}");
        assert!(old.contains("staleness=old"), "{old}");
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
        assert!(prompt_context.starts_with("# remem prompt context"));
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
