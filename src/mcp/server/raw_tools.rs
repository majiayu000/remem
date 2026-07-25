use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router};

use super::super::types::{ListRawSessionsParams, SearchRawParams};
use super::errors::{self, McpToolError, McpToolResult};
use super::MemoryServer;
use crate::memory::raw_archive;
use crate::memory::raw_query::{
    build_raw_search_json, parse_time_lower_bound, parse_time_upper_bound,
};

#[tool_router(router = tool_router_raw, vis = "pub(super)")]
impl MemoryServer {
    #[tool(
        description = "Search the raw archive (every user/assistant turn captured by the Stop hook). \
        Use this when `search` returns no curated match or you need to recall a literal phrase from past chats. \
        Returns the untreated conversation content — expect noise. \
        The raw archive is what guarantees 'what was said remains searchable' even when summarize/promote skip a turn."
    )]
    pub(super) fn search_raw(
        &self,
        Parameters(params): Parameters<SearchRawParams>,
    ) -> McpToolResult<String> {
        const TOOL: &str = "search_raw";
        let start = std::time::Instant::now();
        let normalized_limit = params.limit.unwrap_or(20).max(1);
        let normalized_offset = params.offset.unwrap_or(0).max(0);
        crate::log::info(
            "mcp",
            &format!(
                "search_raw called query={:?} project={:?} branch={:?} role={:?} limit={} offset={}",
                params.query,
                params.project,
                params.branch,
                params.role,
                normalized_limit,
                normalized_offset,
            ),
        );
        let since_epoch = parse_optional_bound(
            TOOL,
            "since",
            params.since.as_deref(),
            parse_time_lower_bound,
        )?;
        let until_epoch = parse_optional_bound(
            TOOL,
            "until",
            params.until.as_deref(),
            parse_time_upper_bound,
        )?;
        self.with_conn(TOOL, |conn| {
            let req = raw_archive::RawSearchRequest {
                query: params.query.clone(),
                project: params.project.clone(),
                branch: params.branch.clone(),
                role: params.role.clone(),
                limit: normalized_limit.saturating_add(1),
                offset: normalized_offset,
                since_epoch,
                until_epoch,
            };
            let mut hits = raw_archive::search_raw_messages(conn, &req).map_err(|e| {
                crate::log::warn("mcp", &format!("search_raw failed: {}", e));
                McpToolError::db_query(TOOL, e)
            })?;
            let has_more = hits.len() as i64 > normalized_limit;
            hits.truncate(normalized_limit as usize);

            crate::log::info(
                "mcp",
                &format!(
                    "search_raw done count={} {}ms",
                    hits.len(),
                    start.elapsed().as_millis()
                ),
            );
            let output = build_raw_search_json(
                &params.query,
                params.project.as_deref(),
                params.branch.as_deref(),
                params.role.as_deref(),
                normalized_limit,
                normalized_offset,
                since_epoch,
                until_epoch,
                has_more,
                &hits,
            );
            errors::to_json_pretty(TOOL, &output)
        })
    }

    #[tool(
        description = "List sessions with raw archive messages inside a time window, grouped by \
        (source_root, project, session_id) with first/last message epoch, message count, and optional \
        role=user message samples. Use for recap-style summaries of what happened in a period. \
        Output fields match `remem raw sessions --json`."
    )]
    pub(super) fn list_raw_sessions(
        &self,
        Parameters(params): Parameters<ListRawSessionsParams>,
    ) -> McpToolResult<String> {
        const TOOL: &str = "list_raw_sessions";
        let start = std::time::Instant::now();
        crate::log::info(
            "mcp",
            &format!(
                "list_raw_sessions called since={:?} until={:?} project={:?} sample={}",
                params.since,
                params.until,
                params.project,
                params.sample.unwrap_or(0),
            ),
        );
        let since_epoch = parse_optional_bound(
            TOOL,
            "since",
            params.since.as_deref(),
            parse_time_lower_bound,
        )?;
        let until_epoch = parse_optional_bound(
            TOOL,
            "until",
            params.until.as_deref(),
            parse_time_upper_bound,
        )?;
        self.with_conn(TOOL, |conn| {
            let query = raw_archive::RawSessionQuery {
                since_epoch,
                until_epoch,
                project: params.project.clone(),
                sample_user_messages: params.sample.unwrap_or(0).max(0),
            };
            let sessions = raw_archive::list_sessions(conn, &query).map_err(|e| {
                crate::log::warn("mcp", &format!("list_raw_sessions failed: {}", e));
                McpToolError::db_query(TOOL, e)
            })?;

            crate::log::info(
                "mcp",
                &format!(
                    "list_raw_sessions done count={} {}ms",
                    sessions.len(),
                    start.elapsed().as_millis()
                ),
            );
            errors::to_json_pretty(TOOL, &raw_archive::build_sessions_json(&query, sessions))
        })
    }
}

