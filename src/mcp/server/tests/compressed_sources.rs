use super::*;

#[test]
fn get_observations_attaches_compressed_observation_sources() {
    let _dir = ScopedTestDataDir::new("mcp-compressed-observation-sources");
    let conn = crate::db::open_db().expect("db opens");
    let source_id = crate::db::insert_observation(
        &conn,
        "source-session",
        "/repo",
        "discovery",
        Some("Source evidence"),
        None,
        Some("Original source observation"),
        None,
        None,
        None,
        None,
        None,
        0,
    )
    .expect("source observation inserts");
    let compressed_id = crate::db::insert_observation(
        &conn,
        "compressed-test",
        "/repo",
        "decision",
        Some("Compressed decision"),
        None,
        Some("Compressed narrative"),
        None,
        None,
        None,
        None,
        None,
        0,
    )
    .expect("compressed observation inserts");
    let sources = crate::db::get_observations_by_ids(&conn, &[source_id], Some("/repo"))
        .expect("source observation loads");
    let expected_hash = crate::db::observation_source_hash(&sources[0]);
    crate::db::insert_compressed_observation_sources(
        &conn,
        &[compressed_id],
        &sources,
        "compressed-test",
    )
    .expect("compressed source links insert");
    drop(conn);

    let server = MemoryServer::new().expect("memory server should initialize");
    let expanded = server
        .get_observations(Parameters(GetObservationsParams {
            ids: vec![compressed_id],
            project: Some("/repo".to_string()),
            source: Some("observation".to_string()),
            include_suppressed: None,
        }))
        .expect("expansion succeeds");
    let json: Value = serde_json::from_str(&expanded).expect("expanded json");

    assert_eq!(json[0]["id"], compressed_id);
    assert_eq!(
        json[0]["compressed_sources"][0]["source_observation_id"],
        source_id
    );
    assert_eq!(
        json[0]["compressed_sources"][0]["source_hash"],
        expected_hash
    );
    assert!(json[0]["compressed_sources"][0]["source_snapshot_json"].is_null());
}
