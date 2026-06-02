use super::*;
use crate::memory;
use anyhow::Result;
use rusqlite::{params, Connection};

const STASH: &str = "/Users/lifcc/Desktop/code/AI/tool/stash";
const NOW: i64 = 1_780_000_000;

fn setup_conn() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    memory::types::tests_helper::setup_memory_schema(&conn);
    setup_workstream_schema(&conn);
    conn
}

fn setup_workstream_schema(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE workstreams (
            id INTEGER PRIMARY KEY,
            project TEXT NOT NULL,
            title TEXT NOT NULL,
            description TEXT,
            status TEXT NOT NULL DEFAULT 'active',
            progress TEXT,
            next_action TEXT,
            blockers TEXT,
            created_at_epoch INTEGER NOT NULL,
            updated_at_epoch INTEGER NOT NULL,
            completed_at_epoch INTEGER,
            source_project TEXT,
            target_project TEXT,
            owner_scope TEXT,
            owner_key TEXT,
            topic_domain TEXT,
            routing_confidence REAL,
            routing_reason TEXT,
            context_class TEXT,
            expires_at_epoch INTEGER,
            valid_from_epoch INTEGER,
            valid_to_epoch INTEGER
        );",
    )
    .unwrap();
}

fn seed_stash_pollution(conn: &Connection) {
    let memory_rows = [
        (
            1001,
            "Stash DnD sensors",
            "Stash DnD uses pointer sensors for drag and drop.",
            "architecture",
            "stash-ui",
            0.95,
            None,
        ),
        (
            1002,
            "Stash dev server",
            "Stash dev server runs on Vite port 5173.",
            "discovery",
            "stash-dev",
            0.95,
            Some(NOW + 86_400),
        ),
        (
            1010,
            "Codex sandbox",
            "Codex CLI uses workspace-write sandbox approvals.",
            "discovery",
            "codex-sandbox",
            0.95,
            None,
        ),
        (
            1011,
            "Codex approval prompts",
            "Codex approval prompts require explicit confirm before destructive commands.",
            "procedure",
            "codex-sandbox",
            0.95,
            None,
        ),
        (
            1020,
            "Grok API payloads",
            "Grok API supports image reference payloads.",
            "discovery",
            "grok-api",
            0.95,
            None,
        ),
        (
            1021,
            "Warp macOS launch",
            "Warp terminal launch behavior depends on macOS app routing.",
            "discovery",
            "macos",
            0.95,
            None,
        ),
        (
            1030,
            "Preference: direct UI critique",
            "Prefer direct UI critique in Stash reviews.",
            "preference",
            "stash-ui",
            0.95,
            None,
        ),
        (
            1031,
            "Preference: Stash UI critique",
            "For Stash UI reviews, prefer direct critique and avoid decorative fluff.",
            "preference",
            "stash-ui",
            0.95,
            None,
        ),
        (
            1032,
            "Preference: concise critique",
            "Prefer concise UI critique when reviewing Stash.",
            "preference",
            "stash-ui",
            0.95,
            None,
        ),
        (
            1040,
            "Stash dev server health",
            "Stash dev server is currently healthy on localhost:5173.",
            "session_activity",
            "stash-dev",
            0.95,
            Some(NOW - 3600),
        ),
        (
            1050,
            "Ambiguous sidebar behavior",
            "The sidebar behavior may belong to Stash or a generic browser tool.",
            "discovery",
            "stash-ui",
            0.42,
            None,
        ),
        (
            1060,
            "Stash repo Codex approval UI",
            "Stash repo implements Codex approval UI copy for its own settings screen.",
            "discovery",
            "stash-ui",
            0.95,
            None,
        ),
    ];

    for (id, title, content, memory_type, topic_domain, confidence, expires_at_epoch) in memory_rows
    {
        conn.execute(
            "INSERT INTO memories
             (id, session_id, project, topic_key, title, content, memory_type,
              created_at_epoch, updated_at_epoch, status, scope, source_project,
              target_project, owner_scope, owner_key, topic_domain,
              routing_confidence, context_class, expires_at_epoch)
             VALUES
             (?1, 'stash-session', ?2, ?3, ?4, ?5, ?6,
              ?7, ?8, 'active', 'project', ?2, ?2, 'repo', ?2, ?9, ?10,
              'startup_core', ?11)",
            params![
                id,
                STASH,
                format!("topic-{id}"),
                title,
                content,
                memory_type,
                NOW - (2000 - id),
                NOW - (2000 - id),
                topic_domain,
                confidence,
                expires_at_epoch
            ],
        )
        .unwrap();
    }
    conn.execute(
        "UPDATE memories
         SET owner_scope = NULL, owner_key = NULL, routing_confidence = 0.42
         WHERE id = 1050",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type,
          created_at_epoch, updated_at_epoch, status, scope, source_project,
          target_project, owner_scope, owner_key, topic_domain,
          routing_confidence, context_class)
         VALUES
         (1051, 'stash-session', ?1, 'topic-1051', 'Backfilled route',
          'A v019 backfilled route should still be reviewed when confidence is missing.',
          'discovery', ?2, ?2, 'active', 'project', ?1, ?1, 'repo', ?1,
          'stash-ui', NULL, 'startup_core')",
        params![STASH, NOW - 10],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type,
          created_at_epoch, updated_at_epoch, status, scope, source_project,
          target_project, owner_scope, owner_key, topic_domain,
          routing_confidence, context_class)
         VALUES
         (1033, 'stash-session', ?1, 'topic-1033', 'Preference: global UI critique',
          'Prefer direct UI critique for all product reviews.',
          'preference', ?2, ?2, 'active', 'global', ?1, NULL, 'user',
          'user:default', 'user-preference', 0.95, 'startup_core')",
        params![STASH, NOW - 9],
    )
    .unwrap();

    let workstream_rows = [
        (2001, "Finish Stash drag-and-drop polish", "stash-ui", 0.95),
        (2010, "Build Grok image API wrapper", "grok-api", 0.95),
        (2011, "Fix Warp terminal startup routing", "macos", 0.95),
        (2012, "Investigate Hermes MCP transport", "hermes", 0.95),
        (2020, "Polish Stash sidebar interactions", "stash-ui", 0.95),
        (2021, "Stash sidebar polish follow-up", "stash-ui", 0.95),
    ];

    for (id, title, topic_domain, confidence) in workstream_rows {
        conn.execute(
            "INSERT INTO workstreams
             (id, project, title, status, progress, created_at_epoch, updated_at_epoch,
              source_project, target_project, owner_scope, owner_key, topic_domain,
              routing_confidence, context_class)
             VALUES (?1, ?2, ?3, 'active', 'in progress', ?4, ?5,
                     ?2, ?2, 'repo', ?2, ?6, ?7, 'startup_core')",
            params![
                id,
                STASH,
                title,
                NOW - (3000 - id),
                NOW - (3000 - id),
                topic_domain,
                confidence
            ],
        )
        .unwrap();
    }
}

