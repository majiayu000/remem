//! Transport-neutral raw query bounds and JSON response contracts (GH720).

use anyhow::Result;
use serde::Serialize;

use super::raw_archive::RawMessage;

const RAW_ARCHIVE_NOTE: &str = "raw archive rows are captured chat turns, not curated memories";

pub(crate) fn parse_time_lower_bound(value: &str) -> Result<i64> {
    parse_time_bound(value, DateBound::Lower)
}

pub(crate) fn parse_time_upper_bound(value: &str) -> Result<i64> {
    parse_time_bound(value, DateBound::Upper)
}

#[derive(Clone, Copy)]
enum DateBound {
    Lower,
    Upper,
}

fn parse_time_bound(value: &str, date_bound: DateBound) -> Result<i64> {
    let trimmed = value.trim();
    if let Ok(epoch) = trimmed.parse::<i64>() {
        return Ok(epoch);
    }
    if let Ok(datetime) = chrono::DateTime::parse_from_rfc3339(trimmed) {
        return Ok(datetime.timestamp());
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        let time = match date_bound {
            DateBound::Lower => (0, 0, 0),
            DateBound::Upper => (23, 59, 59),
        };
        let datetime = date
            .and_hms_opt(time.0, time.1, time.2)
            .ok_or_else(|| anyhow::anyhow!("invalid UTC day boundary for {trimmed:?}"))?;
        return Ok(datetime.and_utc().timestamp());
    }
    anyhow::bail!(
        "invalid time bound {trimmed:?}: expected Unix epoch, ISO8601 datetime, or YYYY-MM-DD"
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_raw_search_json(
    query: &str,
    project: Option<&str>,
    branch: Option<&str>,
    role: Option<&str>,
    limit: i64,
    offset: i64,
    since_epoch: Option<i64>,
    until_epoch: Option<i64>,
    has_more: bool,
    rows: &[RawMessage],
) -> RawSearchJson {
    let normalized_limit = limit.max(1);
    let normalized_offset = offset.max(0);
    RawSearchJson {
        query: query.to_string(),
        project: project.map(str::to_string),
        branch: branch.map(str::to_string),
        role: role.map(str::to_string),
        limit: normalized_limit,
        offset: normalized_offset,
        since_epoch,
        until_epoch,
        source_type: "raw_archive".to_string(),
        note: RAW_ARCHIVE_NOTE.to_string(),
        count: rows.len(),
        has_more,
        next_offset: has_more.then_some(normalized_offset.saturating_add(normalized_limit)),
        results: rows.iter().map(RawArchiveRowJson::from).collect(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RawSearchJson {
    pub query: String,
    pub project: Option<String>,
    pub branch: Option<String>,
    pub role: Option<String>,
    pub limit: i64,
    pub offset: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since_epoch: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until_epoch: Option<i64>,
    pub source_type: String,
    pub note: String,
    pub count: usize,
    pub has_more: bool,
    pub next_offset: Option<i64>,
    pub results: Vec<RawArchiveRowJson>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RawArchiveRowJson {
    pub id: i64,
    pub source_type: String,
    pub session_id: String,
    pub project: String,
    pub role: String,
    pub content: String,
    pub source: String,
    pub branch: Option<String>,
    pub cwd: Option<String>,
    pub created_at_epoch: i64,
}

impl From<&RawMessage> for RawArchiveRowJson {
    fn from(row: &RawMessage) -> Self {
        Self {
            id: row.id,
            source_type: "raw_archive".to_string(),
            session_id: row.session_id.clone(),
            project: row.project.clone(),
            role: row.role.clone(),
            content: row.content.clone(),
            source: row.source.clone(),
            branch: row.branch.clone(),
            cwd: row.cwd.clone(),
            created_at_epoch: row.created_at_epoch,
        }
    }
}
