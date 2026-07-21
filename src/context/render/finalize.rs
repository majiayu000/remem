use std::collections::{HashMap, HashSet};

use super::super::audit::workstream_stable_key;
use super::super::format::char_len;
use super::super::relevance::{memory_stable_key, session_stable_key};
use super::helpers::{
    build_context_stats_footer_with_style, enforce_total_char_limit_preserving_footer,
};
use super::{ContextRenderStats, Result};

pub(super) struct RenderedIdentityBounds<'a> {
    pub core_ids: &'a [i64],
    pub core_ends: &'a [usize],
    pub lesson_ids: &'a [i64],
    pub lesson_ends: &'a [usize],
    pub index_ids: &'a [i64],
    pub index_ends: &'a [usize],
    pub session_ids: &'a [i64],
    pub session_ends: &'a [usize],
    pub workstream_ids: &'a [i64],
    pub workstream_ends: &'a [usize],
}

pub(super) struct FinalizedContextOutput {
    pub output: String,
    pub final_core_ids: Vec<i64>,
    pub final_lesson_ids: Vec<i64>,
    pub final_index_ids: Vec<i64>,
    pub final_session_ids: Vec<i64>,
    pub final_workstream_ids: Vec<i64>,
    pub total_truncated_keys: HashSet<String>,
    pub item_end_chars: HashMap<String, usize>,
}

pub(super) fn finalize_context_output(
    untruncated_body: String,
    stats: &mut ContextRenderStats,
    use_colors: bool,
    bounds: &RenderedIdentityBounds<'_>,
) -> Result<FinalizedContextOutput> {
    validate_bounds(bounds)?;
    let pre_total_governed_count =
        bounds.lesson_ids.len() + bounds.index_ids.len() + bounds.session_ids.len();
    let mut final_core_ids = bounds.core_ids.to_vec();
    let mut final_lesson_ids = bounds.lesson_ids.to_vec();
    let mut final_index_ids = bounds.index_ids.to_vec();
    let mut final_session_ids = bounds.session_ids.to_vec();
    let mut final_workstream_ids = bounds.workstream_ids.to_vec();
    let mut output = String::new();

    for _ in 0..4 {
        let mut footer = build_context_stats_footer_with_style(stats, use_colors);
        stats.output_chars = char_len(&untruncated_body) + char_len(&footer);
        stats.truncated = stats.total_char_limit > 0 && stats.output_chars > stats.total_char_limit;
        footer = build_context_stats_footer_with_style(stats, use_colors);
        stats.output_chars = char_len(&untruncated_body) + char_len(&footer);

        let mut candidate_output = untruncated_body.clone();
        candidate_output.push_str(&footer);
        let retained_body_chars = enforce_total_char_limit_preserving_footer(
            &mut candidate_output,
            stats.total_char_limit,
            &footer,
        );
        final_core_ids = surviving_ids(bounds.core_ids, bounds.core_ends, retained_body_chars);
        final_lesson_ids =
            surviving_ids(bounds.lesson_ids, bounds.lesson_ends, retained_body_chars);
        final_index_ids = surviving_ids(bounds.index_ids, bounds.index_ends, retained_body_chars);
        final_session_ids =
            surviving_ids(bounds.session_ids, bounds.session_ends, retained_body_chars);
        final_workstream_ids = surviving_ids(
            bounds.workstream_ids,
            bounds.workstream_ends,
            retained_body_chars,
        );
        let next_final = final_lesson_ids.len() + final_index_ids.len() + final_session_ids.len();
        let next_total_limited = pre_total_governed_count.saturating_sub(next_final);
        let stable = next_final == stats.relevance.final_injected
            && next_total_limited == stats.relevance.total_limited;
        stats.relevance.final_injected = next_final;
        stats.relevance.total_limited = next_total_limited;
        output = candidate_output;
        if stable {
            break;
        }
    }

    let total_truncated_keys = truncated_memory_keys(
        bounds.core_ids,
        &final_core_ids,
        bounds.lesson_ids,
        &final_lesson_ids,
        bounds.index_ids,
        &final_index_ids,
    )
    .chain(
        bounds
            .session_ids
            .iter()
            .filter(|id| !final_session_ids.contains(id))
            .map(|id| session_stable_key(*id)),
    )
    .chain(
        bounds
            .workstream_ids
            .iter()
            .filter(|id| !final_workstream_ids.contains(id))
            .map(|id| workstream_stable_key(*id)),
    )
    .collect();
    let mut item_end_chars = HashMap::new();
    record_item_ends(
        &mut item_end_chars,
        bounds.core_ids,
        bounds.core_ends,
        memory_stable_key,
    );
    record_item_ends(
        &mut item_end_chars,
        bounds.lesson_ids,
        bounds.lesson_ends,
        memory_stable_key,
    );
    record_item_ends(
        &mut item_end_chars,
        bounds.index_ids,
        bounds.index_ends,
        memory_stable_key,
    );
    record_item_ends(
        &mut item_end_chars,
        bounds.session_ids,
        bounds.session_ends,
        session_stable_key,
    );
    record_item_ends(
        &mut item_end_chars,
        bounds.workstream_ids,
        bounds.workstream_ends,
        workstream_stable_key,
    );

    Ok(FinalizedContextOutput {
        output,
        final_core_ids,
        final_lesson_ids,
        final_index_ids,
        final_session_ids,
        final_workstream_ids,
        total_truncated_keys,
        item_end_chars,
    })
}

fn validate_bounds(bounds: &RenderedIdentityBounds<'_>) -> Result<()> {
    for (channel, ids, ends) in [
        ("core", bounds.core_ids, bounds.core_ends),
        ("lessons", bounds.lesson_ids, bounds.lesson_ends),
        ("index", bounds.index_ids, bounds.index_ends),
        ("sessions", bounds.session_ids, bounds.session_ends),
        ("workstreams", bounds.workstream_ids, bounds.workstream_ends),
    ] {
        if ids.len() != ends.len() {
            anyhow::bail!(
                "{channel} renderer returned {} identities but {} item boundaries",
                ids.len(),
                ends.len()
            );
        }
    }
    Ok(())
}

fn surviving_ids(ids: &[i64], ends: &[usize], retained_body_chars: usize) -> Vec<i64> {
    ids.iter()
        .zip(ends)
        .filter_map(|(id, end)| (*end <= retained_body_chars).then_some(*id))
        .collect()
}

fn truncated_memory_keys<'a>(
    core_ids: &'a [i64],
    final_core_ids: &'a [i64],
    lesson_ids: &'a [i64],
    final_lesson_ids: &'a [i64],
    index_ids: &'a [i64],
    final_index_ids: &'a [i64],
) -> impl Iterator<Item = String> + 'a {
    core_ids
        .iter()
        .filter(|id| !final_core_ids.contains(id))
        .chain(
            lesson_ids
                .iter()
                .filter(|id| !final_lesson_ids.contains(id)),
        )
        .chain(index_ids.iter().filter(|id| !final_index_ids.contains(id)))
        .map(|id| memory_stable_key(*id))
}

fn record_item_ends(
    target: &mut HashMap<String, usize>,
    ids: &[i64],
    ends: &[usize],
    key: impl Fn(i64) -> String,
) {
    target.extend(ids.iter().zip(ends).map(|(id, end)| (key(*id), *end)));
}
