use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use super::cursor::{
    continuation_id, decode_cursor, encode_cursor, filter_fingerprint, CursorKind,
};
use super::helpers::error_response;

const DEFAULT_PAGE_SIZE: i64 = 50;
const MAX_PAGE_SIZE: i64 = 100;
const MAX_RAW_SCAN_BUDGET: usize = 1000;
const RAW_BATCH_SIZE: usize = 100;
const MAX_VISIBLE_CHARS: usize = 240;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ReadResourceParams {
    pub project: Option<String>,
    pub status: Option<String>,
    pub page_size: Option<String>,
    pub cursor: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct SafeResourceRef {
    pub kind: &'static str,
    pub id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Serialize)]
struct ReadResourceListResponse<T: Serialize> {
    data: Vec<T>,
    next_cursor: Option<String>,
    page_size: i64,
}

#[derive(Debug, Serialize)]
struct ReadResourceDetailResponse<T: Serialize> {
    data: T,
}

#[derive(Debug, Clone)]
#[cfg_attr(not(test), allow(dead_code))]
pub(super) enum PolicyRelation<'a> {
    Memory(i64),
    Topic(&'a str),
    Entity(&'a str),
}

#[derive(Debug, Default)]
pub(super) struct ResourceProjectionPolicy {
    patterns: Vec<String>,
    memory_ids: std::collections::HashSet<i64>,
    topic_keys: std::collections::HashSet<String>,
    entities: std::collections::HashSet<String>,
}

impl ResourceProjectionPolicy {
    fn load(conn: &Connection) -> anyhow::Result<Self> {
        let mut stmt = conn.prepare(
            "SELECT target_kind, target_id, target_value
             FROM memory_suppressions WHERE status = 'active' ORDER BY id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;
        let mut policy = Self::default();
        for row in rows {
            let (kind, target_id, target_value) = row?;
            match kind.as_str() {
                "pattern" => policy.patterns.push(required_value(target_value)?),
                "memory" => {
                    let id = target_id.filter(|id| *id > 0).ok_or_else(|| {
                        anyhow::anyhow!("active memory suppression has no positive id")
                    })?;
                    policy.memory_ids.insert(id);
                }
                "topic_key" => {
                    policy
                        .topic_keys
                        .insert(required_value(target_value)?.to_lowercase());
                }
                "entity" => {
                    policy
                        .entities
                        .insert(required_value(target_value)?.to_lowercase());
                }
                "user_claim" | "user_candidate" | "summary" => {}
                _ => anyhow::bail!("unsupported active suppression target kind"),
            }
        }
        Ok(policy)
    }

    pub(super) fn suppresses(
        &self,
        visible_text: &[&str],
        relations: &[PolicyRelation<'_>],
    ) -> bool {
        let pattern_match = self.patterns.iter().any(|pattern| {
            let pattern = pattern.to_lowercase();
            visible_text
                .iter()
                .any(|text| text.to_lowercase().contains(&pattern))
        });
        pattern_match
            || relations.iter().any(|relation| match relation {
                PolicyRelation::Memory(id) => self.memory_ids.contains(id),
                PolicyRelation::Topic(value) => self.topic_keys.contains(&value.to_lowercase()),
                PolicyRelation::Entity(value) => self.entities.contains(&value.to_lowercase()),
            })
    }
}

fn required_value(value: Option<String>) -> anyhow::Result<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("active suppression has no target value"))
}

pub(super) trait ReadResourceSpec {
    type Row;
    type Item: Serialize;

    const KIND: CursorKind;
    fn row_id(row: &Self::Row) -> i64;

    fn load_batch(
        conn: &Connection,
        resume_before_id: Option<i64>,
        project: Option<&str>,
        status: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<Self::Row>>;

    fn load_one(conn: &Connection, id: i64) -> anyhow::Result<Option<Self::Row>>;

    fn project(
        row: Self::Row,
        policy: &ResourceProjectionPolicy,
    ) -> anyhow::Result<Option<Self::Item>>;
}

#[derive(Debug, Serialize)]
struct CursorFilters<'a> {
    page_size: i64,
    project: Option<&'a str>,
    status: Option<&'a str>,
}

pub(super) fn list_resource<R: ReadResourceSpec>(params: ReadResourceParams) -> Response {
    let page_size = match parse_page_size(params.page_size.as_deref()) {
        Ok(page_size) => page_size,
        Err(()) => return stable_error(StatusCode::BAD_REQUEST, "page_size_invalid"),
    };
    let project = normalize_filter(params.project);
    let status = normalize_filter(params.status);
    let filters = CursorFilters {
        page_size,
        project: project.as_deref(),
        status: status.as_deref(),
    };
    let fingerprint = match filter_fingerprint(&filters) {
        Ok(fingerprint) => fingerprint,
        Err(_) => return stable_error(StatusCode::INTERNAL_SERVER_ERROR, "cursor_encode_failed"),
    };
    let mut resume_before_id = match params.cursor.as_deref() {
        Some(cursor) => match decode_cursor(cursor, R::KIND, &fingerprint) {
            Ok(cursor) => Some(cursor.resume_before_id),
            Err(_) => return stable_error(StatusCode::BAD_REQUEST, "cursor_invalid"),
        },
        None => None,
    };
    let conn = match crate::db::open_db() {
        Ok(conn) => conn,
        Err(_) => return stable_error(StatusCode::INTERNAL_SERVER_ERROR, "db_open_failed"),
    };
    let policy = match ResourceProjectionPolicy::load(&conn) {
        Ok(policy) => policy,
        Err(_) => return stable_error(StatusCode::INTERNAL_SERVER_ERROR, "resource_policy_failed"),
    };
    let raw_scan_budget = ((page_size as usize) * 10).clamp(RAW_BATCH_SIZE, MAX_RAW_SCAN_BUDGET);
    let mut scanned = 0usize;
    let mut items = Vec::with_capacity(page_size as usize);
    let mut last_scanned_raw_id = None;
    let mut last_returned_safe_id = None;
    let mut eligible_rows_exhausted = false;

    while items.len() < page_size as usize && scanned < raw_scan_budget {
        let batch_limit = RAW_BATCH_SIZE.min(raw_scan_budget - scanned);
        let rows = match R::load_batch(
            &conn,
            resume_before_id,
            project.as_deref(),
            status.as_deref(),
            batch_limit,
        ) {
            Ok(rows) => rows,
            Err(_) => {
                return stable_error(StatusCode::INTERNAL_SERVER_ERROR, "resource_query_failed")
            }
        };
        if rows.is_empty() {
            eligible_rows_exhausted = true;
            break;
        }
        let batch_was_short = rows.len() < batch_limit;
        for row in rows {
            let id = R::row_id(&row);
            scanned += 1;
            last_scanned_raw_id = Some(id);
            resume_before_id = Some(id);
            match R::project(row, &policy) {
                Ok(Some(item)) => {
                    items.push(item);
                    last_returned_safe_id = Some(id);
                    if items.len() == page_size as usize {
                        break;
                    }
                }
                Ok(None) => {}
                Err(_) => {
                    return stable_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "resource_projection_failed",
                    )
                }
            }
        }
        if items.len() == page_size as usize {
            break;
        }
        if batch_was_short {
            eligible_rows_exhausted = true;
            break;
        }
    }

    let page_is_full = items.len() == page_size as usize;
    let scan_budget_exhausted = scanned == raw_scan_budget && !page_is_full;
    let continuation = match continuation_id(
        page_is_full,
        last_returned_safe_id,
        scan_budget_exhausted,
        last_scanned_raw_id,
        eligible_rows_exhausted,
    ) {
        Ok(continuation) => continuation,
        Err(_) => return stable_error(StatusCode::INTERNAL_SERVER_ERROR, "cursor_encode_failed"),
    };
    let next_cursor = match continuation {
        Some(id) => match encode_cursor(R::KIND, &fingerprint, id) {
            Ok(cursor) => Some(cursor),
            Err(_) => {
                return stable_error(StatusCode::INTERNAL_SERVER_ERROR, "cursor_encode_failed")
            }
        },
        None => None,
    };
    Json(ReadResourceListResponse {
        data: items,
        next_cursor,
        page_size,
    })
    .into_response()
}