fn refs(items: &[&str]) -> Vec<String> {
    items.iter().map(|value| value.to_string()).collect()
}

fn has_ref(items: &[AuditItem], object_ref: &str) -> bool {
    items.iter().any(|item| item.object_ref == object_ref)
}

#[test]
fn stash_pollution_audit_classifies_cleanup_buckets() -> Result<()> {
    let conn = setup_conn();
    seed_stash_pollution(&conn);

    let report = audit_scope(
        &conn,
        &ScopeAuditRequest {
            project: STASH,
            limit: 100,
            now_epoch: NOW,
        },
    )?;

    assert!(has_ref(&report.likely_correct_repo_memory, "memory:1001"));
    assert!(has_ref(&report.likely_correct_repo_memory, "memory:1002"));
    assert!(has_ref(
        &report.likely_correct_repo_memory,
        "workstream:2001"
    ));

    let pollution = &report.likely_cross_tool_domain_pollution;
    assert!(has_ref(pollution, "memory:1010"));
    assert!(has_ref(pollution, "memory:1011"));
    assert!(has_ref(pollution, "memory:1020"));
    assert!(has_ref(pollution, "memory:1021"));
    assert!(has_ref(pollution, "workstream:2010"));
    assert!(has_ref(pollution, "workstream:2011"));
    assert!(has_ref(pollution, "workstream:2012"));
    assert!(!has_ref(pollution, "memory:1060"));

    let codex = pollution
        .iter()
        .find(|item| item.object_ref == "memory:1010")
        .unwrap();
    assert_eq!(codex.suggested_owner_scope.as_deref(), Some("tool"));
    assert_eq!(codex.suggested_owner_key.as_deref(), Some("codex-cli"));
    assert_eq!(codex.suggested_target_project, None);

    assert!(report
        .duplicate_preferences
        .iter()
        .any(|cluster| cluster.cluster_key == "ui-critique"
            && cluster.canonical_ref == "memory:1030"
            && cluster.refs.contains(&"memory:1030".to_string())
            && cluster.refs.contains(&"memory:1032".to_string())
            && !cluster.refs.contains(&"memory:1033".to_string())));
    assert!(report
        .duplicate_workstreams
        .iter()
        .any(|cluster| cluster.cluster_key == "stash-sidebar-polish"
            && cluster.canonical_ref == "workstream:2020"
            && cluster.refs.contains(&"workstream:2020".to_string())
            && cluster.refs.contains(&"workstream:2021".to_string())));
    assert!(has_ref(&report.stale_temporal_facts, "memory:1040"));
    assert!(has_ref(&report.low_confidence_routing, "memory:1050"));
    assert!(has_ref(&report.low_confidence_routing, "memory:1051"));
    assert!(has_ref(&report.low_confidence_routing, "memory:1060"));
    Ok(())
}

