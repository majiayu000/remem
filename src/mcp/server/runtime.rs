use anyhow::{Context, Result};
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{tool_handler, ServerHandler, ServiceExt};

use super::MemoryServer;
use crate::db;

const SERVER_INSTRUCTIONS: &str = r#"Persistent memory for Claude Code and Codex sessions.

Retrieval:
- Use `search(query, project?)` for compact memory IDs.
- Use `get_observations(ids, source)` only for selected full details.
- Use `search_raw(query)` for literal chat recall when curated search is sparse.
- Use `timeline(anchor/query)` for chronological context around a change.

Persistence:
- Use `save_memory` only for durable decisions, bugfix root causes, important discoveries, architecture notes, or user preferences.
- For user-requested documents, write the local/project file first; memory is only a backup.
- Search before saving and use a stable kebab-case `topic_key` for repeat topics.

Workstreams:
- `workstreams(project)` lists active tasks.
- `update_workstream(id, status?, next_action?, blockers?)` updates status, next action, or blockers."#;

#[tool_handler]
impl ServerHandler for MemoryServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(SERVER_INSTRUCTIONS.into()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

pub async fn run_mcp_server() -> Result<()> {
    let db_path = crate::db::db_path();
    let db_exists = db_path.exists();
    crate::log::info(
        "mcp",
        &format!(
            "server starting db={} exists={}",
            db_path.display(),
            db_exists
        ),
    );
    let server = MemoryServer::new()?;
    let (mem_count, obs_count) = preflight_mcp_database()?;
    crate::log::info(
        "mcp",
        &format!(
            "server ready memories={} observations={}",
            mem_count, obs_count
        ),
    );
    let service = server.serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    crate::log::info("mcp", "server stopped");
    Ok(())
}

fn preflight_mcp_database() -> Result<(i64, i64)> {
    let conn = db::open_db().with_context(mcp_preflight_context)?;
    read_mcp_preflight_counts(&conn).with_context(mcp_preflight_context)
}

fn mcp_preflight_context() -> String {
    format!(
        "MCP server database preflight failed before serving tools; \
         if remem was just upgraded, restart Codex/Claude sessions so MCP servers reconnect to {}; \
         then verify `remem --version` and run `remem doctor`",
        crate::build_info::version_label()
    )
}

fn read_mcp_preflight_counts(conn: &rusqlite::Connection) -> Result<(i64, i64)> {
    let mem_count: i64 = conn
        .query_row("SELECT count(*) FROM memories", [], |row| row.get(0))
        .context("MCP server database preflight could not read memories table")?;
    let obs_count: i64 = conn
        .query_row("SELECT count(*) FROM observations", [], |row| row.get(0))
        .context("MCP server database preflight could not read observations table")?;
    Ok((mem_count, obs_count))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::ScopedTestDataDir;
    use rusqlite::Connection;

    #[test]
    fn mcp_preflight_rejects_newer_schema_before_serving_tools() -> Result<()> {
        let _test_dir = ScopedTestDataDir::new("mcp-preflight-newer-schema");
        let setup = crate::db::open_db()?;
        let future = crate::migrate::latest_schema_version() + 1;
        setup.execute(
            "INSERT INTO _schema_migrations (version, name, applied_at_epoch)
             VALUES (?1, 'future-test', 0)",
            [future],
        )?;
        drop(setup);

        let err = preflight_mcp_database().expect_err("future schema must fail before serve");
        let rendered = format!("{err:#}");
        assert!(
            rendered.contains("preflight failed before serving tools"),
            "{rendered}"
        );
        assert!(
            rendered.contains("restart Codex/Claude sessions"),
            "{rendered}"
        );
        assert!(
            rendered.contains(&format!("schema v{future}")),
            "{rendered}"
        );
        Ok(())
    }

    #[test]
    fn mcp_preflight_rejects_missing_core_tables_before_serving_tools() -> Result<()> {
        let _test_dir = ScopedTestDataDir::new("mcp-preflight-missing-table");
        let setup = crate::db::open_db()?;
        setup.execute("DROP TABLE observations", [])?;
        drop(setup);

        let err = preflight_mcp_database().expect_err("missing table must fail before serve");
        let rendered = format!("{err:#}");
        assert!(
            rendered.contains("preflight failed before serving tools"),
            "{rendered}"
        );
        assert!(
            rendered.contains("no such table: observations"),
            "{rendered}"
        );
        Ok(())
    }

    #[test]
    fn mcp_preflight_count_reader_rejects_missing_core_tables() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute("CREATE TABLE memories(id INTEGER PRIMARY KEY)", [])?;

        let err = read_mcp_preflight_counts(&conn).expect_err("missing observations must fail");
        let rendered = format!("{err:#}");
        assert!(
            rendered.contains("preflight could not read observations table"),
            "{rendered}"
        );
        Ok(())
    }
}
