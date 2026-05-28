const DEFAULT_DELTA_CHAR_LIMIT: usize = 1200;

pub(super) fn build_delta_output(output: &str) -> String {
    let limit = read_usize_env("REMEM_CONTEXT_DELTA_CHAR_LIMIT", DEFAULT_DELTA_CHAR_LIMIT);
    if limit == 0 {
        return String::new();
    }

    let (body, footer) = split_stats_footer(output);
    let mut delta = String::new();
    delta.push_str(&delta_header(body));
    delta.push_str(
        "remem context changed since the previous injection. Compact delta shown; run `remem context --force` for a full refresh.\n\n",
    );
    let body_without_header = body
        .split_once('\n')
        .map(|(_, rest)| rest)
        .unwrap_or_default();
    delta.push_str(body_without_header.trim_start_matches('\n'));

    enforce_char_limit_preserving_footer(&mut delta, limit, footer);
    delta
}

fn split_stats_footer(output: &str) -> (&str, &str) {
    let Some(last_line_start) = output.trim_end_matches('\n').rfind('\n') else {
        return (output, "");
    };
    let footer_start = last_line_start + 1;
    let footer = &output[footer_start..];
    if super::is_context_stats_footer(footer.trim_end_matches('\n')) {
        (&output[..footer_start], footer)
    } else {
        (output, "")
    }
}

fn delta_header(output: &str) -> String {
    let first_line = output.lines().next().unwrap_or("# remem context");
    if let Some(context_idx) = first_line.find("] context ") {
        let mut header = String::new();
        header.push_str(&first_line[..context_idx]);
        header.push_str("] context delta ");
        header.push_str(&first_line[context_idx + "] context ".len()..]);
        header.push('\n');
        return header;
    }
    "# remem context delta\n".to_string()
}

fn enforce_char_limit_preserving_footer(output: &mut String, char_limit: usize, footer: &str) {
    if output.chars().count() <= char_limit {
        return;
    }

    let marker = "\n[remem context delta truncated]\n";
    let marker_chars = marker.chars().count();
    let footer_chars = footer.chars().count();

    if !footer.is_empty() && marker_chars + footer_chars < char_limit {
        let keep_chars = char_limit - marker_chars - footer_chars;
        let mut truncated: String = output.chars().take(keep_chars).collect();
        truncated.push_str(marker);
        truncated.push_str(footer);
        *output = truncated;
        return;
    }

    if marker_chars >= char_limit {
        *output = output.chars().take(char_limit).collect();
        return;
    }

    let keep_chars = char_limit - marker_chars;
    let mut truncated: String = output.chars().take(keep_chars).collect();
    truncated.push_str(marker);
    *output = truncated;
}

fn read_usize_env(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use anyhow::Result;
    use rusqlite::params;

    fn invocation(session_id: Option<&str>) -> ContextInvocation {
        ContextInvocation {
            cwd: "/tmp/remem".to_string(),
            project: "/tmp/remem".to_string(),
            session_id: session_id.map(str::to_string),
            transcript_path: Some("/tmp/remem.jsonl".to_string()),
            source: None,
            host: HostKind::CodexCli,
            use_colors: false,
            debug: false,
            force: false,
            gate_mode: None,
        }
    }

    #[test]
    fn delta_mode_emits_compact_changed_hash() -> Result<()> {
        let _data_dir = crate::db::test_support::ScopedTestDataDir::new("context-gate-delta");
        let mut invocation = invocation(Some("sess-delta"));
        invocation.gate_mode = Some("delta".to_string());
        let first = apply_context_gate(
            &invocation,
            "# [/tmp/remem] context now\nBody A\n".to_string(),
        );
        assert_eq!(first.action, ContextGateAction::EmittedFull);

        let changed_output = format!(
            "# [/tmp/remem] context later\n{}\n\n1 context memories loaded. 1 core (10 chars). 0 lessons (0 chars). 0 indexed (0 chars). 0 preferences (project:0 global:0, 0 chars). 0 sessions (0 chars). host=codex-cli branch=main total=3000 chars/~750 tokens limit=12000 truncated=no\n",
            "Body B ".repeat(400)
        );
        let second = apply_context_gate(&invocation, changed_output.clone());
        assert_eq!(second.action, ContextGateAction::EmittedDelta);
        assert_eq!(second.reason, "changed_hash");
        assert_ne!(second.output, changed_output);
        assert!(second.output.contains("context delta"));
        assert!(second.output.chars().count() <= 1200);

        let key = injection_key(&invocation);
        let mode: String = crate::db::open_db()?.query_row(
            "SELECT output_mode FROM context_injections WHERE host = ?1 AND injection_key = ?2",
            params![invocation.host.as_env_value(), key],
            |row| row.get(0),
        )?;
        assert_eq!(mode, "delta");
        Ok(())
    }

    #[test]
    fn auto_mode_emits_delta_on_changed_hash() {
        let _data_dir = crate::db::test_support::ScopedTestDataDir::new("context-gate-auto-delta");
        let invocation = invocation(Some("sess-auto-delta"));
        let first = apply_context_gate(
            &invocation,
            "# [/tmp/remem] context now\nBody A\n".to_string(),
        );
        assert_eq!(first.action, ContextGateAction::EmittedFull);

        let second = apply_context_gate(
            &invocation,
            "# [/tmp/remem] context now\nBody B\n".to_string(),
        );
        assert_eq!(second.action, ContextGateAction::EmittedDelta);
        assert_eq!(second.reason, "changed_hash");
    }

    #[test]
    fn fallback_cooldown_expiry_reemits_full_for_same_hash() -> Result<()> {
        let _data_dir =
            crate::db::test_support::ScopedTestDataDir::new("context-gate-fallback-expired");
        let invocation = invocation(None);
        let output = "# [/tmp/remem] context now\nBody\n".to_string();
        let first = apply_context_gate(&invocation, output.clone());
        assert_eq!(first.action, ContextGateAction::EmittedFull);

        let key = injection_key(&invocation);
        crate::db::open_db()?.execute(
            "UPDATE context_injections
             SET last_emitted_epoch = 0
             WHERE host = ?1 AND injection_key = ?2",
            params![invocation.host.as_env_value(), key],
        )?;

        let second = apply_context_gate(&invocation, output.clone());
        assert_eq!(second.action, ContextGateAction::EmittedFull);
        assert_eq!(second.reason, "fallback_cooldown_expired");
        assert_eq!(second.output, output);
        Ok(())
    }
}
