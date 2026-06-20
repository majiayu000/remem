use anyhow::Result;
use rusqlite::Connection;

use super::types::{MultiHopMeta, SearchRequest, SearchResultSet};

/// Curated hits below this count trigger a raw archive fallback so the caller
/// always has *something* to show when the conversation happened but was
/// never promoted.
const RAW_FALLBACK_THRESHOLD: usize = 3;
const RAW_FALLBACK_LIMIT: i64 = 10;

pub fn search_memories(conn: &Connection, req: &SearchRequest) -> Result<SearchResultSet> {
    let limit = req.limit.max(1);
    let query = req.query.as_deref();

    if req.multi_hop {
        return multi_hop_search(conn, query, req.project.as_deref(), limit, req);
    }

    let (mut memories, mut explain) = if req.explain {
        crate::retrieval::search::search_with_branch_explain_with_suppressed_policy(
            conn,
            query,
            req.project.as_deref(),
            req.memory_type.as_deref(),
            limit + 1,
            req.offset.max(0),
            req.include_stale,
            req.branch.as_deref(),
            req.include_suppressed,
        )?
    } else {
        (
            crate::retrieval::search::search_with_branch_with_suppressed_policy(
                conn,
                query,
                req.project.as_deref(),
                req.memory_type.as_deref(),
                limit + 1,
                req.offset.max(0),
                req.include_stale,
                req.branch.as_deref(),
                req.include_suppressed,
            )?,
            None,
        )
    };
    let has_more = memories.len() as i64 > limit;
    memories.truncate(limit as usize);
    let (raw_hits, raw_error) = maybe_fallback_raw(conn, req, memories.len());
    if let Some(explain) = explain.as_mut() {
        let result_ids: Vec<i64> = memories.iter().map(|memory| memory.id).collect();
        explain.retain_result_ids(&result_ids, has_more, limit);
        explain.set_raw_fallback_count(raw_hits.len());
    }
    Ok(SearchResultSet {
        memories,
        multi_hop: None,
        has_more,
        explain,
        raw_hits,
        raw_error,
    })
}

fn multi_hop_search(
    conn: &Connection,
    query: Option<&str>,
    project: Option<&str>,
    limit: i64,
    req: &SearchRequest,
) -> Result<SearchResultSet> {
    if let Some(query_text) = query.filter(|query_text| !query_text.is_empty()) {
        let mut result = crate::retrieval::search_multihop::search_multi_hop(
            conn,
            query_text,
            project,
            limit + 1,
            req.offset.max(0),
            req.memory_type.as_deref(),
            req.branch.as_deref(),
            req.include_stale,
            req.include_suppressed,
        )?;
        let has_more = result.memories.len() as i64 > limit;
        result.memories.truncate(limit as usize);
        let (raw_hits, raw_error) = maybe_fallback_raw(conn, req, result.memories.len());
        Ok(SearchResultSet {
            memories: result.memories,
            multi_hop: Some(MultiHopMeta {
                hops: result.hops,
                entities_discovered: result.entities_discovered,
            }),
            has_more,
            explain: None,
            raw_hits,
            raw_error,
        })
    } else {
        Ok(SearchResultSet {
            memories: vec![],
            multi_hop: Some(MultiHopMeta {
                hops: 1,
                entities_discovered: vec![],
            }),
            has_more: false,
            explain: None,
            raw_hits: vec![],
            raw_error: None,
        })
    }
}

