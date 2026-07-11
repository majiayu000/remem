//! Rule-compilation enqueue helpers shared by preference lifecycle mutations.
//!
//! These helpers only schedule worker jobs. They never compile or write the
//! derived artifact in a caller process.

use anyhow::{Context, Result};
use rusqlite::{params, params_from_iter, Connection};

use crate::memory::suppression::SuppressionTarget;

#[derive(Debug)]
struct AffectedPreference {
    project: String,
    global: bool,
}

const SUPPRESSION_TARGET_MATCH_SQL: &str = "(
       (?1 = 'memory' AND ?2 IS NOT NULL AND m.id = ?2)
    OR (?1 = 'topic_key' AND ?3 IS NOT NULL AND m.topic_key = ?3)
    OR (?1 = 'entity' AND ?3 IS NOT NULL AND EXISTS (
           SELECT 1
           FROM memory_entities ms_me
           JOIN entities ms_e ON ms_e.id = ms_me.entity_id
           WHERE ms_me.memory_id = m.id
             AND lower(ms_e.canonical_name) = lower(?3)
       ))
    OR (?1 = 'pattern' AND ?3 IS NOT NULL AND (
           instr(lower(m.title), lower(?3)) > 0
        OR instr(lower(m.content), lower(?3)) > 0
       ))
)";

pub(crate) fn enqueue_for_memory_ids(conn: &Connection, memory_ids: &[i64]) -> Result<()> {
    if memory_ids.is_empty() {
        return Ok(());
    }
    let mut unique = memory_ids
        .iter()
        .copied()
        .filter(|id| *id > 0)
        .collect::<Vec<_>>();
    unique.sort_unstable();
    unique.dedup();
    if unique.is_empty() {
        return Ok(());
    }

    let placeholders = std::iter::repeat_n("?", unique.len())
        .collect::<Vec<_>>()
        .join(", ");
    let preference_check_sql = format!(
        "SELECT EXISTS(
             SELECT 1 FROM memories
             WHERE memory_type = 'preference' AND id IN ({placeholders})
         )"
    );
    let has_preferences: bool = conn.query_row(
        &preference_check_sql,
        params_from_iter(unique.iter()),
        |row| row.get(0),
    )?;
    if !has_preferences {
        return Ok(());
    }
    let sql = format!(
        "SELECT m.project, COALESCE(m.scope, 'project') = 'global'
         FROM memories m
         JOIN memory_preference_reinforcements r ON r.memory_id = m.id
         WHERE m.memory_type = 'preference' AND m.id IN ({placeholders})"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(unique.iter()), |row| {
        Ok(AffectedPreference {
            project: row.get(0)?,
            global: row.get(1)?,
        })
    })?;
    let affected = crate::db::query::collect_rows(rows)?;
    enqueue_affected(conn, affected)
}

pub(crate) fn enqueue_for_suppression_targets(
    conn: &Connection,
    targets: &[SuppressionTarget],
) -> Result<()> {
    let mut affected = Vec::new();
    for target in targets {
        let exists_sql = format!(
            "SELECT EXISTS(
                 SELECT 1 FROM memories m
                 WHERE m.memory_type = 'preference'
                   AND m.status = 'active'
                   AND {SUPPRESSION_TARGET_MATCH_SQL}
             )"
        );
        let matches_preference: bool = conn.query_row(
            &exists_sql,
            params![target.kind, target.id, target.value],
            |row| row.get(0),
        )?;
        if !matches_preference {
            continue;
        }
        let affected_sql = format!(
            "SELECT m.project, COALESCE(m.scope, 'project') = 'global'
             FROM memories m
             JOIN memory_preference_reinforcements r ON r.memory_id = m.id
             WHERE m.memory_type = 'preference'
               AND m.status = 'active'
               AND {SUPPRESSION_TARGET_MATCH_SQL}"
        );
        let mut stmt = conn.prepare(&affected_sql)?;
        let rows = stmt.query_map(params![target.kind, target.id, target.value], |row| {
            Ok(AffectedPreference {
                project: row.get(0)?,
                global: row.get(1)?,
            })
        })?;
        affected.extend(crate::db::query::collect_rows(rows)?);
    }
    enqueue_affected(conn, affected)
}