#[test]
fn reroute_defaults_to_dry_run_and_confirm_preserves_provenance() -> Result<()> {
    let conn = setup_conn();
    seed_stash_pollution(&conn);
    let parsed_refs = parse_object_refs(&refs(&["memory:1010,memory:1011"]))?;

    let preview = reroute_objects(
        &conn,
        &RerouteRequest {
            refs: &parsed_refs,
            owner_scope: "tool",
            owner_key: "codex-cli",
            target_project: TargetProjectUpdate::Clear,
            topic_domain: Some("codex-sandbox"),
            context_class: Some("search_only"),
            routing_confidence: Some(1.0),
            reason: Some("Stash scope cleanup"),
            dry_run: false,
            confirm: false,
        },
    )?;

    assert!(preview.dry_run);
    assert_eq!(preview.affected.len(), 2);
    let unchanged: (String, String, Option<String>, String) = conn.query_row(
        "SELECT owner_scope, owner_key, target_project, status FROM memories WHERE id = 1010",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(
        unchanged,
        (
            "repo".to_string(),
            STASH.to_string(),
            Some(STASH.to_string()),
            "active".to_string()
        )
    );
    let event_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE event_type = 'scope_cleanup'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(event_count, 0);

    let applied = reroute_objects(
        &conn,
        &RerouteRequest {
            refs: &parsed_refs,
            owner_scope: "tool",
            owner_key: "codex-cli",
            target_project: TargetProjectUpdate::Clear,
            topic_domain: Some("codex-sandbox"),
            context_class: Some("search_only"),
            routing_confidence: Some(1.0),
            reason: Some("Stash scope cleanup"),
            dry_run: false,
            confirm: true,
        },
    )?;

    assert!(!applied.dry_run);
    let routed: (String, String, Option<String>, String, String) = conn.query_row(
        "SELECT owner_scope, owner_key, target_project, source_project, status
         FROM memories WHERE id = 1010",
        [],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )?;
    assert_eq!(
        routed,
        (
            "tool".to_string(),
            "codex-cli".to_string(),
            None,
            STASH.to_string(),
            "active".to_string()
        )
    );
    let event_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE event_type = 'scope_cleanup'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(event_count, 2);
    Ok(())
}

#[test]
fn audit_limit_is_applied_per_bucket_after_classification() -> Result<()> {
    let conn = setup_conn();
    let now = NOW;
    for id in 1..=8 {
        conn.execute(
            "INSERT INTO memories
             (id, session_id, project, topic_key, title, content, memory_type,
              created_at_epoch, updated_at_epoch, status, scope, source_project,
              target_project, owner_scope, owner_key, topic_domain,
              routing_confidence, context_class)
             VALUES
             (?1, 'limit-session', ?2, ?3, ?4, 'Stash repo UI memory.',
              'discovery', ?5, ?5, 'active', 'project', ?2, ?2, 'repo',
              ?2, 'stash-ui', 0.95, 'startup_core')",
            params![
                id,
                STASH,
                format!("topic-limit-{id}"),
                format!("Correct repo memory {id}"),
                now + id
            ],
        )?;
    }
    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type,
          created_at_epoch, updated_at_epoch, status, scope, source_project,
          target_project, owner_scope, owner_key, topic_domain,
          routing_confidence, context_class)
         VALUES
         (99, 'limit-session', ?1, 'topic-old-codex', 'Codex sandbox',
          'Codex CLI uses workspace-write sandbox approvals.', 'discovery',
          ?2, ?2, 'active', 'project', ?1, ?1, 'repo', ?1, 'codex-sandbox',
          0.95, 'startup_core')",
        params![STASH, now - 10_000],
    )?;

    let report = audit_scope(
        &conn,
        &ScopeAuditRequest {
            project: STASH,
            limit: 1,
            now_epoch: now,
        },
    )?;

    assert_eq!(report.likely_correct_repo_memory.len(), 1);
    assert_eq!(report.likely_cross_tool_domain_pollution.len(), 1);
    assert_eq!(
        report.likely_cross_tool_domain_pollution[0].object_ref,
        "memory:99"
    );
    Ok(())
}