pub(super) fn detail_resource<R: ReadResourceSpec>(raw_id: String) -> Response {
    let id = match raw_id.parse::<i64>() {
        Ok(id) if id > 0 => id,
        _ => return stable_error(StatusCode::BAD_REQUEST, "id_invalid"),
    };
    let conn = match crate::db::open_db() {
        Ok(conn) => conn,
        Err(_) => return stable_error(StatusCode::INTERNAL_SERVER_ERROR, "db_open_failed"),
    };
    let policy = match ResourceProjectionPolicy::load(&conn) {
        Ok(policy) => policy,
        Err(_) => return stable_error(StatusCode::INTERNAL_SERVER_ERROR, "resource_policy_failed"),
    };
    let row = match R::load_one(&conn, id) {
        Ok(Some(row)) => row,
        Ok(None) => return stable_error(StatusCode::NOT_FOUND, "not_found"),
        Err(_) => return stable_error(StatusCode::INTERNAL_SERVER_ERROR, "resource_query_failed"),
    };
    match R::project(row, &policy) {
        Ok(Some(item)) => Json(ReadResourceDetailResponse { data: item }).into_response(),
        Ok(None) => stable_error(StatusCode::NOT_FOUND, "not_found"),
        Err(_) => stable_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "resource_projection_failed",
        ),
    }
}

