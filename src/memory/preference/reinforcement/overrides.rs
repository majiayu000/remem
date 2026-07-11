//! Preference-rule override reconciliation across memory replacements and reroutes.

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection};

#[derive(Debug)]
struct StoredOverride {
    id: i64,
    project: String,
    rule_id: String,
    disabled: i64,
    action_override: Option<String>,
    reason: Option<String>,
    updated_by: String,
    updated_at_epoch: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PredicateIdentity {
    kind: &'static str,
    value: String,
}

pub(super) fn transfer_rule_overrides(
    conn: &Connection,
    old_memory_id: i64,
    new_memory_id: i64,
    old_content: &str,
    new_content: &str,
) -> Result<()> {
    let old_identities = predicate_identities(old_content);
    let new_identities = predicate_identities(new_content);
    if old_identities.is_empty() || new_identities.is_empty() {
        return remove_rule_overrides(conn, old_memory_id);
    }

    let old_prefix = format!("pref-{old_memory_id}-");
    let mut stmt = conn.prepare(
        "SELECT id, project, rule_id, disabled, action_override, reason,
                updated_by, updated_at_epoch
         FROM preference_rule_overrides
         WHERE source_memory_id = ?1 OR rule_id LIKE ?2
         ORDER BY updated_at_epoch ASC, id ASC",
    )?;
    let rows = stmt.query_map(params![old_memory_id, format!("{old_prefix}%")], |row| {
        Ok(StoredOverride {
            id: row.get(0)?,
            project: row.get(1)?,
            rule_id: row.get(2)?,
            disabled: row.get(3)?,
            action_override: row.get(4)?,
            reason: row.get(5)?,
            updated_by: row.get(6)?,
            updated_at_epoch: row.get(7)?,
        })
    })?;
    let overrides = crate::db::query::collect_rows(rows)?;
    drop(stmt);

    let mut mapped = Vec::with_capacity(overrides.len());
    for stored in overrides {
        let old_index = generated_rule_index(&stored.rule_id, &old_prefix)?;
        let new_rule_id = old_identities
            .get(old_index)
            .and_then(|identity| {
                new_identities
                    .iter()
                    .position(|candidate| candidate == identity)
            })
            .map(|index| format!("pref-{new_memory_id}-{}", index + 1));
        mapped.push((stored, new_rule_id));
    }

    for (stored, _) in &mapped {
        conn.execute(
            "DELETE FROM preference_rule_overrides WHERE id = ?1",
            [stored.id],
        )?;
    }
    for (stored, new_rule_id) in mapped {
        let Some(new_rule_id) = new_rule_id else {
            continue;
        };
        upsert_override(conn, &stored, &stored.project, &new_rule_id, new_memory_id)?;
    }
    Ok(())
}

pub(crate) fn reconcile_preference_project_change(
    conn: &Connection,
    memory_id: i64,
    previous_project: &str,
    new_project: &str,
) -> Result<()> {
    if previous_project == new_project {
        return Ok(());
    }
    if new_project.trim().is_empty() {
        bail!("preference authority project must not be empty");
    }

    let prefix = format!("pref-{memory_id}-%");
    let mut stmt = conn.prepare(
        "SELECT id, project, rule_id, disabled, action_override, reason,
                updated_by, updated_at_epoch
         FROM preference_rule_overrides
         WHERE project = ?1 AND (source_memory_id = ?2 OR rule_id LIKE ?3)
         ORDER BY updated_at_epoch ASC, id ASC",
    )?;
    let rows = stmt.query_map(params![previous_project, memory_id, prefix], |row| {
        Ok(StoredOverride {
            id: row.get(0)?,
            project: row.get(1)?,
            rule_id: row.get(2)?,
            disabled: row.get(3)?,
            action_override: row.get(4)?,
            reason: row.get(5)?,
            updated_by: row.get(6)?,
            updated_at_epoch: row.get(7)?,
        })
    })?;
    let overrides = crate::db::query::collect_rows(rows)?;
    drop(stmt);

    for stored in &overrides {
        conn.execute(
            "DELETE FROM preference_rule_overrides WHERE id = ?1",
            [stored.id],
        )?;
    }
    for stored in overrides {
        upsert_override(conn, &stored, new_project, &stored.rule_id, memory_id)?;
    }
    Ok(())
}

pub(super) fn remove_rule_overrides(conn: &Connection, memory_id: i64) -> Result<()> {
    let prefix = format!("pref-{memory_id}-%");
    conn.execute(
        "DELETE FROM preference_rule_overrides
         WHERE source_memory_id = ?1 OR rule_id LIKE ?2",
        params![memory_id, prefix],
    )?;
    Ok(())
}

fn predicate_identities(content: &str) -> Vec<PredicateIdentity> {
    crate::rules::classify_preference_predicates(content)
        .into_iter()
        .map(|classification| match classification.predicate {
            crate::rules::PreferencePredicate::CommandRegex { pattern, .. } => PredicateIdentity {
                kind: "command_regex",
                value: pattern.trim().to_ascii_lowercase(),
            },
            crate::rules::PreferencePredicate::CommitTrailerForbidden { trailer, .. } => {
                PredicateIdentity {
                    kind: "commit_trailer_forbidden",
                    value: trailer.trim().to_ascii_lowercase(),
                }
            }
        })
        .collect()
}

fn generated_rule_index(rule_id: &str, prefix: &str) -> Result<usize> {
    let suffix = rule_id.strip_prefix(prefix).with_context(|| {
        format!("override {rule_id} has no generated preference rule prefix {prefix}")
    })?;
    if suffix.is_empty() || !suffix.chars().all(|ch| ch.is_ascii_digit()) {
        bail!("override {rule_id} has an invalid generated rule suffix");
    }
    let ordinal = suffix
        .parse::<usize>()
        .with_context(|| format!("parse generated rule ordinal for override {rule_id}"))?;
    ordinal
        .checked_sub(1)
        .with_context(|| format!("override {rule_id} uses invalid zero rule ordinal"))
}

fn upsert_override(
    conn: &Connection,
    stored: &StoredOverride,
    project: &str,
    rule_id: &str,
    source_memory_id: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO preference_rule_overrides
         (project, rule_id, source_memory_id, disabled, action_override, reason,
          updated_by, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(project, rule_id) DO UPDATE SET
           source_memory_id = excluded.source_memory_id,
           disabled = excluded.disabled,
           action_override = excluded.action_override,
           reason = excluded.reason,
           updated_by = excluded.updated_by,
           updated_at_epoch = excluded.updated_at_epoch
         WHERE excluded.updated_at_epoch >= preference_rule_overrides.updated_at_epoch",
        params![
            project,
            rule_id,
            source_memory_id,
            stored.disabled,
            stored.action_override.as_deref(),
            stored.reason.as_deref(),
            stored.updated_by.as_str(),
            stored.updated_at_epoch,
        ],
    )?;
    Ok(())
}
