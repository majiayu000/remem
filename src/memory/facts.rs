use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactPredicate {
    FixedBy,
    VerifiedBy,
    Supersedes,
    BlockedBy,
    UsesFile,
    UsesCommand,
    AffectsProject,
}

impl FactPredicate {
    pub fn db_value(self) -> &'static str {
        match self {
            Self::FixedBy => "fixed_by",
            Self::VerifiedBy => "verified_by",
            Self::Supersedes => "supersedes",
            Self::BlockedBy => "blocked_by",
            Self::UsesFile => "uses_file",
            Self::UsesCommand => "uses_command",
            Self::AffectsProject => "affects_project",
        }
    }

    fn parse_db(raw: &str) -> Option<Self> {
        match raw {
            "fixed_by" => Some(Self::FixedBy),
            "verified_by" => Some(Self::VerifiedBy),
            "supersedes" => Some(Self::Supersedes),
            "blocked_by" => Some(Self::BlockedBy),
            "uses_file" => Some(Self::UsesFile),
            "uses_command" => Some(Self::UsesCommand),
            "affects_project" => Some(Self::AffectsProject),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TemporalFactInput<'a> {
    pub project: &'a str,
    pub subject: &'a str,
    pub predicate: FactPredicate,
    pub object: &'a str,
    pub valid_from_epoch: Option<i64>,
    pub valid_to_epoch: Option<i64>,
    pub learned_at_epoch: Option<i64>,
    pub source_memory_id: Option<i64>,
    pub source_observation_id: Option<i64>,
    pub source_event_ids: &'a [i64],
    pub confidence: f64,
    pub supersedes_fact_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TemporalFact {
    pub id: i64,
    pub project: String,
    pub subject: String,
    pub predicate: FactPredicate,
    pub object: String,
    pub valid_from_epoch: Option<i64>,
    pub valid_to_epoch: Option<i64>,
    pub learned_at_epoch: i64,
    pub source_memory_id: Option<i64>,
    pub source_observation_id: Option<i64>,
    pub source_event_ids: Vec<i64>,
    pub confidence: f64,
    pub supersedes_fact_id: Option<i64>,
    pub status: String,
}

pub(crate) fn invalidated_at_epoch_available(conn: &Connection) -> Result<bool> {
    let exists: i64 = conn.query_row(
        "SELECT EXISTS (
             SELECT 1 FROM pragma_table_info('memory_facts')
             WHERE name = 'invalidated_at_epoch'
         )",
        [],
        |row| row.get(0),
    )?;
    Ok(exists != 0)
}

pub(crate) fn current_fact_filter_sql(alias: &str, has_invalidated_at_epoch: bool) -> String {
    let alias = alias.trim();
    let prefix = if alias.is_empty() {
        String::new()
    } else {
        format!("{alias}.")
    };
    if has_invalidated_at_epoch {
        format!("{prefix}status = 'active' AND {prefix}invalidated_at_epoch IS NULL")
    } else {
        format!("{prefix}status = 'active'")
    }
}

pub fn insert_temporal_fact(conn: &mut Connection, input: &TemporalFactInput<'_>) -> Result<i64> {
    validate_input(input)?;
    let now = chrono::Utc::now().timestamp();

    let tx = conn.transaction()?;
    let id = insert_temporal_fact_in_current_tx(&tx, input, now)?;
    tx.commit()?;
    Ok(id)
}

pub(crate) fn insert_temporal_fact_in_current_tx(
    conn: &Connection,
    input: &TemporalFactInput<'_>,
    now: i64,
) -> Result<i64> {
    validate_input(input)?;
    let learned_at = input.learned_at_epoch.unwrap_or(now);
    let superseded_at = input.valid_from_epoch.unwrap_or(learned_at);
    let source_event_ids = serde_json::to_string(input.source_event_ids)?;

    if let Some(old_id) = input.supersedes_fact_id {
        let old_fact: Option<(String, Option<i64>)> = conn
            .query_row(
                "SELECT project, valid_from_epoch FROM memory_facts WHERE id = ?1",
                [old_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        match old_fact {
            Some((project, old_valid_from)) if project == input.project => {
                if let Some(old_from) = old_valid_from {
                    if superseded_at < old_from {
                        bail!(
                            "cannot supersede fact {old_id}: cutoff {superseded_at} is before existing valid_from_epoch {}",
                            old_from
                        );
                    }
                }
            }
            Some((project, _)) => bail!(
                "cannot supersede fact {old_id} from project '{project}' with project '{}'",
                input.project
            ),
            None => bail!("cannot supersede missing memory fact {old_id}"),
        }
        conn.execute(
            "UPDATE memory_facts
             SET status = 'stale',
                 valid_to_epoch = CASE
                     WHEN valid_to_epoch IS NULL OR valid_to_epoch > ?1 THEN ?1
                     ELSE valid_to_epoch
                 END,
                 invalidated_at_epoch = COALESCE(invalidated_at_epoch, ?2),
                 updated_at_epoch = ?2
             WHERE id = ?3",
            params![superseded_at, now, old_id],
        )?;
    }

    conn.execute(
        "INSERT INTO memory_facts
         (project, subject, predicate, object, valid_from_epoch, valid_to_epoch,
          learned_at_epoch, source_memory_id, source_observation_id, source_event_ids,
          confidence, supersedes_fact_id, status, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'active', ?13, ?13)",
        params![
            input.project,
            input.subject,
            input.predicate.db_value(),
            input.object,
            input.valid_from_epoch,
            input.valid_to_epoch,
            learned_at,
            input.source_memory_id,
            input.source_observation_id,
            source_event_ids,
            input.confidence,
            input.supersedes_fact_id,
            now
        ],
    )?;
    let id = conn.last_insert_rowid();
    Ok(id)
}

pub fn list_current_facts(
    conn: &Connection,
    project: &str,
    subject: Option<&str>,
    predicate: Option<FactPredicate>,
) -> Result<Vec<TemporalFact>> {
    let now = chrono::Utc::now().timestamp();
    query_facts(conn, project, subject, predicate, Some(now), true)
}

pub fn list_facts_as_of(
    conn: &Connection,
    project: &str,
    as_of_epoch: i64,
    subject: Option<&str>,
    predicate: Option<FactPredicate>,
) -> Result<Vec<TemporalFact>> {
    query_facts(conn, project, subject, predicate, Some(as_of_epoch), false)
}

fn query_facts(
    conn: &Connection,
    project: &str,
    subject: Option<&str>,
    predicate: Option<FactPredicate>,
    as_of_epoch: Option<i64>,
    active_only: bool,
) -> Result<Vec<TemporalFact>> {
    let has_invalidated_at_epoch = invalidated_at_epoch_available(conn)?;
    let mut conditions = vec!["project = ?1".to_string()];
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(project.to_string())];
    let mut idx = 2;
    if let Some(subject) = subject {
        conditions.push(format!("subject = ?{idx}"));
        params.push(Box::new(subject.to_string()));
        idx += 1;
    }
    if let Some(predicate) = predicate {
        conditions.push(format!("predicate = ?{idx}"));
        params.push(Box::new(predicate.db_value().to_string()));
        idx += 1;
    }
    if let Some(as_of_epoch) = as_of_epoch {
        conditions.push(format!(
            "(valid_from_epoch IS NULL OR valid_from_epoch <= ?{idx})"
        ));
        if has_invalidated_at_epoch {
            conditions.push(format!(
                "(valid_to_epoch IS NULL OR valid_to_epoch > ?{idx} \
                  OR (invalidated_at_epoch IS NOT NULL AND invalidated_at_epoch > ?{idx}))"
            ));
        } else {
            conditions.push(format!(
                "(valid_to_epoch IS NULL OR valid_to_epoch > ?{idx})"
            ));
        }
        conditions.push(format!("learned_at_epoch <= ?{idx}"));
        if has_invalidated_at_epoch {
            conditions.push(format!(
                "(invalidated_at_epoch IS NULL OR invalidated_at_epoch > ?{idx})"
            ));
        }
        params.push(Box::new(as_of_epoch));
    }
    if active_only {
        conditions.push(current_fact_filter_sql("", has_invalidated_at_epoch));
    }

    let sql = format!(
        "SELECT id, project, subject, predicate, object, valid_from_epoch,
                valid_to_epoch, learned_at_epoch, source_memory_id,
                source_observation_id, source_event_ids, confidence,
                supersedes_fact_id, status
         FROM memory_facts
         WHERE {}
         ORDER BY learned_at_epoch DESC, id DESC",
        conditions.join(" AND ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&params);
    let rows = stmt.query_map(refs.as_slice(), map_fact_row)?;
    crate::db::query::collect_rows(rows)
}

fn validate_input(input: &TemporalFactInput<'_>) -> Result<()> {
    if input.project.trim().is_empty() {
        bail!("memory fact project is required");
    }
    if input.subject.trim().is_empty() {
        bail!("memory fact subject is required");
    }
    if input.object.trim().is_empty() {
        bail!("memory fact object is required");
    }
    if !(0.0..=1.0).contains(&input.confidence) {
        bail!("memory fact confidence out of range");
    }
    if let (Some(valid_from), Some(valid_to)) = (input.valid_from_epoch, input.valid_to_epoch) {
        if valid_to < valid_from {
            bail!("memory fact valid_to_epoch cannot be before valid_from_epoch");
        }
    }
    Ok(())
}

fn map_fact_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TemporalFact> {
    let predicate_raw: String = row.get(3)?;
    let source_event_json: String = row.get(10)?;
    let source_event_ids = serde_json::from_str(&source_event_json).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(10, rusqlite::types::Type::Text, Box::new(err))
    })?;
    let predicate = FactPredicate::parse_db(&predicate_raw).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown memory fact predicate: {predicate_raw}"),
            )),
        )
    })?;
    Ok(TemporalFact {
        id: row.get(0)?,
        project: row.get(1)?,
        subject: row.get(2)?,
        predicate,
        object: row.get(4)?,
        valid_from_epoch: row.get(5)?,
        valid_to_epoch: row.get(6)?,
        learned_at_epoch: row.get(7)?,
        source_memory_id: row.get(8)?,
        source_observation_id: row.get(9)?,
        source_event_ids,
        confidence: row.get(11)?,
        supersedes_fact_id: row.get(12)?,
        status: row.get(13)?,
    })
}

#[cfg(test)]
mod tests;
