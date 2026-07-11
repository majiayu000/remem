use anyhow::Result;
use rusqlite::{params, Connection};

use crate::memory;

use super::{query_global_preferences, query_project_preferences};

pub fn list_preferences(conn: &Connection, project: &str) -> Result<()> {
    let project_prefs = query_project_preferences(conn, project, 50)?;
    let global_prefs = query_global_preferences(conn, 10).unwrap_or_default();

    if project_prefs.is_empty() && global_prefs.is_empty() {
        println!("No preferences found.");
        return Ok(());
    }

    if !project_prefs.is_empty() {
        println!("Project preferences ({}):", project);
        for pref in &project_prefs {
            let text_preview: String = pref.text.chars().take(80).collect();
            println!("  [{}] {}", pref.id, text_preview);
        }
    }

    if !global_prefs.is_empty() {
        println!("\nGlobal preferences:");
        for pref in &global_prefs {
            let text_preview: String = pref.text.chars().take(80).collect();
            println!("  [{}] {} (from: {})", pref.id, text_preview, pref.project);
        }
    }

    Ok(())
}

pub fn add_preference(conn: &Connection, project: &str, text: &str, global: bool) -> Result<i64> {
    let title = format!("Preference: {}", &text[..text.len().min(60)]);
    let topic_key = format!(
        "manual-preference-{}",
        crate::memory::slugify_for_topic(text, 50)
    );
    let scope = if global { "global" } else { "project" };
    let tx = conn.unchecked_transaction()?;
    let mut stmt = tx.prepare(
        "SELECT m.id, m.content,
                EXISTS(
                    SELECT 1 FROM memory_preference_reinforcements r WHERE r.memory_id = m.id
                )
         FROM memories m
         WHERE m.memory_type = 'preference'",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, bool>(2)?,
        ))
    })?;
    let previous_preferences = crate::db::query::collect_rows(rows)?;
    drop(stmt);
    let id = memory::insert_memory_full(
        &tx,
        None,
        project,
        Some(&topic_key),
        &title,
        text,
        "preference",
        None,
        None,
        scope,
        None,
    )?;
    if let Some((_, previous_text, _)) = previous_preferences
        .iter()
        .find(|(memory_id, _, had_rule_state)| *memory_id == id && *had_rule_state)
    {
        crate::memory::preference::compilation::enqueue_for_memory_ids(&tx, &[id])?;
        crate::memory::preference::reinforcement::reconcile_in_place_preference_update(
            &tx,
            id,
            previous_text,
            text,
        )?;
    }
    tx.commit()?;
    Ok(id)
}

pub fn remove_preference(conn: &Connection, id: i64) -> Result<bool> {
    let tx = conn.unchecked_transaction()?;
    crate::memory::preference::compilation::enqueue_for_memory_ids(&tx, &[id])?;
    let count = tx.execute(
        "UPDATE memories SET status = 'archived' WHERE id = ?1 AND memory_type = 'preference'",
        params![id],
    )?;
    tx.commit()?;
    Ok(count > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::ScopedTestDataDir;

    #[test]
    fn cli_add_refinement_reconciles_candidate_rule_state_and_enqueues_compile() -> Result<()> {
        let _dir = ScopedTestDataDir::new("preference-cli-add-rule-reconciliation");
        crate::runtime_config::init_config()?;
        crate::runtime_config::set_config_value("rule_compilation.enabled", "true")?;
        let conn = crate::db::open_db()?;
        let original = "Do not add AI-generated-by or Co-authored-by trailers to commits";
        let refinement = "Do not add Co-authored-by trailers to commits";
        let id = add_preference(&conn, "/repo", original, false)?;
        conn.execute(
            "INSERT INTO memory_candidates
             (id, scope, memory_type, topic_key, text, evidence_event_ids,
              confidence, risk_class, review_status, created_at_epoch, updated_at_epoch,
              source_trust_class)
             VALUES (900, 'project', 'preference', 'commit-trailers', ?1, '[1,2,3]',
                     0.95, 'low', 'approved', 1, 3, 'user_prompt')",
            [original],
        )?;
        conn.execute(
            "UPDATE memories
             SET source_candidate_id = 900, source_trust_class = 'user_prompt'
             WHERE id = ?1",
            [id],
        )?;
        conn.execute(
            "INSERT INTO memory_preference_reinforcements
             (memory_id, reinforcement_count, source_evidence,
              last_reinforced_at_epoch, created_at_epoch, updated_at_epoch,
              machine_checkable, risk_class)
             VALUES (?1, 3, '[1,2,3]', 3, 1, 3, 1, 'low')",
            [id],
        )?;
        conn.execute(
            "INSERT INTO preference_rule_overrides
             (project, rule_id, source_memory_id, disabled, action_override,
              updated_by, updated_at_epoch)
             VALUES ('/repo', ?1, ?2, 1, 'block', 'user', 4)",
            params![format!("pref-{id}-1"), id],
        )?;

        let refined_id = add_preference(&conn, "/repo", refinement, false)?;

        assert_eq!(
            refined_id, id,
            "CLI refinement should reuse the canonical row"
        );
        let state: (String, Option<i64>) = conn.query_row(
            "SELECT content, source_candidate_id FROM memories WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(state, (refinement.to_string(), None));
        let reinforcement_rows: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_preference_reinforcements WHERE memory_id = ?1",
            [id],
            |row| row.get(0),
        )?;
        assert_eq!(reinforcement_rows, 0);
        let override_rows: i64 = conn.query_row(
            "SELECT COUNT(*) FROM preference_rule_overrides WHERE source_memory_id = ?1",
            [id],
            |row| row.get(0),
        )?;
        assert_eq!(override_rows, 0);
        let pending: i64 = conn.query_row(
            "SELECT COUNT(*) FROM jobs
             WHERE job_type = 'compile_rules' AND project = '/repo' AND state = 'pending'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(pending, 1);
        Ok(())
    }
}