pub(super) fn redact_bounded(value: &str) -> String {
    crate::adapter::common::redact_sensitive_text(value)
        .chars()
        .take(MAX_VISIBLE_CHARS)
        .collect()
}

pub(super) fn redact_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| redact_bounded(&value))
        .filter(|value| !value.is_empty())
}

fn normalize_filter(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_page_size(raw: Option<&str>) -> Result<i64, ()> {
    let Some(raw) = raw else {
        return Ok(DEFAULT_PAGE_SIZE);
    };
    raw.parse::<i64>()
        .map(|value| value.clamp(1, MAX_PAGE_SIZE))
        .map_err(|_| ())
}

fn stable_error(status: StatusCode, code: &str) -> Response {
    error_response(status, code, code).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_size_defaults_clamps_and_rejects_malformed_values() {
        assert_eq!(parse_page_size(None), Ok(50));
        assert_eq!(parse_page_size(Some("0")), Ok(1));
        assert_eq!(parse_page_size(Some("-9")), Ok(1));
        assert_eq!(parse_page_size(Some("101")), Ok(100));
        assert_eq!(parse_page_size(Some("9223372036854775807")), Ok(100));
        assert_eq!(parse_page_size(Some("nope")), Err(()));
        assert_eq!(parse_page_size(Some("9223372036854775808")), Err(()));
    }

    #[test]
    fn pattern_and_relation_policy_is_fail_closed_before_redaction() {
        let policy = ResourceProjectionPolicy {
            patterns: vec!["secret-pattern".to_string()],
            memory_ids: [7].into_iter().collect(),
            topic_keys: ["hidden-topic".to_string()].into_iter().collect(),
            entities: ["hidden-entity".to_string()].into_iter().collect(),
        };
        assert!(policy.suppresses(&["before secret-pattern after"], &[]));
        assert!(policy.suppresses(&[], &[PolicyRelation::Memory(7)]));
        assert!(policy.suppresses(&[], &[PolicyRelation::Topic("HIDDEN-TOPIC")]));
        assert!(policy.suppresses(&[], &[PolicyRelation::Entity("Hidden-Entity")]));
        assert!(!policy.suppresses(&["visible"], &[]));
    }
}
