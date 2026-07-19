use super::super::invocation::ContextInvocation;
use super::super::policy::{ContextLimits, ContextPolicy, SectionKind};
use super::truncation::truncate_context_body_at_stable_boundary;
use super::{ContextRenderStats, Result};

pub(super) fn section_render_limits(policy: &ContextPolicy) -> ContextLimits {
    let mut limits = policy.limits;
    limits.core_item_limit = policy.section_item_limit(SectionKind::Core, limits.core_item_limit);
    limits.core_char_limit = policy.section_char_limit(SectionKind::Core, limits.core_char_limit);
    limits.memory_index_limit =
        policy.section_item_limit(SectionKind::MemoryIndex, limits.memory_index_limit);
    limits.memory_index_char_limit =
        policy.section_char_limit(SectionKind::MemoryIndex, limits.memory_index_char_limit);
    limits
}

#[cfg(test)]
pub(in crate::context) fn build_context_stats_footer(stats: &ContextRenderStats) -> String {
    build_context_stats_footer_with_style(stats, false)
}

pub(super) fn build_context_stats_footer_with_style(
    stats: &ContextRenderStats,
    use_colors: bool,
) -> String {
    super::super::style::context_stats_footer(stats, use_colors)
}

pub(super) fn context_source_note(source: Option<&str>) -> Option<&'static str> {
    match source?.trim().to_ascii_lowercase().as_str() {
        "compact" => Some("Codex compacted the chat, so remem refreshed memory context."),
        "clear" => Some("Context was reloaded after an explicit clear."),
        _ => None,
    }
}

pub(super) fn context_debug_enabled() -> bool {
    std::env::var("REMEM_CONTEXT_DEBUG")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

#[cfg(test)]
pub(in crate::context) fn enforce_total_char_limit(output: &mut String, char_limit: usize) {
    enforce_total_char_limit_preserving_footer(output, char_limit, "");
}

pub(in crate::context) fn enforce_total_char_limit_preserving_footer(
    output: &mut String,
    char_limit: usize,
    footer: &str,
) {
    if char_limit == 0 || output.chars().count() <= char_limit {
        return;
    }

    let marker = "\n[remem context truncated to REMEM_CONTEXT_TOTAL_CHAR_LIMIT]\n";
    let marker_chars = marker.chars().count();
    let footer_chars = footer.chars().count();

    if !footer.is_empty() && output.ends_with(footer) && marker_chars + footer_chars <= char_limit {
        let keep_chars = char_limit - marker_chars - footer_chars;
        let body = output.strip_suffix(footer).unwrap_or(output.as_str());
        let mut truncated = truncate_context_body_at_stable_boundary(body, keep_chars);
        truncated.push_str(marker);
        truncated.push_str(footer);
        *output = truncated;
        return;
    }

    if marker_chars >= char_limit {
        *output = marker.chars().take(char_limit).collect();
        return;
    }

    let keep_chars = char_limit - marker_chars;
    let mut truncated = truncate_context_body_at_stable_boundary(output, keep_chars);
    truncated.push_str(marker);
    *output = truncated;
}

pub(super) fn build_context_header_with_style(
    project: &str,
    current_branch: Option<&str>,
    hook_source: Option<&str>,
    host: super::super::host::HostKind,
    use_colors: bool,
) -> String {
    super::super::style::context_header(project, current_branch, hook_source, host, use_colors)
}

pub(in crate::context) fn context_stdout_for_invocation(
    output: &str,
    invocation: &ContextInvocation,
) -> Result<String> {
    if output.is_empty() || !is_codex_session_start_hook(invocation) {
        return Ok(output.to_string());
    }

    let additional_context = super::super::style::strip_ansi(output);
    let hook_output = serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "SessionStart",
            "additionalContext": additional_context,
        }
    });
    Ok(format!("{}\n", serde_json::to_string(&hook_output)?))
}

fn is_codex_session_start_hook(invocation: &ContextInvocation) -> bool {
    if invocation.host != super::super::host::HostKind::CodexCli {
        return false;
    }

    matches!(
        invocation
            .source
            .as_deref()
            .map(|source| source.trim().to_ascii_lowercase()),
        Some(source)
            if matches!(source.as_str(), "startup" | "resume" | "clear" | "compact")
    )
}
