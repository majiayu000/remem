use anyhow::{bail, Result};
use toml_edit::{DocumentMut, Item};

use crate::context::ContextLimits;

const CONTEXT_FIELDS: &[(&str, fn(&ContextLimits) -> usize)] = &[
    ("total_char_limit", |limits| limits.total_char_limit),
    ("candidate_fetch_limit", |limits| {
        limits.candidate_fetch_limit
    }),
    ("memory_index_limit", |limits| limits.memory_index_limit),
    ("memory_index_char_limit", |limits| {
        limits.memory_index_char_limit
    }),
    ("core_item_limit", |limits| limits.core_item_limit),
    ("core_char_limit", |limits| limits.core_char_limit),
    ("session_count", |limits| limits.session_limit),
    ("self_diagnostic_limit", |limits| {
        limits.self_diagnostic_limit
    }),
    ("preference_project_limit", |limits| {
        limits.preference_project_limit
    }),
    ("preference_global_limit", |limits| {
        limits.preference_global_limit
    }),
    ("preference_char_limit", |limits| {
        limits.preference_char_limit
    }),
    ("lesson_limit", |limits| limits.lesson_limit),
    ("lesson_char_limit", |limits| limits.lesson_char_limit),
    ("relevance_k", |limits| limits.sessionstart_relevance_k),
];

pub(crate) fn context_budget_limits() -> Result<ContextLimits> {
    let doc = super::read_config_doc_or_default()?;
    context_budget_limits_from_doc(&doc)
}

pub(super) fn ensure_defaults(doc: &mut DocumentMut) -> Result<()> {
    let context = super::top_table_mut(doc, "context")?;
    let defaults = ContextLimits::default();
    for (key, getter) in CONTEXT_FIELDS {
        super::set_i64_if_missing(context, key, getter(&defaults) as i64);
    }
    Ok(())
}

fn context_budget_limits_from_doc(doc: &DocumentMut) -> Result<ContextLimits> {
    let defaults = ContextLimits::default();
    let Some(table) = doc.get("context").and_then(Item::as_table) else {
        return Ok(defaults);
    };
    Ok(ContextLimits {
        total_char_limit: read_usize_field(table, "total_char_limit", defaults.total_char_limit)?,
        candidate_fetch_limit: read_usize_field(
            table,
            "candidate_fetch_limit",
            defaults.candidate_fetch_limit,
        )?,
        memory_index_limit: read_usize_field(
            table,
            "memory_index_limit",
            defaults.memory_index_limit,
        )?,
        memory_index_char_limit: read_usize_field(
            table,
            "memory_index_char_limit",
            defaults.memory_index_char_limit,
        )?,
        core_item_limit: read_usize_field(table, "core_item_limit", defaults.core_item_limit)?,
        core_char_limit: read_usize_field(table, "core_char_limit", defaults.core_char_limit)?,
        session_limit: read_usize_field(table, "session_count", defaults.session_limit)?,
        self_diagnostic_limit: read_usize_field(
            table,
            "self_diagnostic_limit",
            defaults.self_diagnostic_limit,
        )?,
        preference_project_limit: read_usize_field(
            table,
            "preference_project_limit",
            defaults.preference_project_limit,
        )?,
        preference_global_limit: read_usize_field(
            table,
            "preference_global_limit",
            defaults.preference_global_limit,
        )?,
        preference_char_limit: read_usize_field(
            table,
            "preference_char_limit",
            defaults.preference_char_limit,
        )?,
        lesson_limit: read_usize_field(table, "lesson_limit", defaults.lesson_limit)?,
        lesson_char_limit: read_usize_field(
            table,
            "lesson_char_limit",
            defaults.lesson_char_limit,
        )?,
        sessionstart_relevance_k: read_usize_field(
            table,
            "relevance_k",
            defaults.sessionstart_relevance_k,
        )?,
    })
}

fn read_usize_field(table: &toml_edit::Table, key: &str, default: usize) -> Result<usize> {
    let Some(item) = table.get(key) else {
        return Ok(default);
    };
    let Some(value) = item.as_integer() else {
        bail!("context.{key} must be an integer");
    };
    if value < 0 {
        bail!("context.{key} must be >= 0, got {value}");
    }
    Ok(value as usize)
}

#[cfg(test)]
mod tests;