fn maybe_fallback_raw(
    conn: &Connection,
    req: &SearchRequest,
    curated_len: usize,
) -> (Vec<crate::memory::raw_archive::RawMessage>, Option<String>) {
    if curated_len >= RAW_FALLBACK_THRESHOLD {
        return (vec![], None);
    }
    let Some(query) = req
        .query
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty())
    else {
        return (vec![], None);
    };
    if !req.include_suppressed {
        match crate::memory::suppression::has_active_suppressions(conn) {
            Ok(true) => return (vec![], None),
            Ok(false) => {}
            Err(error) => {
                let message = format!("suppression policy lookup failed: {error}");
                crate::log::error("search", &message);
                return (vec![], Some(message));
            }
        }
    }
    let raw_req = crate::memory::raw_archive::RawSearchRequest {
        query: query.to_string(),
        project: req.project.clone(),
        branch: req.branch.clone(),
        role: None,
        limit: RAW_FALLBACK_LIMIT,
        offset: 0,
    };
    match crate::memory::raw_archive::search_raw_messages(conn, &raw_req) {
        Ok(hits) => (hits, None),
        Err(error) => {
            let message = format!("raw archive fallback failed: {error}");
            crate::log::warn("search", &message);
            (vec![], Some(message))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::raw_archive::{insert_raw_message, ROLE_USER, SOURCE_HOOK};
    use crate::memory::suppression::{create_suppression, parse_target, SuppressRequest};

    #[test]
    fn raw_fallback_respects_branch_filter() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        insert_raw_message(
            &conn,
            "s-main",
            "/repo",
            ROLE_USER,
            "fallback needle from main",
            SOURCE_HOOK,
            Some("main"),
            None,
        )?;
        insert_raw_message(
            &conn,
            "s-feature",
            "/repo",
            ROLE_USER,
            "fallback needle from feature",
            SOURCE_HOOK,
            Some("feature"),
            None,
        )?;
        insert_raw_message(
            &conn,
            "s-branchless",
            "/repo",
            ROLE_USER,
            "fallback needle from branchless history",
            SOURCE_HOOK,
            None,
            None,
        )?;

        let result = search_memories(
            &conn,
            &SearchRequest {
                query: Some("needle".to_string()),
                project: Some("/repo".to_string()),
                limit: 10,
                branch: Some("main".to_string()),
                ..SearchRequest::default()
            },
        )?;
        let branches: Vec<Option<String>> =
            result.raw_hits.into_iter().map(|hit| hit.branch).collect();

        assert!(result.raw_error.is_none());
        assert!(branches.contains(&Some("main".to_string())));
        assert!(branches.contains(&None));
        assert!(
            !branches.contains(&Some("feature".to_string())),
            "{branches:?}"
        );
        Ok(())
    }

    #[test]
    fn raw_fallback_error_is_reported_without_failing_curated_search() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        conn.execute("DROP TABLE raw_messages_fts", [])?;

        let result = search_memories(
            &conn,
            &SearchRequest {
                query: Some("needle".to_string()),
                project: Some("/repo".to_string()),
                limit: 10,
                ..SearchRequest::default()
            },
        )?;

        assert!(result.memories.is_empty());
        assert!(result.raw_hits.is_empty());
        assert!(result
            .raw_error
            .as_deref()
            .is_some_and(|error| error.contains("raw archive fallback failed")));
        Ok(())
    }

    #[test]
    fn search_hides_suppressed_memories_unless_explicitly_included() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        crate::memory::insert_memory(
            &conn,
            Some("s1"),
            "/repo",
            None,
            "Visible needle",
            "The visible suppression needle should remain searchable.",
            "decision",
            None,
        )?;
        let hidden = crate::memory::insert_memory(
            &conn,
            Some("s2"),
            "/repo",
            None,
            "Hidden needle",
            "The hidden suppression needle should require include_suppressed.",
            "decision",
            None,
        )?;
        create_suppression(
            &conn,
            &SuppressRequest {
                target: parse_target(&format!("memory:{hidden}"))?,
                reason: Some("not relevant"),
                actor: Some("test"),
            },
        )?;

        let default = search_memories(
            &conn,
            &SearchRequest {
                query: Some("suppression needle".to_string()),
                project: Some("/repo".to_string()),
                limit: 10,
                ..SearchRequest::default()
            },
        )?;
        let default_ids = default
            .memories
            .iter()
            .map(|memory| memory.id)
            .collect::<Vec<_>>();
        assert!(!default_ids.contains(&hidden), "{default_ids:?}");

        let explicit = search_memories(
            &conn,
            &SearchRequest {
                query: Some("suppression needle".to_string()),
                project: Some("/repo".to_string()),
                limit: 10,
                include_suppressed: true,
                ..SearchRequest::default()
            },
        )?;
        let explicit_ids = explicit
            .memories
            .iter()
            .map(|memory| memory.id)
            .collect::<Vec<_>>();
        assert!(explicit_ids.contains(&hidden), "{explicit_ids:?}");
        Ok(())
    }

    #[test]
    fn raw_fallback_does_not_bypass_active_suppression_policy() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        crate::memory::insert_memory(
            &conn,
            Some("s1"),
            "/repo",
            None,
            "Suppressed memory",
            "Suppressed raw fallback guard.",
            "decision",
            None,
        )?;
        create_suppression(
            &conn,
            &SuppressRequest {
                target: parse_target("memory:1")?,
                reason: Some("not relevant"),
                actor: Some("test"),
            },
        )?;
        insert_raw_message(
            &conn,
            "raw-session",
            "/repo",
            ROLE_USER,
            "fallback-only needle",
            SOURCE_HOOK,
            None,
            None,
        )?;

        let result = search_memories(
            &conn,
            &SearchRequest {
                query: Some("fallback-only needle".to_string()),
                project: Some("/repo".to_string()),
                limit: 10,
                ..SearchRequest::default()
            },
        )?;

        assert!(result.memories.is_empty());
        assert!(result.raw_hits.is_empty());
        assert!(result.raw_error.is_none());
        Ok(())
    }
}