#[test]
fn reroute_trims_owner_scope_and_key_before_persisting() -> Result<()> {
    let conn = setup_conn();
    seed_stash_pollution(&conn);
    let parsed_refs = parse_object_refs(&refs(&["memory:1010"]))?;

    reroute_objects(
        &conn,
        &RerouteRequest {
            refs: &parsed_refs,
            owner_scope: " tool ",
            owner_key: " codex-cli ",
            target_project: TargetProjectUpdate::Clear,
            topic_domain: Some("codex-sandbox"),
            context_class: None,
            routing_confidence: Some(1.0),
            reason: Some("trim owner fields"),
            dry_run: false,
            confirm: true,
        },
    )?;

    let routed: (String, String, Option<String>, String) = conn.query_row(
        "SELECT owner_scope, owner_key, target_project, context_class
         FROM memories WHERE id = 1010",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(
        routed,
        (
            "tool".to_string(),
            "codex-cli".to_string(),
            None,
            "search_only".to_string()
        )
    );
    Ok(())
}

#[test]
fn archive_mixed_refs_does_not_hard_delete_and_rolls_back_on_invalid_ref() -> Result<()> {
    let conn = setup_conn();
    seed_stash_pollution(&conn);
    let parsed_refs = parse_object_refs(&refs(&["memory:1040", "workstream:2010"]))?;

    let preview = archive_objects(
        &conn,
        &ArchiveRequest {
            refs: &parsed_refs,
            reason: Some("not startup context"),
            dry_run: false,
            confirm: false,
        },
    )?;
    assert!(preview.dry_run);
    assert_eq!(
        conn.query_row("SELECT status FROM memories WHERE id = 1040", [], |row| {
            row.get::<_, String>(0)
        })?,
        "active"
    );

    let applied = archive_objects(
        &conn,
        &ArchiveRequest {
            refs: &parsed_refs,
            reason: Some("not startup context"),
            dry_run: false,
            confirm: true,
        },
    )?;
    assert!(!applied.dry_run);
    assert_eq!(
        conn.query_row("SELECT status FROM memories WHERE id = 1040", [], |row| {
            row.get::<_, String>(0)
        })?,
        "archived"
    );
    assert_eq!(
        conn.query_row(
            "SELECT status FROM workstreams WHERE id = 2010",
            [],
            |row| { row.get::<_, String>(0) }
        )?,
        "paused"
    );
    let remaining: i64 =
        conn.query_row("SELECT COUNT(*) FROM memories WHERE id = 1040", [], |row| {
            row.get(0)
        })?;
    assert_eq!(remaining, 1);

    let conn = setup_conn();
    seed_stash_pollution(&conn);
    let bad_refs = parse_object_refs(&refs(&["memory:1040", "workstream:999"]))?;
    let err = archive_objects(
        &conn,
        &ArchiveRequest {
            refs: &bad_refs,
            reason: Some("rollback test"),
            dry_run: false,
            confirm: true,
        },
    )
    .expect_err("invalid mixed refs should fail");
    assert!(err.to_string().contains("workstream:999 not found"));
    assert_eq!(
        conn.query_row("SELECT status FROM memories WHERE id = 1040", [], |row| {
            row.get::<_, String>(0)
        })?,
        "active"
    );
    Ok(())
}

