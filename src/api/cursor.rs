use std::fmt;

use anyhow::Result;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::mutation::canonical_json_bytes;

const CURSOR_VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CursorKind {
    Observations,
    Sessions,
    Workstreams,
    Events,
    Tasks,
}

impl CursorKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Observations => "observations",
            Self::Sessions => "sessions",
            Self::Workstreams => "workstreams",
            Self::Events => "events",
            Self::Tasks => "tasks",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DecodedCursor {
    pub resume_before_id: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CursorError {
    Invalid,
}

impl fmt::Display for CursorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("cursor_invalid")
    }
}

impl std::error::Error for CursorError {}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CursorPayload {
    version: u8,
    kind: String,
    filter_fingerprint: String,
    resume_before_id: i64,
}

pub(crate) fn filter_fingerprint<T: Serialize>(filters: &T) -> Result<String> {
    let value = serde_json::to_value(filters)?;
    let digest = Sha256::digest(canonical_json_bytes(&value)?);
    Ok(format!("{digest:x}"))
}

pub(crate) fn encode_cursor(
    kind: CursorKind,
    filter_fingerprint: &str,
    resume_before_id: i64,
) -> std::result::Result<String, CursorError> {
    if resume_before_id <= 0 || !valid_fingerprint(filter_fingerprint) {
        return Err(CursorError::Invalid);
    }
    let payload = CursorPayload {
        version: CURSOR_VERSION,
        kind: kind.as_str().to_string(),
        filter_fingerprint: filter_fingerprint.to_string(),
        resume_before_id,
    };
    let encoded = serde_json::to_vec(&payload).map_err(|_| CursorError::Invalid)?;
    Ok(URL_SAFE_NO_PAD.encode(encoded))
}

pub(crate) fn decode_cursor(
    encoded: &str,
    expected_kind: CursorKind,
    expected_filter_fingerprint: &str,
) -> std::result::Result<DecodedCursor, CursorError> {
    if encoded.is_empty() || !valid_fingerprint(expected_filter_fingerprint) {
        return Err(CursorError::Invalid);
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| CursorError::Invalid)?;
    let payload: CursorPayload =
        serde_json::from_slice(&bytes).map_err(|_| CursorError::Invalid)?;
    if payload.version != CURSOR_VERSION
        || payload.kind != expected_kind.as_str()
        || payload.filter_fingerprint != expected_filter_fingerprint
        || payload.resume_before_id <= 0
    {
        return Err(CursorError::Invalid);
    }
    Ok(DecodedCursor {
        resume_before_id: payload.resume_before_id,
    })
}

pub(crate) fn continuation_id(
    page_is_full: bool,
    last_returned_safe_id: Option<i64>,
    scan_budget_exhausted: bool,
    last_scanned_raw_id: Option<i64>,
    eligible_rows_exhausted: bool,
) -> std::result::Result<Option<i64>, CursorError> {
    if eligible_rows_exhausted {
        return Ok(None);
    }
    let resume_id = if page_is_full {
        last_returned_safe_id
    } else if scan_budget_exhausted {
        last_scanned_raw_id
    } else {
        return Err(CursorError::Invalid);
    };
    match resume_id {
        Some(id) if id > 0 => Ok(Some(id)),
        _ => Err(CursorError::Invalid),
    }
}

fn valid_fingerprint(fingerprint: &str) -> bool {
    fingerprint.len() == 64
        && fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use base64::Engine as _;
    use serde_json::json;

    use super::*;

    #[test]
    fn cursor_round_trip_binds_kind_filter_and_version() -> Result<()> {
        let filters = filter_fingerprint(&json!({"page_size": 50, "project": "p"}))?;
        let cursor = encode_cursor(CursorKind::Observations, &filters, 91)?;
        assert_eq!(
            decode_cursor(&cursor, CursorKind::Observations, &filters)?,
            DecodedCursor {
                resume_before_id: 91
            }
        );
        assert_eq!(
            decode_cursor(&cursor, CursorKind::Events, &filters),
            Err(CursorError::Invalid)
        );
        let other_filters = filter_fingerprint(&json!({"page_size": 100, "project": "p"}))?;
        assert_eq!(
            decode_cursor(&cursor, CursorKind::Observations, &other_filters),
            Err(CursorError::Invalid)
        );
        Ok(())
    }

    #[test]
    fn malformed_unknown_and_non_positive_cursor_payloads_fail_closed() -> Result<()> {
        let filters = filter_fingerprint(&json!({"page_size": 50}))?;
        for cursor in ["", "not-base64", "e30"] {
            assert_eq!(
                decode_cursor(cursor, CursorKind::Sessions, &filters),
                Err(CursorError::Invalid)
            );
        }
        let invalid_payload = json!({
            "version": 2,
            "kind": "sessions",
            "filter_fingerprint": filters,
            "resume_before_id": 0,
            "unknown": true
        });
        let encoded = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&invalid_payload)?);
        assert_eq!(
            decode_cursor(&encoded, CursorKind::Sessions, &filters),
            Err(CursorError::Invalid)
        );
        Ok(())
    }

    #[test]
    fn continuation_uses_returned_id_for_full_page_and_scanned_id_for_budget_page() {
        assert_eq!(
            continuation_id(true, Some(70), false, Some(60), false),
            Ok(Some(70))
        );
        assert_eq!(
            continuation_id(false, None, true, Some(60), false),
            Ok(Some(60))
        );
        assert_eq!(
            continuation_id(false, None, false, Some(60), true),
            Ok(None)
        );
        assert_eq!(
            continuation_id(true, None, false, Some(60), false),
            Err(CursorError::Invalid)
        );
    }

    #[test]
    fn every_declared_resource_kind_has_a_distinct_binding() {
        let values = [
            CursorKind::Observations,
            CursorKind::Sessions,
            CursorKind::Workstreams,
            CursorKind::Events,
            CursorKind::Tasks,
        ]
        .map(CursorKind::as_str);
        let unique = values
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), 5);
    }
}
