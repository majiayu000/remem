use anyhow::{bail, Result};

use super::types::{
    NormalizedRequest, UserRecallRequest, DEFAULT_BUDGET_CHARS, DEFAULT_LIMIT, MAX_BUDGET_CHARS,
    MAX_LIMIT, MIN_BUDGET_CHARS,
};
use crate::user_context::claims::{DEFAULT_OWNER_KEY, DEFAULT_OWNER_SCOPE};

pub(super) fn normalize_request(req: &UserRecallRequest) -> Result<NormalizedRequest> {
    let query = req.query.trim();
    if query.is_empty() {
        bail!("recall query is required");
    }
    let project = req.project.trim();
    if project.is_empty() {
        bail!("recall project is required");
    }

    let owner_scope = req
        .owner_scope
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_OWNER_SCOPE);
    if !matches!(owner_scope, "user" | "workspace" | "repo" | "session") {
        bail!("unsupported user-context owner scope: {owner_scope}");
    }
    let owner_key = req
        .owner_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let owner_key = match (owner_scope, owner_key) {
        ("user", None) => DEFAULT_OWNER_KEY,
        (_, Some(owner_key)) => owner_key,
        _ => bail!("owner_key is required when owner_scope is not user"),
    };

    let task_intent = optional_trimmed(req.task_intent.as_deref());
    let host = optional_trimmed(req.host.as_deref());
    let current_files = req
        .current_files
        .iter()
        .filter_map(|value| optional_trimmed(Some(value)))
        .collect::<Vec<_>>();
    let state_keys = req
        .state_keys
        .iter()
        .filter_map(|value| optional_trimmed(Some(value)))
        .collect::<Vec<_>>();
    let limit = req
        .limit
        .unwrap_or(DEFAULT_LIMIT as i64)
        .clamp(1, MAX_LIMIT as i64) as usize;
    let budget_chars = req
        .budget_chars
        .unwrap_or(DEFAULT_BUDGET_CHARS)
        .clamp(MIN_BUDGET_CHARS, MAX_BUDGET_CHARS);
    let terms = recall_terms(
        query,
        task_intent.as_deref(),
        &current_files,
        host.as_deref(),
    );

    Ok(NormalizedRequest {
        query: query.to_string(),
        project: project.to_string(),
        task_intent,
        host,
        owner_scope: owner_scope.to_string(),
        owner_key: owner_key.to_string(),
        state_keys,
        include_sensitive: req.include_sensitive,
        include_suppressed: req.include_suppressed,
        limit,
        budget_chars,
        terms,
    })
}

pub(super) fn relevant_to_request(text: &str, req: &NormalizedRequest) -> bool {
    if req.terms.is_empty() {
        return false;
    }
    let haystack = text.to_ascii_lowercase();
    req.terms.iter().any(|term| haystack.contains(term))
}

pub(super) fn search_query(req: &NormalizedRequest) -> String {
    [
        Some(req.query.as_str()),
        req.task_intent.as_deref(),
        req.host.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ")
}

pub(super) fn compact_line(text: &str, max_chars: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= max_chars {
        return text;
    }
    let mut out = text
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    out.push_str("...");
    out
}

fn recall_terms(
    query: &str,
    task_intent: Option<&str>,
    current_files: &[String],
    host: Option<&str>,
) -> Vec<String> {
    let mut terms = Vec::new();
    push_terms(&mut terms, query);
    if let Some(task_intent) = task_intent {
        push_terms(&mut terms, task_intent);
    }
    if let Some(host) = host {
        push_terms(&mut terms, host);
    }
    for file in current_files {
        push_terms(&mut terms, file);
        if let Some(name) = file.rsplit('/').next() {
            push_terms(&mut terms, name);
        }
    }
    terms.sort();
    terms.dedup();
    terms
}

fn push_terms(out: &mut Vec<String>, text: &str) {
    for term in text
        .to_ascii_lowercase()
        .split(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '-' || ch == '/'))
        .map(str::trim)
        .filter(|term| term.chars().count() >= 2)
    {
        out.push(term.to_string());
    }
}

fn optional_trimmed(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}
