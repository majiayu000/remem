const DEFAULT_DELTA_CHAR_LIMIT: usize = 1200;
const DELTA_TRUNCATED_MARKER: &str = "\n[remem context delta truncated]\n";

pub(super) struct DeltaOutput {
    pub output: String,
    pub retained_context_chars: usize,
}

pub(super) fn build_delta_output(output: &str, item_end_chars: &[usize]) -> DeltaOutput {
    let limit = read_usize_env("REMEM_CONTEXT_DELTA_CHAR_LIMIT", DEFAULT_DELTA_CHAR_LIMIT);
    build_delta_output_with_limit(output, item_end_chars, limit)
}

fn build_delta_output_with_limit(
    output: &str,
    item_end_chars: &[usize],
    limit: usize,
) -> DeltaOutput {
    if limit == 0 {
        return DeltaOutput {
            output: String::new(),
            retained_context_chars: 0,
        };
    }

    let (body, footer) = split_stats_footer(output);
    let (header, body_without_header) = delta_header_and_body(body);
    let copied_body = body_without_header.trim_start_matches('\n');
    let original_body_offset = body.chars().count() - copied_body.chars().count();
    let mut delta = String::new();
    delta.push_str(&header);
    delta.push_str(
        "Context changed since the previous injection. Showing a compact preview. Full context: `remem context --force`.\n\n",
    );
    let delta_prefix_chars = delta.chars().count();
    delta.push_str(copied_body);

    let was_truncated = delta.chars().count() > limit;
    let retained_delta_chars = enforce_char_limit_preserving_footer(&mut delta, limit, footer);
    let retained_copied_chars = retained_delta_chars
        .saturating_sub(delta_prefix_chars)
        .min(copied_body.chars().count());
    let retained_context_chars = original_body_offset + retained_copied_chars;
    if was_truncated && !item_end_chars.is_empty() {
        let boundary = item_end_chars
            .iter()
            .copied()
            .filter(|end| *end <= retained_context_chars)
            .max()
            .unwrap_or(0);
        if boundary < retained_context_chars {
            if boundary == 0 && retained_copied_chars == 0 {
                return DeltaOutput {
                    output: delta,
                    retained_context_chars: 0,
                };
            }
            let retained_copied_chars = boundary
                .saturating_sub(original_body_offset)
                .min(copied_body.chars().count());
            let copied_prefix = copied_body
                .chars()
                .take(retained_copied_chars)
                .collect::<String>();
            delta.clear();
            delta.push_str(&header);
            delta.push_str(
                "Context changed since the previous injection. Showing a compact preview. Full context: `remem context --force`.\n\n",
            );
            delta.push_str(&copied_prefix);
            let marker_chars = DELTA_TRUNCATED_MARKER.chars().count();
            let footer_chars = footer.chars().count();
            if marker_chars < limit {
                delta.push_str(DELTA_TRUNCATED_MARKER);
                if !footer.is_empty() && marker_chars + footer_chars < limit {
                    delta.push_str(footer);
                }
            }
            debug_assert!(delta.chars().count() <= limit);
            return DeltaOutput {
                output: delta,
                retained_context_chars: boundary,
            };
        }
    }
    DeltaOutput {
        output: delta,
        retained_context_chars,
    }
}

fn split_stats_footer(output: &str) -> (&str, &str) {
    let trimmed = output.trim_end_matches('\n');
    let mut offset = 0;
    for segment in trimmed.split_inclusive('\n') {
        let footer = &output[offset..];
        if super::is_context_stats_footer(footer.trim_end_matches('\n')) {
            return (&output[..offset], footer);
        }
        offset += segment.len();
    }
    (output, "")
}

