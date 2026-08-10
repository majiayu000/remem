use anyhow::{bail, Context, Result};

use super::BatchFilter;

const SECS_PER_DAY: i64 = 86_400;

pub(super) fn anonymous_placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count)
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn validate_batch_filter(filter: &BatchFilter) -> Result<()> {
    if filter.limit <= 0 {
        bail!("limit must be positive");
    }
    if let Some(contains) = &filter.contains {
        if contains.trim().is_empty() {
            bail!("contains filter must not be empty");
        }
    }
    if let Some(min_confidence) = filter.min_confidence {
        if !(0.0..=1.0).contains(&min_confidence) {
            bail!("min_confidence must be between 0 and 1");
        }
    }
    if let Some(older_than_days) = filter.older_than_days {
        if older_than_days < 0 {
            bail!("older_than_days must be non-negative");
        }
        older_than_cutoff(chrono::Utc::now().timestamp(), older_than_days)?;
    }
    Ok(())
}

pub(super) fn older_than_cutoff(now_epoch: i64, older_than_days: i64) -> Result<i64> {
    let age_secs = older_than_days
        .checked_mul(SECS_PER_DAY)
        .context("older_than_days is too large")?;
    now_epoch
        .checked_sub(age_secs)
        .context("older_than_days is too large")
}

pub(super) fn like_pattern(query: &str) -> String {
    let mut pattern = String::with_capacity(query.len() + 2);
    pattern.push('%');
    for ch in query.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            pattern.push('\\');
        }
        pattern.push(ch);
    }
    pattern.push('%');
    pattern
}
