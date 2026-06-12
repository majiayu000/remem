use std::collections::HashSet;

use anyhow::Result;
use rusqlite::params;

use crate::memory::Memory;

use super::audit::{memory_render_metadata, record_context_injection_items, ContextAuditItem};
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
    let retrieved = query_hybrid_context_memories(
        conn,
        project,
        prompt,
        current_branch.as_deref(),
        excluded_types,
        PROMPT_SUBMIT_MEMORY_LIMIT,
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

    audit_items.extend(rendered.iter().enumerate().map(|(index, memory)| {
        ContextAuditItem::injected_memory(memory, "prompt_submit", index as i64 + 1)
    }));
    let output = render_prompt_submit_context(&rendered);
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

fn render_prompt_submit_context(memories: &[Memory]) -> String {
    let mut output = String::from("# remem prompt context\n\n## Relevant Memories\n");
    let now = chrono::Utc::now().timestamp();
    for memory in memories {
        let header = format!(
            "**#{} {}** ({}, {}; {})\n",
            memory.id,
            memory.title,
            memory.memory_type,
            format_epoch_short(memory.updated_at_epoch),
            memory_render_metadata(memory, now)
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
    fn prompt_submit_p95_latency_stays_under_budget() -> Result<()> {
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