fn delta_header_and_body(output: &str) -> (String, &str) {
    if let Some(delta) = boxed_delta_header_and_body(output) {
        return delta;
    }

    let first_line = output.lines().next().unwrap_or("# remem context");
    let first_line = super::super::style::strip_ansi(first_line);
    if first_line == "# remem context" || first_line == "remem context" {
        let body = output
            .split_once('\n')
            .map(|(_, rest)| rest)
            .unwrap_or_default();
        return (
            super::super::style::context_delta_title_line_like(
                first_line.as_str(),
                super::super::style::contains_ansi(output),
            ),
            body,
        );
    }
    ("# remem context delta\n\n".to_string(), output)
}

fn boxed_delta_header_and_body(output: &str) -> Option<(String, &str)> {
    let use_colors = super::super::style::contains_ansi(output);
    let mut header = String::new();
    let mut offset = 0;

    for (idx, segment) in output.split_inclusive('\n').enumerate() {
        let line = segment.trim_end_matches('\n');
        let plain_line = super::super::style::strip_ansi(line);
        if idx == 0 {
            if !plain_line.starts_with("╭─ remem context")
                && !plain_line.starts_with("┌─ remem context")
            {
                return None;
            }
            header.push_str(&super::super::style::context_delta_title_line_like(
                line, use_colors,
            ));
            offset += segment.len();
            continue;
        }

        header.push_str(segment);
        offset += segment.len();
        if plain_line.starts_with('╰') && plain_line.ends_with('╯') {
            if output[offset..].starts_with('\n') {
                header.push('\n');
                offset += 1;
            }
            return Some((header, &output[offset..]));
        }
        if plain_line.starts_with("└─ ") {
            return Some((header, &output[offset..]));
        }
    }

    None
}