fn parse_optional_bound(
    tool: &'static str,
    field: &str,
    value: Option<&str>,
    parser: fn(&str) -> anyhow::Result<i64>,
) -> Result<Option<i64>, McpToolError> {
    value.map(parser).transpose().map_err(|e| {
        crate::log::warn("mcp", &format!("{tool} invalid {field}: {e}"));
        McpToolError::invalid_request(tool, e.to_string())
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use rmcp::handler::server::wrapper::Parameters;
    use serde_json::Value;

    use super::*;
    use crate::db::test_support::ScopedTestDataDir;
    use crate::memory::raw_archive::{
        insert_raw_message, insert_raw_message_from_root_at, ROLE_USER, SOURCE_HOOK,
        SOURCE_ROOT_LOCAL,
    };

    #[test]
    fn search_raw_returns_the_cli_json_envelope() -> anyhow::Result<()> {
        let _dir = ScopedTestDataDir::new("mcp-raw-search-envelope");
        let conn = crate::db::open_db()?;
        for content in ["first literal needle", "second literal needle"] {
            insert_raw_message(
                &conn,
                "session-raw",
                "/repo",
                ROLE_USER,
                content,
                SOURCE_HOOK,
                Some("main"),
                Some("/repo"),
            )?;
        }
        drop(conn);

        let server = MemoryServer::new()?;
        let response = server
            .search_raw(Parameters(SearchRawParams {
                query: "literal needle".to_string(),
                project: Some("/repo".to_string()),
                branch: Some("main".to_string()),
                role: Some(ROLE_USER.to_string()),
                limit: Some(1),
                offset: Some(0),
                since: Some("0".to_string()),
                until: Some("9999999999".to_string()),
            }))
            .map_err(anyhow::Error::msg)?;
        let json: Value = serde_json::from_str(&response)?;

        let keys = json
            .as_object()
            .expect("raw search response must be an object")
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            keys,
            BTreeSet::from([
                "branch",
                "count",
                "has_more",
                "limit",
                "next_offset",
                "note",
                "offset",
                "project",
                "query",
                "results",
                "role",
                "since_epoch",
                "source_type",
                "until_epoch",
            ])
        );
        assert_eq!(json["query"], "literal needle");
        assert_eq!(json["limit"], 1);
        assert_eq!(json["offset"], 0);
        assert_eq!(json["count"], 1);
        assert_eq!(json["has_more"], true);
        assert_eq!(json["next_offset"], 1);
        assert_eq!(json["source_type"], "raw_archive");
        assert!(json["results"][0]["content"]
            .as_str()
            .is_some_and(|content| content.contains("literal needle")));
        assert_eq!(json["results"][0]["cwd"], "/repo");
        assert!(json["results"][0].get("preview").is_none());
        Ok(())
    }

    #[test]
    fn search_raw_date_only_until_includes_messages_later_that_utc_day() -> anyhow::Result<()> {
        let _dir = ScopedTestDataDir::new("mcp-raw-search-date-until");
        let conn = crate::db::open_db()?;
        insert_raw_message_from_root_at(
            &conn,
            "session-date-bound",
            "/repo",
            ROLE_USER,
            "midday boundary needle",
            SOURCE_HOOK,
            Some("main"),
            Some("/repo"),
            SOURCE_ROOT_LOCAL,
            Some(1_767_355_200),
        )?;
        drop(conn);

        let server = MemoryServer::new()?;
        let response = server
            .search_raw(Parameters(SearchRawParams {
                query: "boundary needle".to_string(),
                project: Some("/repo".to_string()),
                branch: Some("main".to_string()),
                role: Some(ROLE_USER.to_string()),
                limit: Some(20),
                offset: Some(0),
                since: Some("2026-01-02".to_string()),
                until: Some("2026-01-02".to_string()),
            }))
            .map_err(anyhow::Error::msg)?;
        let json: Value = serde_json::from_str(&response)?;

        assert_eq!(json["since_epoch"], 1_767_312_000_i64);
        assert_eq!(json["until_epoch"], 1_767_398_399_i64);
        assert_eq!(json["count"], 1);
        assert_eq!(json["results"][0]["content"], "midday boundary needle");
        Ok(())
    }
}
