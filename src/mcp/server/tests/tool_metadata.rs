use super::MemoryServer;
use crate::db::test_support::ScopedTestDataDir;

#[test]
fn memory_server_new_does_not_open_database_eagerly() {
    let test_dir = ScopedTestDataDir::new("mcp-new");
    let db_path = test_dir.db_path();
    assert!(!db_path.exists());

    let _server = MemoryServer::new().expect("memory server should initialize");
    assert!(!db_path.exists());
}

#[test]
fn get_observations_tool_description_labels_observations_as_current() -> anyhow::Result<()> {
    let server = MemoryServer::new()?;
    let route = server
        .tool_router
        .map
        .get("get_observations")
        .expect("get_observations tool should be registered");
    let description = route
        .attr
        .description
        .as_deref()
        .expect("tool should have a description");

    assert!(description.contains("source='observation'"));
    assert!(description.contains("current extracted observations"));
    assert!(!description.contains("legacy observations"));
    Ok(())
}
