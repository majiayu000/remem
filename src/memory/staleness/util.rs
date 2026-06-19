use anyhow::{Context, Result};

pub(super) fn parse_evidence_event_ids(raw: Option<&str>, context: &str) -> Result<Vec<i64>> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(Vec::new());
    };
    let mut ids = serde_json::from_str::<Vec<i64>>(raw)
        .with_context(|| format!("parse {context} for source-anchor staleness"))?;
    ids.retain(|id| *id > 0);
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

pub(super) fn placeholders(start: usize, count: usize) -> String {
    (start..start + count)
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn non_empty_trimmed(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

pub(super) fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

pub(super) fn push_unique_i64(values: &mut Vec<i64>, value: i64) {
    if !values.contains(&value) {
        values.push(value);
    }
}