fn enqueue_affected(conn: &Connection, affected: Vec<AffectedPreference>) -> Result<()> {
    if affected.is_empty() {
        return Ok(());
    }
    let has_global = affected.iter().any(|preference| preference.global);
    let mut projects = affected
        .into_iter()
        .map(|preference| preference.project)
        .collect::<std::collections::BTreeSet<_>>();
    if has_global {
        projects.extend(known_compilation_projects(conn)?);
    }
    enqueue_projects(conn, projects)
}

fn known_compilation_projects(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT project FROM preference_rule_diagnostics
         UNION
         SELECT project FROM preference_rule_overrides
         UNION
         SELECT project FROM jobs WHERE job_type = 'compile_rules'
         UNION
         SELECT m.project
         FROM memories m
         JOIN memory_preference_reinforcements r ON r.memory_id = m.id",
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    crate::db::query::collect_rows(rows)
}

fn enqueue_projects(conn: &Connection, projects: impl IntoIterator<Item = String>) -> Result<()> {
    let projects = projects
        .into_iter()
        .filter(|project| !project.trim().is_empty())
        .collect::<std::collections::BTreeSet<_>>();
    if projects.is_empty() {
        return Ok(());
    }
    let config = crate::runtime_config::rule_compilation_config()
        .context("read rule compilation config before enqueue")?;
    if !config.enabled {
        return Ok(());
    }
    for project in projects {
        crate::db::enqueue_job(
            conn,
            "worker",
            crate::db::JobType::CompileRules,
            &project,
            None,
            "{}",
            100,
        )
        .with_context(|| format!("enqueue rule compilation for {project}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::ScopedTestDataDir;

    #[test]
    fn enabled_enqueue_failure_is_propagated() -> Result<()> {
        let _dir = ScopedTestDataDir::new("preference-enqueue-error");
        crate::runtime_config::init_config()?;
        crate::runtime_config::set_config_value("rule_compilation.enabled", "true")?;
        let conn = Connection::open_in_memory()?;

        let error = enqueue_projects(&conn, std::iter::once("/repo".to_string()))
            .expect_err("missing jobs table must not be swallowed when compilation is enabled");
        assert!(
            error.to_string().contains("enqueue rule compilation"),
            "{error:#}"
        );
        Ok(())
    }

    #[test]
    fn malformed_enqueue_config_is_propagated() -> Result<()> {
        let _dir = ScopedTestDataDir::new("preference-enqueue-config-error");
        std::fs::create_dir_all(crate::db::data_dir())?;
        std::fs::write(
            crate::runtime_config::config_path(),
            "[rule_compilation]\nenabled = 'yes'\n",
        )?;
        let conn = Connection::open_in_memory()?;

        let error = enqueue_projects(&conn, std::iter::once("/repo".to_string()))
            .expect_err("malformed compilation config must not be swallowed");
        assert!(
            error.to_string().contains("read rule compilation config"),
            "{error:#}"
        );
        Ok(())
    }

    #[test]
    fn global_preference_enqueues_each_known_compilation_project() -> Result<()> {
        let _dir = ScopedTestDataDir::new("global-preference-enqueue");
        crate::runtime_config::init_config()?;
        crate::runtime_config::set_config_value("rule_compilation.enabled", "true")?;
        let conn = crate::db::open_db()?;
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch,
              status, scope)
             VALUES (1, '/source', 'Preference', 'Use bun, not npm', 'preference',
                     1, 1, 'active', 'global')",
            [],
        )?;
        conn.execute(
            "INSERT INTO memory_preference_reinforcements
             (memory_id, reinforcement_count, last_reinforced_at_epoch,
              created_at_epoch, updated_at_epoch, machine_checkable)
             VALUES (1, 3, 1, 1, 1, 1)",
            [],
        )?;
        conn.execute(
            "INSERT INTO preference_rule_diagnostics
             (project, event_kind, status, rule_count, occurred_at_epoch)
             VALUES ('/consumer', 'compile', 'ok', 0, 1)",
            [],
        )?;

        enqueue_for_memory_ids(&conn, &[1])?;

        let mut stmt = conn.prepare(
            "SELECT project FROM jobs
             WHERE job_type = 'compile_rules' AND state = 'pending'
             ORDER BY project",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        assert_eq!(
            crate::db::query::collect_rows(rows)?,
            vec!["/consumer".to_string(), "/source".to_string()]
        );
        Ok(())
    }
}
