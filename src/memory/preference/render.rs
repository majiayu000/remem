use anyhow::Result;
use rusqlite::Connection;

use crate::memory::Memory;

use super::{
    consolidation::{classify_preference_texts, PreferenceConsolidationKind},
    query_global_preferences, query_project_preferences,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PreferenceRenderSummary {
    pub rendered: usize,
    pub project_rendered: usize,
    pub global_rendered: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PreferenceRenderDetails {
    pub summary: PreferenceRenderSummary,
    pub rendered_ids: Vec<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreferenceSource {
    Project,
    Global,
}

pub fn dedup_with_claude_md(prefs: &[Memory], cwd: &str) -> Vec<usize> {
    let claude_md_path = std::path::Path::new(cwd).join("CLAUDE.md");
    let claude_md_content = std::fs::read_to_string(&claude_md_path).unwrap_or_default();

    if claude_md_content.is_empty() {
        return (0..prefs.len()).collect();
    }

    let claude_lower = claude_md_content.to_lowercase();
    (0..prefs.len())
        .filter(|&i| {
            let title_lower = prefs[i].title.to_lowercase();
            let search_term = title_lower
                .strip_prefix("preference: ")
                .unwrap_or(&title_lower);
            !claude_lower.contains(search_term)
        })
        .collect()
}

pub fn render_preferences(
    output: &mut String,
    conn: &Connection,
    project: &str,
    cwd: &str,
) -> Result<()> {
    render_preferences_with_limits(output, conn, project, cwd, 20, 0, 1500).map(|_| ())
}

pub fn render_preferences_with_limits(
    output: &mut String,
    conn: &Connection,
    project: &str,
    cwd: &str,
    project_limit: usize,
    global_limit: usize,
    char_limit: usize,
) -> Result<usize> {
    render_preferences_with_context_details(
        output,
        conn,
        project,
        cwd,
        project_limit,
        global_limit,
        char_limit,
    )
    .map(|details| details.summary.rendered)
}

pub fn render_preferences_with_limits_detailed(
    output: &mut String,
    conn: &Connection,
    project: &str,
    cwd: &str,
    project_limit: usize,
    global_limit: usize,
    char_limit: usize,
) -> Result<PreferenceRenderSummary> {
    render_preferences_with_context_details(
        output,
        conn,
        project,
        cwd,
        project_limit,
        global_limit,
        char_limit,
    )
    .map(|details| details.summary)
}

pub(crate) fn render_preferences_with_context_details(
    output: &mut String,
    conn: &Connection,
    project: &str,
    cwd: &str,
    project_limit: usize,
    global_limit: usize,
    char_limit: usize,
) -> Result<PreferenceRenderDetails> {
    let project_prefs = query_project_preferences(conn, project, project_limit)?;
    let global_prefs = query_global_preferences(conn, global_limit)?;

    let mut all_prefs: Vec<(Memory, PreferenceSource)> = project_prefs
        .into_iter()
        .map(|memory| (memory, PreferenceSource::Project))
        .collect();
    let project_topics: std::collections::HashSet<String> = all_prefs
        .iter()
        .filter_map(|(memory, _)| memory.topic_key.clone())
        .collect();
    for global_pref in global_prefs {
        if let Some(ref topic_key) = global_pref.topic_key {
            if !project_topics.contains(topic_key) {
                all_prefs.push((global_pref, PreferenceSource::Global));
            }
        }
    }

    if all_prefs.is_empty() {
        return Ok(PreferenceRenderDetails::default());
    }

    let memories = all_prefs
        .iter()
        .map(|(memory, _)| memory.clone())
        .collect::<Vec<_>>();
    let keep_indices = dedup_with_claude_md(&memories, cwd);
    if keep_indices.is_empty() {
        return Ok(PreferenceRenderDetails::default());
    }
    let keep_indices = dedup_with_preference_similarity(&memories, &keep_indices);
    if keep_indices.is_empty() {
        return Ok(PreferenceRenderDetails::default());
    }

    output.push_str("## Your Preferences (always apply these)\n");
    let mut total_chars = 0usize;
    let mut summary = PreferenceRenderSummary::default();
    let mut rendered_ids = Vec::new();
    for &idx in &keep_indices {
        let (pref, source) = &all_prefs[idx];
        let text = normalize_rendered_preference_text(&pref.text);
        let preview: String = text.chars().take(120).collect();
        let line = if preview.chars().count() < text.chars().count() {
            format!("- {}...\n", preview)
        } else {
            format!("- {text}\n")
        };
        let line_chars = line.chars().count();
        if total_chars + line_chars > char_limit && total_chars > 0 {
            break;
        }
        output.push_str(&line);
        total_chars += line_chars;
        summary.rendered += 1;
        rendered_ids.push(pref.id);
        match source {
            PreferenceSource::Project => summary.project_rendered += 1,
            PreferenceSource::Global => summary.global_rendered += 1,
        }
    }
    output.push('\n');

    Ok(PreferenceRenderDetails {
        summary,
        rendered_ids,
    })
}

fn normalize_rendered_preference_text(text: &str) -> String {
    text.trim()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn dedup_with_preference_similarity(prefs: &[Memory], indices: &[usize]) -> Vec<usize> {
    let mut kept: Vec<usize> = Vec::new();
    for &idx in indices {
        let incoming = &prefs[idx];
        let already_represented = kept.iter().any(|&kept_idx| {
            let existing = &prefs[kept_idx];
            classify_preference_texts(existing.id, &existing.text, &incoming.text).is_some_and(
                |matched| {
                    matches!(
                        matched.kind,
                        PreferenceConsolidationKind::SamePreference
                            | PreferenceConsolidationKind::Refinement
                    )
                },
            )
        });
        if !already_represented {
            kept.push(idx);
        }
    }
    kept
}