#[test]
fn merge_preferences_keeps_one_active_preference_with_merged_content() -> Result<()> {
    let conn = setup_conn();
    seed_stash_pollution(&conn);

    let preview = merge_preferences(
        &conn,
        &MergePreferencesRequest {
            project: STASH,
            dry_run: false,
            confirm: false,
        },
    )?;
    assert!(preview.dry_run);
    assert_eq!(preview.clusters.len(), 1);
    assert_eq!(preview.clusters[0].canonical_ref, "memory:1030");
    let merged = preview.clusters[0].merged_content.as_deref().unwrap();
    assert!(merged.contains("direct UI critique"));
    assert!(merged.contains("avoid decorative fluff"));
    assert!(merged.contains("concise UI critique"));
    assert_eq!(
        conn.query_row("SELECT status FROM memories WHERE id = 1030", [], |row| {
            row.get::<_, String>(0)
        })?,
        "active"
    );

    let applied = merge_preferences(
        &conn,
        &MergePreferencesRequest {
            project: STASH,
            dry_run: false,
            confirm: true,
        },
    )?;
    assert!(!applied.dry_run);
    assert_eq!(applied.affected.len(), 3);
    let canonical_mutation = applied
        .affected
        .iter()
        .find(|mutation| mutation.object_ref == "memory:1030")
        .expect("canonical mutation should be reported");
    assert_eq!(
        canonical_mutation.new_owner.owner_scope.as_deref(),
        Some("repo")
    );
    let canonical: (String, String, String, String, String) = conn.query_row(
        "SELECT status, content, owner_scope, owner_key, target_project
         FROM memories WHERE id = 1030",
        [],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )?;
    assert_eq!(canonical.0, "active");
    assert!(canonical.1.contains("direct UI critique"));
    assert!(canonical.1.contains("avoid decorative fluff"));
    assert!(canonical.1.contains("concise UI critique"));
    assert_eq!(canonical.2, "repo");
    assert_eq!(canonical.3, STASH);
    assert_eq!(canonical.4, STASH);
    for id in [1031, 1032] {
        assert_eq!(
            conn.query_row(
                "SELECT status, source_project FROM memories WHERE id = ?1",
                params![id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            )?,
            ("stale".to_string(), STASH.to_string())
        );
    }
    let global_pref: (String, String, Option<String>) = conn.query_row(
        "SELECT status, owner_scope, target_project FROM memories WHERE id = 1033",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(
        global_pref,
        ("active".to_string(), "user".to_string(), None)
    );
    let prefs = memory::preference::query_project_preferences(&conn, STASH, 10)?;
    let ui_prefs = prefs
        .iter()
        .filter(|pref| pref.text.contains("UI critique"))
        .count();
    assert_eq!(ui_prefs, 1);
    let event_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE event_type = 'scope_cleanup'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(event_count, 3);
    let duplicate_edges = conn
        .prepare(
            "SELECT from_memory_id, to_memory_id, edge_type
             FROM memory_edges
             WHERE edge_type = 'duplicates'
             ORDER BY from_memory_id ASC",
        )?
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    assert_eq!(
        duplicate_edges,
        vec![
            (1031, 1030, "duplicates".to_string()),
            (1032, 1030, "duplicates".to_string())
        ]
    );
    Ok(())
}

#[test]
fn object_ref_parser_requires_explicit_kind_and_dedupes() -> Result<()> {
    let parsed = parse_object_refs(&refs(&["memory:1, workstream:2", "memory:1"]))?;
    assert_eq!(
        parsed,
        vec![
            ObjectRef::memory(1),
            ObjectRef {
                kind: ScopeObjectKind::Workstream,
                id: 2
            }
        ]
    );
    let err = parse_object_refs(&refs(&["1"])).expect_err("bare ids should fail");
    assert!(err.to_string().contains("kind prefix"));
    assert_eq!(
        memory_refs_from_ids(&[3, 3, 4])?,
        vec![ObjectRef::memory(3), ObjectRef::memory(4)]
    );
    Ok(())
}