fn enforce_char_limit_preserving_footer(
    output: &mut String,
    char_limit: usize,
    footer: &str,
) -> usize {
    if output.chars().count() <= char_limit {
        return output
            .strip_suffix(footer)
            .unwrap_or(output.as_str())
            .chars()
            .count();
    }

    let marker_chars = DELTA_TRUNCATED_MARKER.chars().count();
    let footer_chars = footer.chars().count();

    if !footer.is_empty() && marker_chars + footer_chars < char_limit {
        let keep_chars = char_limit - marker_chars - footer_chars;
        let mut truncated: String = output.chars().take(keep_chars).collect();
        truncated.push_str(DELTA_TRUNCATED_MARKER);
        truncated.push_str(footer);
        *output = truncated;
        return keep_chars;
    }

    if marker_chars >= char_limit {
        *output = output.chars().take(char_limit).collect();
        return 0;
    }

    let keep_chars = char_limit - marker_chars;
    let mut truncated: String = output.chars().take(keep_chars).collect();
    truncated.push_str(DELTA_TRUNCATED_MARKER);
    *output = truncated;
    keep_chars
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
    use super::build_delta_output_with_limit;
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

    fn apply_test_context_gate(
        invocation: &ContextInvocation,
        output: String,
    ) -> ContextGateDecision {
        let conn = match crate::db::test_support::runtime_connection() {
            Ok(conn) => conn,
            Err(error) => panic!("test database should open: {error:#}"),
        };
        apply_context_gate(&conn, invocation, output)
    }

    #[test]
    fn delta_mode_emits_compact_changed_hash() -> Result<()> {
        let _data_dir = crate::db::test_support::ScopedTestDataDir::new("context-gate-delta");
        let mut invocation = invocation(Some("sess-delta"));
        invocation.gate_mode = Some("delta".to_string());
        let first = apply_test_context_gate(
            &invocation,
            "# [/tmp/remem] context now\nBody A\n".to_string(),
        );
        assert_eq!(first.action, ContextGateAction::EmittedFull);

        let changed_output = format!(
            "# [/tmp/remem] context later\n{}\n\n1 context memories loaded. 1 core (10 chars). 0 lessons (0 chars). 0 indexed (0 chars). 0 preferences (project:0 global:0, 0 chars). 0 sessions (0 chars). host=codex-cli branch=main total=3000 chars/~750 tokens limit=12000 truncated=no\n",
            "Body B ".repeat(400)
        );
        let second = apply_test_context_gate(&invocation, changed_output.clone());
        assert_eq!(second.action, ContextGateAction::EmittedDelta);
        assert_eq!(second.reason, "changed_hash");
        assert_ne!(second.output, changed_output);
        assert!(second.output.contains("context delta"));
        assert!(second.output.chars().count() <= 1200);

        let key = injection_key(&invocation);
        let mode: String = crate::db::test_support::runtime_connection()?.query_row(
            "SELECT output_mode FROM context_injections WHERE host = ?1 AND injection_key = ?2",
            params![invocation.host.as_env_value(), key],
            |row| row.get(0),
        )?;
        assert_eq!(mode, "delta");
        Ok(())
    }

    #[test]
    fn delta_limit_rewinds_to_complete_item_boundary() {
        let header = "# remem context\n\n";
        let first = "- retained preference\n";
        let second = format!("- clipped memory {}\n", "x".repeat(600));
        let output = format!("{header}{first}{second}");
        let first_end = format!("{header}{first}").chars().count();
        let all_end = output.chars().count();

        let delta = build_delta_output_with_limit(&output, &[first_end, all_end], 240);

        assert_eq!(delta.retained_context_chars, first_end);
        assert!(delta.output.contains("retained preference"));
        assert!(!delta.output.contains("clipped memory"));
        assert!(delta.output.contains("context delta truncated"));
        assert!(delta.output.chars().count() <= 240);
    }

    #[test]
    fn delta_boundary_rewind_drops_oversized_stats_footer() {
        let header = "# remem context\n\n";
        let first = "- retained preference\n";
        let second = format!("- clipped memory {}\n", "x".repeat(600));
        let footer = format!(
            "## Loaded\n- Memories: 2\n- Preferences: 1\n- Budget: {}\n",
            "y".repeat(300)
        );
        let output = format!("{header}{first}{second}{footer}");
        let first_end = format!("{header}{first}").chars().count();
        let all_end = format!("{header}{first}{second}").chars().count();

        let delta = build_delta_output_with_limit(&output, &[first_end, all_end], 240);

        assert_eq!(delta.retained_context_chars, first_end);
        assert!(delta.output.contains("retained preference"));
        assert!(!delta.output.contains("clipped memory"));
        assert!(!delta.output.contains("## Loaded"));
        assert!(delta.output.contains("context delta truncated"));
        assert!(delta.output.chars().count() <= 240);
    }

    #[test]
    fn delta_under_limit_keeps_trailing_non_item_text() {
        let output = "# remem context\n\n- retained preference\n\nLoaded: 1\n";
        let item_end = "# remem context\n\n- retained preference\n".chars().count();

        let delta = build_delta_output_with_limit(output, &[item_end], 500);

        assert_eq!(delta.retained_context_chars, output.chars().count());
        assert!(delta.output.contains("Loaded: 1"));
        assert!(!delta.output.contains("context delta truncated"));
    }

    #[test]
    fn auto_mode_emits_delta_on_changed_hash() {
        let _data_dir = crate::db::test_support::ScopedTestDataDir::new("context-gate-auto-delta");
        let invocation = invocation(Some("sess-auto-delta"));
        let first = apply_test_context_gate(
            &invocation,
            "# [/tmp/remem] context now\nBody A\n".to_string(),
        );
        assert_eq!(first.action, ContextGateAction::EmittedFull);

        let second = apply_test_context_gate(
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
        let first = apply_test_context_gate(&invocation, output.clone());
        assert_eq!(first.action, ContextGateAction::EmittedFull);

        let key = injection_key(&invocation);
        crate::db::test_support::runtime_connection()?.execute(
            "UPDATE context_injections
             SET last_emitted_epoch = 0
             WHERE host = ?1 AND injection_key = ?2",
            params![invocation.host.as_env_value(), key],
        )?;

        let second = apply_test_context_gate(&invocation, output.clone());
        assert_eq!(second.action, ContextGateAction::EmittedFull);
        assert_eq!(second.reason, "fallback_cooldown_expired");
        assert_eq!(second.output, output);
        Ok(())
    }
}
