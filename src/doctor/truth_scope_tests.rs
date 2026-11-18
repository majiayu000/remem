//! `doctor truth` scoping against production-shaped rows (#1136): explicit
//! repo owners (v019 and later), rerouted memories and aliased projects.

use rusqlite::OptionalExtension;

use super::*;

/// Give memory `id` the production ownership shape: v019 and the writer set
/// `owner_scope = 'repo'` on every non-global memory. Rows with one owner and
/// topic share a repo-owned state key named after the topic, as the rows of
/// one semantic slot do, so their subject is
/// `state:repo:{owner}:decision:{topic}`.
fn own(conn: &Connection, id: i64, project: &str, owner: &str) -> Result<()> {
    let topic: String = conn.query_row(
        "SELECT topic_key FROM memories WHERE id = ?1",
        [id],
        |row| row.get(0),
    )?;
    let lookup = "SELECT id FROM memory_state_keys
                  WHERE owner_scope = 'repo' AND owner_key = ?1
                    AND memory_type = 'decision' AND state_key = ?2";
    let existing = conn
        .query_row(lookup, params![owner, topic], |row| row.get::<_, i64>(0))
        .optional()?;
    let state_key_id = match existing {
        Some(state_key_id) => state_key_id,
        None => {
            conn.execute(
                "INSERT INTO memory_state_keys
                 (owner_scope, owner_key, memory_type, state_key,
                  created_at_epoch, updated_at_epoch)
                 VALUES ('repo', ?1, 'decision', ?2, 1, 1)",
                params![owner, topic],
            )?;
            conn.last_insert_rowid()
        }
    };
    conn.execute(
        "UPDATE memories
         SET project = ?2, target_project = ?3, owner_scope = 'repo', owner_key = ?3,
             state_key_id = ?4
         WHERE id = ?1",
        params![id, project, owner, state_key_id],
    )?;
    Ok(())
}

fn supersede(conn: &Connection, old: i64, new: i64, created_at: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_edges (edge_type, from_memory_id, to_memory_id, created_at_epoch)
         VALUES ('supersedes', ?1, ?2, ?3)",
        params![old, new, created_at],
    )?;
    Ok(())
}

fn alias(conn: &Connection, alias_path: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO project_identity_alias_events(
            alias_path, canonical_project_id, action, proof_kind,
            proof_payload_json, proof_sha256, source_inventory_sha256,
            actor, reason, created_at_epoch)
         VALUES(?1, 8100000, 'activate', 'git_commit_membership', '{}',
                ?2, ?3, 'test', 'same project', 1)",
        params![alias_path, "a".repeat(64), "b".repeat(64)],
    )?;
    let event_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO project_identity_aliases(
            alias_path, canonical_project_id, status, last_event_id,
            created_at_epoch, updated_at_epoch)
         VALUES(?1, 8100000, 'active', ?2, 1, 1)",
        params![alias_path, event_id],
    )?;
    Ok(())
}

fn options_for(project: &str, subject: Option<&str>) -> TruthDoctorOptions {
    let mut opts = options();
    opts.project = project.to_string();
    opts.subject = subject.map(str::to_string);
    opts
}

fn memory_lifecycle(report: &TruthDoctorReport) -> Vec<(String, i64)> {
    report
        .lifecycle_mappings
        .iter()
        .filter(|mapping| mapping.object_kind == "memory")
        .map(|mapping| (mapping.stored_status.clone(), mapping.count))
        .collect()
}

fn superseded_and_active() -> Vec<(String, i64)> {
    vec![("active".to_string(), 1), ("superseded".to_string(), 1)]
}

#[test]
fn subject_topic_key_selects_owned_truths_and_scopes_lifecycle_to_their_members() -> Result<()> {
    let conn = test_conn()?;
    insert_memory(&conn, 1, "deploy", "superseded", 10)?;
    insert_memory(&conn, 2, "deploy", "active", 20)?;
    insert_memory(&conn, 3, "runtime", "active", 20)?;
    for id in 1..=3 {
        own(&conn, id, "/repo", "/repo")?;
    }
    supersede(&conn, 1, 2, 20)?;

    let unscoped = build_truth_report(&conn, &options_for("/repo", None))?;
    assert_eq!(unscoped.counts.truth_items, 2);

    for selector in ["deploy", "state:repo:/repo:decision:deploy"] {
        let report = build_truth_report(&conn, &options_for("/repo", Some(selector)))?;
        assert_eq!(report.counts.truth_items, 1, "selector={selector}");
        assert_eq!(report.counts.current, 1, "selector={selector}");
        assert_eq!(
            memory_lifecycle(&report),
            superseded_and_active(),
            "selector={selector}"
        );
        assert_eq!(report.counts.supersedes_relations, 1, "selector={selector}");
        assert_eq!(report.supersedes[0].relation_ref, "memory_edge:1");
    }

    let unknown = build_truth_report(&conn, &options_for("/repo", Some("missing")))?;
    assert_eq!(unknown.counts.truth_items, 0);
    assert!(memory_lifecycle(&unknown).is_empty());
    assert!(unknown.supersedes.is_empty());
    Ok(())
}

#[test]
fn rerouted_memory_reports_its_links_and_lifecycle_under_the_owner() -> Result<()> {
    let conn = test_conn()?;
    insert_memory(&conn, 1, "deploy", "superseded", 10)?;
    insert_memory(&conn, 2, "deploy", "active", 20)?;
    own(&conn, 1, "/other", "/repo")?;
    own(&conn, 2, "/repo", "/repo")?;
    supersede(&conn, 1, 2, 20)?;

    let owner = build_truth_report(&conn, &options_for("/repo", None))?;
    assert_eq!(owner.counts.truth_items, 1);
    assert_eq!(memory_lifecycle(&owner), superseded_and_active());
    assert_eq!(owner.counts.supersedes_relations, 1);

    let source = build_truth_report(&conn, &options_for("/other", None))?;
    assert_eq!(source.counts.truth_items, 0);
    assert!(memory_lifecycle(&source).is_empty(), "{source:?}");
    assert!(source.supersedes.is_empty(), "{source:?}");
    Ok(())
}

#[test]
fn aliased_project_rows_count_and_global_rows_stay_out() -> Result<()> {
    let conn = test_conn()?;
    insert_memory(&conn, 1, "deploy", "superseded", 10)?;
    insert_memory(&conn, 2, "deploy", "active", 20)?;
    insert_memory(&conn, 3, "global-tone", "active", 20)?;
    alias(&conn, "/repo-old")?;
    conn.execute("UPDATE memories SET project = '/repo-old' WHERE id = 1", [])?;
    conn.execute("UPDATE memories SET scope = 'global' WHERE id = 3", [])?;
    supersede(&conn, 1, 2, 20)?;
    conn.execute(
        "INSERT INTO memory_edges (edge_type, from_memory_id, to_memory_id, created_at_epoch)
         VALUES ('conflicts', 3, 3, 20)",
        [],
    )?;

    for project in ["/repo", "/repo-old"] {
        let report = build_truth_report(&conn, &options_for(project, None))?;
        assert_eq!(report.counts.truth_items, 1, "project={project}");
        assert_eq!(
            memory_lifecycle(&report),
            superseded_and_active(),
            "project={project}"
        );
        assert_eq!(report.counts.supersedes_relations, 1, "project={project}");
        assert!(
            report.reference_issues.is_empty(),
            "the global row's self-edge is out of scope: {:?}",
            report.reference_issues
        );
    }
    Ok(())
}
