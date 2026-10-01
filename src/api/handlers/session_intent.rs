//! Explicit, snapshot-bound overrides. Preview and apply share the existing audit ledger.
use axum::{
    body::Bytes,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::super::read_resources::redact_bounded;
use crate::adapter::common::redact_projected_sensitive_text;
use crate::adapter::redaction::redact_tokens;
use crate::db::summary_poisoning::LABEL_ROW_ELIGIBLE_SQL;
use crate::memory::session_label::{normalize_topic, SessionIntent, TOPIC_MAX_CHARS};

const PREVIEW_EVENT: &str = "session_intent_preview";
const APPLIED_EVENT: &str = "session_intent_override";
const TTL_SECONDS: i64 = 900;

#[derive(Debug)]
struct Failure(StatusCode, &'static str);
type Result<T> = std::result::Result<T, Failure>;
impl Failure {
    fn response(self) -> Response {
        let message = match self.1 {
            "session_summary_required" => {
                "This session needs a persisted summary before its label can be edited."
            }
            "preview_stale" => {
                "A selected label changed after preview. Preview again before applying."
            }
            "preview_expired" => "This preview expired. Preview again before applying.",
            "preview_already_applied" => "This preview was already applied. Refresh the labels.",
            "target_not_found" => "A selected session or canonical workstream is unavailable.",
            "session_topic_unsafe" => {
                "The topic contains unsafe instructions. Use a descriptive topic."
            }
            "session_intent_cross_line_sensitive_argument" => {
                "Put each sensitive option and its value on the same line before previewing."
            }
            "session_intent_invalid" => "Choose a supported intent code or explicitly clear it.",
            "session_topic_invalid" => "Use a topic of 1 to 80 characters or explicitly clear it.",
            "reason_invalid" => "Provide a reason of 1 to 1000 characters.",
            "confirmation_required" => "Confirm the reviewed changes before applying.",
            "session_intent_override_failed" => {
                "The override could not be completed. Check the remem API log."
            }
            _ => "The override request is invalid. Refresh and preview the changes again.",
        };
        (
            self.0,
            Json(json!({"error":{"code":self.1,"message":message}})),
        )
            .into_response()
    }
}
fn internal(error: impl std::fmt::Display) -> Failure {
    crate::log::error("api", &format!("session intent override failed: {error}"));
    Failure(
        StatusCode::INTERNAL_SERVER_ERROR,
        "session_intent_override_failed",
    )
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Session,
    Workstream,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
struct Target {
    kind: Kind,
    id: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewRequest {
    targets: Vec<Target>,
    #[serde(deserialize_with = "required_nullable")]
    session_intent: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    session_topic: Option<String>,
    reason: String,
}
fn required_nullable<'de, D>(deserializer: D) -> std::result::Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyRequest {
    preview_token: String,
    confirm: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct Fields {
    session_intent: Option<String>,
    session_topic: Option<String>,
    session_intent_source: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Change {
    #[serde(flatten)]
    target: Target,
    before: Fields,
    after: Fields,
}
#[derive(Deserialize, Serialize)]
struct Snapshot {
    changes: Vec<Change>,
    fingerprints: Vec<String>,
    reason: String,
    expires_at_epoch: i64,
}
#[derive(Serialize)]
struct Current {
    row_id: i64,
    fields: Fields,
    updated_at_epoch: Option<i64>,
    project: String,
    title: Option<String>,
}

pub(in crate::api) async fn handle_session_intent_preview(body: Bytes) -> Response {
    let result = (|| {
        let request = serde_json::from_slice(&body)
            .map_err(|_| Failure(StatusCode::BAD_REQUEST, "session_intent_request_invalid"))?;
        let mut conn = crate::db::open_db().map_err(internal)?;
        preview(&mut conn, request)
    })();
    result
        .map(|v| Json(v).into_response())
        .unwrap_or_else(Failure::response)
}
pub(in crate::api) async fn handle_session_intent_apply(body: Bytes) -> Response {
    let result = (|| {
        let request = serde_json::from_slice(&body)
            .map_err(|_| Failure(StatusCode::BAD_REQUEST, "session_intent_request_invalid"))?;
        let mut conn = crate::db::open_db().map_err(internal)?;
        apply(&mut conn, request)
    })();
    result
        .map(|v| Json(v).into_response())
        .unwrap_or_else(Failure::response)
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn fingerprint(current: &Current) -> Result<String> {
    Ok(digest(&serde_json::to_vec(current).map_err(internal)?))
}
fn load(conn: &Connection, target: &Target) -> Result<Current> {
    let visible = match target.kind {
        Kind::Session => super::sessions::session_is_visible(conn, target.id),
        Kind::Workstream => super::workstreams::workstream_is_visible(conn, target.id),
    }
    .map_err(internal)?;
    if !visible {
        return Err(Failure(StatusCode::NOT_FOUND, "target_not_found"));
    }
    let sql = match target.kind {
        Kind::Session => format!("SELECT id, session_intent, session_topic, session_intent_source,
            session_intent_updated_at_epoch, COALESCE(project,''), NULL FROM session_summaries
            WHERE session_row_id=?1 AND {LABEL_ROW_ELIGIBLE_SQL}
            ORDER BY COALESCE(session_intent_updated_at_epoch,created_at_epoch) DESC,id DESC LIMIT 1"),
        Kind::Workstream => "SELECT id, session_intent, session_topic, session_intent_source,
            session_intent_updated_at_epoch, project, title FROM workstreams
            WHERE id=?1 AND merged_into_workstream_id IS NULL".to_owned(),
    };
    conn.query_row(&sql, [target.id], |row| {
        Ok(Current {
            row_id: row.get(0)?,
            fields: Fields {
                session_intent: row.get(1)?,
                session_topic: row.get(2)?,
                session_intent_source: row.get(3)?,
            },
            updated_at_epoch: row.get(4)?,
            project: row.get(5)?,
            title: row.get(6)?,
        })
    })
    .optional()
    .map_err(internal)?
    .ok_or(Failure(StatusCode::CONFLICT, "session_summary_required"))
}
fn project_override_text(text: &str) -> String {
    let continued = text.replace("\\\r\n", "").replace("\\\n", "");
    continued
        .lines()
        .map(redact_projected_sensitive_text)
        .collect::<Vec<_>>()
        .join("\n")
}
fn redact_override_text(text: &str) -> Result<String> {
    let projected = project_override_text(text);
    // Compare only after projection so header redaction keeps unrelated rationale.
    // A changed token pass exposes cross-line option/value ambiguity, including
    // YAML list prefixes that the shell walker may consume as an argument.
    if redact_tokens(&projected, true, false) != projected {
        return Err(Failure(
            StatusCode::BAD_REQUEST,
            "session_intent_cross_line_sensitive_argument",
        ));
    }
    Ok(projected)
}
fn bounded_topic(redacted: &str) -> String {
    const MARKER: &str = "[REDACTED]";
    let mut topic = String::new();
    for part in redacted.split_inclusive(MARKER) {
        let remaining = TOPIC_MAX_CHARS - topic.chars().count();
        if part.chars().count() <= remaining {
            topic.push_str(part);
        } else {
            if let Some(benign) = part.strip_suffix(MARKER) {
                if remaining >= MARKER.len() {
                    topic.extend(benign.chars().take(remaining - MARKER.len()));
                    topic.push_str(MARKER);
                }
            } else {
                topic.extend(part.chars().take(remaining));
            }
            break;
        }
    }
    topic
}
fn preview(conn: &mut Connection, request: PreviewRequest) -> Result<serde_json::Value> {
    if request.targets.is_empty() || request.targets.len() > 50 {
        return Err(Failure(
            StatusCode::BAD_REQUEST,
            "targets_require_1_to_50_items",
        ));
    }
    let mut seen = std::collections::HashSet::new();
    if request.targets.iter().any(|t| t.id <= 0 || !seen.insert(t)) {
        return Err(Failure(
            StatusCode::BAD_REQUEST,
            "targets_invalid_or_duplicate",
        ));
    }
    let intent = request
        .session_intent
        .as_deref()
        .map(SessionIntent::parse_write)
        .transpose()
        .map_err(|_| Failure(StatusCode::BAD_REQUEST, "session_intent_invalid"))?;
    let topic = request
        .session_topic
        .as_deref()
        .map(|raw| {
            if crate::memory::poisoning::scan_instruction_pattern(raw).is_some() {
                return Err(Failure(StatusCode::BAD_REQUEST, "session_topic_unsafe"));
            }
            normalize_topic(raw)
                .ok_or(Failure(StatusCode::BAD_REQUEST, "session_topic_invalid"))?;
            let redacted = redact_override_text(raw)?;
            let topic = bounded_topic(redacted.trim());
            if crate::memory::poisoning::scan_instruction_pattern(&topic).is_some() {
                return Err(Failure(StatusCode::BAD_REQUEST, "session_topic_unsafe"));
            }
            normalize_topic(&topic).ok_or(Failure(StatusCode::BAD_REQUEST, "session_topic_invalid"))
        })
        .transpose()?;
    let reason = request.reason.trim();
    if reason.is_empty() || reason.chars().count() > 1000 {
        return Err(Failure(StatusCode::BAD_REQUEST, "reason_invalid"));
    }
    let reason = redact_override_text(reason)?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(internal)?;
    let mut snapshot = Snapshot {
        changes: vec![],
        fingerprints: vec![],
        reason,
        expires_at_epoch: chrono::Utc::now().timestamp() + TTL_SECONDS,
    };
    for target in request.targets {
        let current = load(&tx, &target)?;
        snapshot.fingerprints.push(fingerprint(&current)?);
        let mut before = current.fields;
        before.session_topic = before.session_topic.as_deref().map(|text| {
            let projected = project_override_text(text);
            // Stored text is display data. Hide ambiguous spans in full:
            // the token walker can consume a YAML dash before its value.
            if redact_tokens(&projected, true, false) != projected {
                "[REDACTED]".to_owned()
            } else {
                redact_bounded(&projected)
            }
        });
        snapshot.changes.push(Change {
            target,
            before,
            after: Fields {
                session_intent: intent.map(|value| value.as_str().to_owned()),
                session_topic: topic.clone(),
                session_intent_source: Some("override".to_owned()),
            },
        });
    }
    let mut random = [0_u8; 32];
    getrandom::fill(&mut random).map_err(internal)?;
    let token = random
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let token_hash = digest(token.as_bytes());
    tx.execute(
        "INSERT INTO events(session_id,project,event_type,summary,detail,created_at_epoch)
        VALUES (?1,'',?2,'Session intent override preview',?3,?4)",
        params![
            token_hash,
            PREVIEW_EVENT,
            serde_json::to_string(&snapshot).map_err(internal)?,
            chrono::Utc::now().timestamp()
        ],
    )
    .map_err(internal)?;
    tx.commit().map_err(internal)?;
    Ok(
        json!({"preview_token":token,"changes":snapshot.changes,"reason":snapshot.reason,"expires_at_epoch":snapshot.expires_at_epoch}),
    )
}
fn apply(conn: &mut Connection, request: ApplyRequest) -> Result<serde_json::Value> {
    if !request.confirm {
        return Err(Failure(StatusCode::BAD_REQUEST, "confirmation_required"));
    }
    if request.preview_token.len() != 64
        || !request.preview_token.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(Failure(StatusCode::BAD_REQUEST, "preview_token_invalid"));
    }
    let token_hash = digest(request.preview_token.as_bytes());
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(internal)?;
    let detail: Option<String> = tx.query_row("SELECT detail FROM events WHERE session_id=?1 AND event_type=?2 ORDER BY id DESC LIMIT 1",
        params![token_hash,PREVIEW_EVENT], |r|r.get(0)).optional().map_err(internal)?;
    let snapshot: Snapshot =
        serde_json::from_str(&detail.ok_or(Failure(StatusCode::NOT_FOUND, "preview_not_found"))?)
            .map_err(internal)?;
    let applied: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM events WHERE session_id=?1 AND event_type=?2)",
            params![token_hash, APPLIED_EVENT],
            |r| r.get(0),
        )
        .map_err(internal)?;
    if applied {
        return Err(Failure(StatusCode::CONFLICT, "preview_already_applied"));
    }
    let now = chrono::Utc::now().timestamp();
    if now >= snapshot.expires_at_epoch {
        return Err(Failure(StatusCode::CONFLICT, "preview_expired"));
    }
    if snapshot.changes.len() != snapshot.fingerprints.len() {
        return Err(internal("invalid preview snapshot"));
    }
    for (change, expected) in snapshot.changes.iter().zip(&snapshot.fingerprints) {
        let current = load(&tx, &change.target)?;
        if fingerprint(&current)? != *expected {
            return Err(Failure(StatusCode::CONFLICT, "preview_stale"));
        }
        if change.target.kind == Kind::Workstream {
            for title in current
                .title
                .as_deref()
                .into_iter()
                .chain(current.fields.session_topic.as_deref())
                .chain(change.after.session_topic.as_deref())
            {
                crate::workstream::ensure_workstream_alias(
                    &tx,
                    current.row_id,
                    title,
                    "session_intent_override",
                    None,
                    None,
                    now,
                )
                .map_err(internal)?;
            }
        }
        let sql = match change.target.kind {
            Kind::Session => "UPDATE session_summaries SET session_intent=?1,session_topic=?2,session_intent_source='override',session_intent_updated_at_epoch=?3 WHERE id=?4",
            Kind::Workstream => "UPDATE workstreams SET session_intent=?1,session_topic=?2,session_intent_source='override',session_intent_updated_at_epoch=?3 WHERE id=?4",
        };
        tx.execute(
            sql,
            params![
                change.after.session_intent,
                change.after.session_topic,
                now,
                current.row_id
            ],
        )
        .map_err(internal)?;
    }
    tx.execute(
        "INSERT INTO events(session_id,project,event_type,summary,detail,created_at_epoch)
        VALUES (?1,'',?2,'Session intent override applied',?3,?4)",
        params![
            token_hash,
            APPLIED_EVENT,
            serde_json::to_string(&snapshot).map_err(internal)?,
            now
        ],
    )
    .map_err(internal)?;
    let audit_id = tx.last_insert_rowid();
    tx.commit().map_err(internal)?;
    Ok(json!({"audit_id":audit_id,"changes":snapshot.changes,"applied":true}))
}

#[cfg(test)]
#[path = "session_intent/tests/eligible.rs"]
mod eligible_tests;

#[cfg(test)]
mod tests;
