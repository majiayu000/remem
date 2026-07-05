use anyhow::Result;
use rusqlite::Connection;

use super::run_migrations;

#[test]
fn preference_rule_state_migration_creates_rows_and_indexes() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    run_migrations(&conn)?;

    for name in [
        "memory_preference_reinforcements",
        "preference_rule_overrides",
        "preference_rule_diagnostics",
        "idx_memory_preference_reinforcements_rank",
        "idx_preference_rule_overrides_project",
        "idx_preference_rule_overrides_source",
        "idx_preference_rule_diagnostics_project_event",
        "idx_preference_rule_diagnostics_rule",
    ] {
        let exists: bool = conn.query_row(
            "SELECT 1 FROM sqlite_master WHERE name = ?1",
            [name],
            |_| Ok(true),
        )?;
        assert!(
            exists,
            "{name} should exist after preference rule migration"
        );
    }

    conn.execute(
        "INSERT INTO memories
         (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status)
         VALUES (1, '/repo', 'Preference', 'Use bun instead of npm.', 'preference', 10, 10, 'active')",
        [],
    )?;
    conn.execute(
        "INSERT INTO memory_preference_reinforcements
         (memory_id, reinforcement_count, last_reinforced_at_epoch,
          created_at_epoch, updated_at_epoch)
         VALUES (1, 3, 20, 10, 20)",
        [],
    )?;
    conn.execute(
        "INSERT INTO preference_rule_overrides
         (project, rule_id, source_memory_id, disabled, action_override, updated_at_epoch)
         VALUES ('/repo', 'pref-1-1', 1, 0, 'warn', 30)",
        [],
    )?;
    conn.execute(
        "INSERT INTO preference_rule_diagnostics
         (project, event_kind, status, rule_id, rule_count, occurred_at_epoch)
         VALUES ('/repo', 'compile', 'ok', 'pref-1-1', 1, 40)",
        [],
    )?;
    Ok(())
}

#[test]
fn preference_rule_state_schema_validates_actions_and_counts() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    run_migrations(&conn)?;

    let err = conn
        .execute(
            "INSERT INTO memory_preference_reinforcements
             (memory_id, reinforcement_count, last_reinforced_at_epoch,
              created_at_epoch, updated_at_epoch)
             VALUES (99, 0, 20, 10, 20)",
            [],
        )
        .expect_err("reinforcement count must be positive");
    assert!(err.to_string().contains("CHECK"), "{err}");

    let err = conn
        .execute(
            "INSERT INTO preference_rule_overrides
             (project, rule_id, action_override, updated_at_epoch)
             VALUES ('/repo', 'pref-1-1', 'audit', 30)",
            [],
        )
        .expect_err("action override must stay closed");
    assert!(err.to_string().contains("CHECK"), "{err}");

    let err = conn
        .execute(
            "INSERT INTO preference_rule_diagnostics
             (project, event_kind, status, rule_count, occurred_at_epoch)
             VALUES ('/repo', 'compile', 'ok', -1, 40)",
            [],
        )
        .expect_err("rule count must be non-negative");
    assert!(err.to_string().contains("CHECK"), "{err}");
    Ok(())
}
