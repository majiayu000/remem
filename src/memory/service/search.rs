use anyhow::Result;
use rusqlite::Connection;

use super::types::{
    MultiHopMeta, SearchRequest, SearchResultSet, SearchResultSetWithExplainDetails,
    SearchRoutingPolicy,
};

/// Curated hits below this count trigger a raw archive fallback so the caller
/// always has *something* to show when the conversation happened but was
/// never promoted.
const RAW_FALLBACK_THRESHOLD: usize = 3;
const RAW_FALLBACK_LIMIT: i64 = 10;

pub fn search_memories(conn: &Connection, req: &SearchRequest) -> Result<SearchResultSet> {
    search_memories_with_explain_details(conn, req).map(|result| result.result)
}

pub(crate) fn search_memories_with_explain_details(
    conn: &Connection,
    req: &SearchRequest,
) -> Result<SearchResultSetWithExplainDetails> {
    search_memories_with_explain_details_with_routing(conn, req, None)
}

pub(crate) fn search_memories_with_explain_details_with_routing(
    conn: &Connection,
    req: &SearchRequest,
    routing: Option<&SearchRoutingPolicy>,
) -> Result<SearchResultSetWithExplainDetails> {
    let limit = req.limit.max(1);
    let query = req.query.as_deref();
    let effective_multi_hop = routing.is_some_and(|routing| routing.use_multi_hop) || req.multi_hop;

    if effective_multi_hop {
        return multi_hop_search(conn, query, req.project.as_deref(), limit, req, routing).map(
            |result| SearchResultSetWithExplainDetails {
                result,
                explain_details: None,
            },
        );
    }

    let (mut memories, mut explain_details) = if req.explain {
        crate::retrieval::search::search_with_branch_execution_policy_with_suppressed_policy(
            conn,
            query,
            req.project.as_deref(),
            req.memory_type.as_deref(),
            limit + 1,
            req.offset.max(0),
            req.include_stale,
            req.branch.as_deref(),
            req.include_suppressed,
            true,
            search_execution_policy(true, routing),
        )?
    } else {
        crate::retrieval::search::search_with_branch_execution_policy_with_suppressed_policy(
            conn,
            query,
            req.project.as_deref(),
            req.memory_type.as_deref(),
            limit + 1,
            req.offset.max(0),
            req.include_stale,
            req.branch.as_deref(),
            req.include_suppressed,
            false,
            search_execution_policy(false, routing),
        )?
    };
    let has_more = memories.len() as i64 > limit;
    memories.truncate(limit as usize);
    let (raw_hits, raw_error) = maybe_fallback_raw(conn, req, memories.len(), routing);
    if let Some(explain) = explain_details.as_mut() {
        let result_ids: Vec<i64> = memories.iter().map(|memory| memory.id).collect();
        explain.retain_result_ids(&result_ids, has_more, limit);
        explain.set_raw_fallback_count(raw_hits.len());
    }
    Ok(SearchResultSetWithExplainDetails {
        result: SearchResultSet {
            memories,
            multi_hop: None,
            has_more,
            explain: explain_details
                .as_ref()
                .map(|details| details.explain.clone()),
            raw_hits,
            raw_error,
        },
        explain_details,
    })
}

fn multi_hop_search(
    conn: &Connection,
    query: Option<&str>,
    project: Option<&str>,
    limit: i64,
    req: &SearchRequest,
    routing: Option<&SearchRoutingPolicy>,
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
        let (raw_hits, raw_error) = maybe_fallback_raw(conn, req, result.memories.len(), routing);
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
    routing: Option<&SearchRoutingPolicy>,
) -> (Vec<crate::memory::raw_archive::RawMessage>, Option<String>) {
    if routing.is_some_and(|routing| !routing.raw_fallback_enabled) {
        return (vec![], None);
    }
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
        since_epoch: None,
        until_epoch: None,
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

fn search_execution_policy(
    explain: bool,
    routing: Option<&SearchRoutingPolicy>,
) -> crate::retrieval::search::SearchExecutionPolicy {
    let base = if explain {
        crate::retrieval::search::SearchWeights::default()
    } else {
        crate::retrieval::search::SearchWeights::production()
    };
    let Some(routing) = routing else {
        return if explain {
            crate::retrieval::search::SearchExecutionPolicy::explain_default()
        } else {
            crate::retrieval::search::SearchExecutionPolicy::production()
        };
    };
    let mut weights = base;
    weights.fts = routing.weights.fts;
    weights.vector = routing.weights.vector;
    weights.entity = routing.weights.entity;
    weights.graph = routing.weights.graph;
    weights.temporal = routing.weights.temporal;
    weights.fact = routing.weights.fact;
    weights.like_fallback = routing.weights.like_fallback;
    weights.usage = routing.weights.usage;
    crate::retrieval::search::SearchExecutionPolicy::routed(
        weights,
        routing.rerank_enabled,
        routing.rerank_candidate_pool,
        routing.rerank_output_k,
    )
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
    fn search_recovers_legacy_unverified_without_underfilling_pagination() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        for id in 1..=2 {
            crate::memory::insert_memory(
                &conn,
                Some("s"),
                "/repo",
                None,
                &format!("needle {id}"),
                "needle legacy searchable payload",
                "bugfix",
                None,
            )?;
        }
        let result = search_memories(
            &conn,
            &SearchRequest {
                query: Some("needle".into()),
                project: Some("/repo".into()),
                limit: 1,
                ..SearchRequest::default()
            },
        )?;
        assert_eq!(result.memories.len(), 1);
        assert!(result.has_more);
        let visibility = crate::truth::classify_memory(
            &conn,
            result.memories[0].id,
            chrono::Utc::now().timestamp(),
        )?;
        assert_eq!(visibility.classification.as_str(), "legacy_unverified");
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
