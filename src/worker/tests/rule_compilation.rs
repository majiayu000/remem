use anyhow::{Context, Result};
use rusqlite::params;

use crate::db::{self, test_support::ScopedTestDataDir};
use crate::rules::{artifact_path_for_project, load_artifact_fail_open, ArtifactLoad};

use super::run;

const PROJECT: &str = "/tmp/remem";

#[tokio::test]
async fn worker_sweep_builds_existing_rules_and_removes_deleted_sources() -> Result<()> {
    let _data_dir = ScopedTestDataDir::new("worker-rule-compilation-sweep");
    crate::runtime_config::init_config()?;
    crate::runtime_config::set_config_value("rule_compilation.enabled", "true")?;

    let conn = db::open_db()?;
    conn.execute(
        "INSERT INTO memory_candidates
         (id, scope, memory_type, topic_key, text, evidence_event_ids,
          confidence, risk_class, review_status, created_at_epoch, updated_at_epoch)
         VALUES (1, 'project', 'preference', 'package-manager',
                 'Use bun, not npm', '[1]', 0.95, 'low', 'auto_promoted', 1, 1)",
        [],
    )?;
    conn.execute(
        "INSERT INTO memories
         (id, project, title, content, memory_type, created_at_epoch,
          updated_at_epoch, status, scope, owner_scope, owner_key, source_candidate_id)
         VALUES (1, ?1, 'pref', 'Use bun, not npm', 'preference', 1,
                 1, 'active', 'project', 'repo', ?1, 1)",
        params![PROJECT],
    )?;
    conn.execute(
        "INSERT INTO memory_preference_reinforcements
         (memory_id, reinforcement_count, last_reinforced_at_epoch,
          created_at_epoch, updated_at_epoch, machine_checkable, risk_class)
         VALUES (1, 3, 1, 1, 1, 1, 'low')",
        [],
    )?;
    drop(conn);

    run(true, 10).await?;

    let artifact_path = artifact_path_for_project(&db::absolute_data_dir()?, PROJECT);
    let loaded = load_artifact_fail_open(&artifact_path);
    let ArtifactLoad::Loaded(artifact) = loaded else {
        anyhow::bail!("worker sweep should create the artifact, got {loaded:?}");
    };
    assert_eq!(artifact.rules.len(), 1);

    let conn = db::open_db()?;
    conn.execute("DELETE FROM memories WHERE id = 1", [])?;
    drop(conn);
    run(true, 10).await?;

    let loaded = load_artifact_fail_open(&artifact_path);
    let ArtifactLoad::Loaded(artifact) = loaded else {
        anyhow::bail!("worker sweep should rebuild the artifact, got {loaded:?}");
    };
    assert!(artifact.rules.is_empty());

    let conn = db::open_db()?;
    let successful_sweeps: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM preference_rule_diagnostics
             WHERE project = ?1 AND event_kind = 'compile' AND status = 'ok'",
            params![PROJECT],
            |row| row.get(0),
        )
        .context("count successful worker rule sweeps")?;
    assert_eq!(successful_sweeps, 2);
    Ok(())
}
