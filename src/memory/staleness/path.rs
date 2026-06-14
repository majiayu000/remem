use std::collections::HashSet;

use anyhow::{Context, Result};

pub(super) fn parse_file_list(raw: Option<&str>, project: &str) -> Result<HashSet<String>> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(HashSet::new());
    };
    let files = if raw.starts_with('[') {
        parse_json_file_array(raw)
            .with_context(|| "parse memory files for source-anchor staleness")?
    } else {
        raw.split([',', '\n'])
            .map(str::trim)
            .map(|value| value.trim_matches('"'))
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect()
    };
    Ok(files
        .into_iter()
        .filter_map(|file| normalize_file_path_for_project(&file, project))
        .collect())
}

pub(super) fn parse_json_file_array(raw: &str) -> Result<Vec<String>> {
    Ok(serde_json::from_str::<Vec<String>>(raw)?)
}

pub(super) fn file_path_overlaps(changed_file: &str, memory_file: &str, project: &str) -> bool {
    let Some(changed_file) = normalize_file_path_for_project(changed_file, project) else {
        return false;
    };
    changed_file == memory_file
        || changed_file
            .strip_prefix(memory_file)
            .is_some_and(|tail| tail.starts_with('/'))
        || memory_file
            .strip_prefix(&changed_file)
            .is_some_and(|tail| tail.starts_with('/'))
}

fn normalize_file_path(path: &str) -> Option<String> {
    let trimmed = path.trim().trim_start_matches("./").trim_matches('/');
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn normalize_file_path_for_project(path: &str, project: &str) -> Option<String> {
    let normalized = normalize_file_path(path)?;
    let Some(project) = normalize_file_path(project) else {
        return Some(normalized);
    };
    if normalized == project {
        return None;
    }
    normalized
        .strip_prefix(&format!("{project}/"))
        .map(str::to_string)
        .filter(|value| !value.is_empty())
        .or(Some(normalized))
}
