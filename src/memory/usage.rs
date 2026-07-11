use std::collections::{BTreeSet, HashMap};

use anyhow::{bail, Context, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};

const CITATION_PREFIX: &str = "Memory citations:";
pub(crate) const STOP_CITATION_SOURCE: &str = "stop_citation";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MemoryUsageReport {
    pub parsed_count: usize,
    pub matched_count: usize,
    pub inserted_count: usize,
    pub duplicate_event: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MemoryUsageFeedbackStats {
    pub total_events: i64,
    pub parsed_events: i64,
    pub matched_events: i64,
    pub inserted_events: i64,
    pub no_citation_events: i64,
    pub unmatched_events: i64,
    pub usage_events: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MemoryCitationFacts {
    line_present: bool,
    ids: Vec<i64>,
}

impl MemoryCitationFacts {
    pub(crate) fn from_text(text: &str) -> Self {
        let mut ids = BTreeSet::new();
        let mut line_present = false;
        for line in text.lines() {
            let Some(rest) = line.trim().strip_prefix(CITATION_PREFIX) else {
                continue;
            };
            line_present = true;
            for token in rest.split_whitespace() {
                let cleaned = token.trim_matches(|ch: char| {
                    ch == ',' || ch == ';' || ch == '.' || ch == ')' || ch == ']' || ch == '}'
                });
                let Some(raw_id) = cleaned.strip_prefix("memory:#") else {
                    continue;
                };
                if let Ok(id) = raw_id.parse::<i64>() {
                    if id > 0 {
                        ids.insert(id);
                    }
                }
            }
        }
        Self {
            line_present,
            ids: ids.into_iter().collect(),
        }
    }

    pub(crate) fn ids(&self) -> &[i64] {
        &self.ids
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if !self.line_present && !self.ids.is_empty() {
            bail!("invalid memory citation facts: ids require a citation line");
        }
        if self.ids.iter().any(|id| *id <= 0) {
            bail!("invalid memory citation facts: ids must be positive");
        }
        if self.ids.windows(2).any(|pair| pair[0] >= pair[1]) {
            bail!("invalid memory citation facts: ids must be unique and sorted");
        }
        Ok(())
    }
}

pub(crate) fn citation_contract_line() -> &'static str {
    "When using injected memories, end with `Memory citations: memory:#<id> ...`; otherwise `Memory citations: none`."
}

#[cfg(test)]
fn parse_memory_citations(text: &str) -> MemoryCitationFacts {
    MemoryCitationFacts::from_text(text)
}

pub(crate) fn record_stop_memory_citations(
    conn: &rusqlite::Connection,
    host: &str,
    project: &str,
    session_id: &str,
    message_hash: &str,
    assistant_message: &str,
) -> Result<MemoryUsageReport> {
    let facts = MemoryCitationFacts::from_text(assistant_message);
    record_stop_memory_citation_facts(conn, host, project, session_id, message_hash, &facts)
}

pub(crate) fn record_stop_memory_citation_facts(
    conn: &rusqlite::Connection,
    host: &str,
    project: &str,
    session_id: &str,
    message_hash: &str,
    facts: &MemoryCitationFacts,
) -> Result<MemoryUsageReport> {
    record_memory_citations(
        conn,
        host,
        project,
        session_id,
        STOP_CITATION_SOURCE,
        message_hash,
        facts,
    )
}

#[allow(clippy::too_many_arguments)]
fn record_memory_citations(
    conn: &rusqlite::Connection,
    host: &str,
    project: &str,
    session_id: &str,
    source: &str,
    message_hash: &str,
    facts: &MemoryCitationFacts,
) -> Result<MemoryUsageReport> {
    facts.validate()?;
    conn.execute_batch("SAVEPOINT remem_memory_usage")
        .context("begin memory usage savepoint")?;
    let result =
        record_memory_citations_inner(conn, host, project, session_id, source, message_hash, facts);
    match result {
        Ok(report) => {
            conn.execute_batch("RELEASE SAVEPOINT remem_memory_usage")
                .context("release memory usage savepoint")?;
            Ok(report)
        }
        Err(error) => {
            let rollback = conn.execute_batch(
                "ROLLBACK TO SAVEPOINT remem_memory_usage;
                 RELEASE SAVEPOINT remem_memory_usage;",
            );
            if let Err(rollback_error) = rollback {
                return Err(error.context(format!(
                    "memory usage rollback also failed: {rollback_error}"
                )));
            }
            Err(error)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn record_memory_citations_inner(
    conn: &rusqlite::Connection,
    host: &str,
    project: &str,
    session_id: &str,
    source: &str,
    message_hash: &str,
    parsed: &MemoryCitationFacts,
) -> Result<MemoryUsageReport> {
    let injected = injected_memory_items(conn, host, project, session_id, &parsed.ids)?;
    let status = citation_status(parsed.ids.len(), injected.len());
    let now = chrono::Utc::now().timestamp();

    let inserted_citation_event = conn.execute(
        "INSERT OR IGNORE INTO memory_citation_events
         (host, project, session_id, source, message_hash, citation_line_present,
          parsed_count, matched_count, inserted_count, status, created_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?10)",
        params![
            host,
            project,
            session_id,
            source,
            message_hash,
            i64::from(parsed.line_present),
            parsed.ids.len() as i64,
            injected.len() as i64,
            status,
            now
        ],
    )?;
    if inserted_citation_event == 0 {
        return Ok(MemoryUsageReport {
            parsed_count: parsed.ids.len(),
            matched_count: injected.len(),
            inserted_count: 0,
            duplicate_event: true,
        });
    }

    let citation_event_id = conn.last_insert_rowid();
    let mut inserted_memory_ids = Vec::new();
    for (memory_id, context_injection_item_id) in injected {
        let inserted_usage_event = conn.execute(
            "INSERT OR IGNORE INTO memory_usage_events
             (citation_event_id, host, project, session_id, source, message_hash, memory_id,
              context_injection_item_id, created_at_epoch)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                citation_event_id,
                host,
                project,
                session_id,
                source,
                message_hash,
                memory_id,
                context_injection_item_id,
                now
            ],
        )?;
        if inserted_usage_event > 0 {
            inserted_memory_ids.push(memory_id);
        }
    }
    if !inserted_memory_ids.is_empty() {
        crate::memory::mark_memories_accessed(conn, &inserted_memory_ids)?;
    }
    conn.execute(
        "UPDATE memory_citation_events SET inserted_count = ?1 WHERE id = ?2",
        params![inserted_memory_ids.len() as i64, citation_event_id],
    )?;

    Ok(MemoryUsageReport {
        parsed_count: parsed.ids.len(),
        matched_count: inserted_memory_ids.len(),
        inserted_count: inserted_memory_ids.len(),
        duplicate_event: false,
    })
}

fn citation_status(parsed_count: usize, matched_count: usize) -> &'static str {
    if parsed_count == 0 {
        "no_citation"
    } else if matched_count == 0 {
        "unmatched"
    } else {
        "matched"
    }
}

fn injected_memory_items(
    conn: &rusqlite::Connection,
    host: &str,
    project: &str,
    session_id: &str,
    cited_ids: &[i64],
) -> Result<HashMap<i64, i64>> {
    if cited_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let placeholders = (4..cited_ids.len() + 4)
        .map(|idx| format!("?{idx}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT memory_id, id
         FROM context_injection_items
         WHERE host = ?1
           AND project = ?2
           AND session_id = ?3
           AND status = 'injected'
           AND memory_id IN ({placeholders})
         ORDER BY injected_at_epoch DESC, id DESC"
    );
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
        Box::new(host.to_string()),
        Box::new(project.to_string()),
        Box::new(session_id.to_string()),
    ];
    params.extend(
        cited_ids
            .iter()
            .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>),
    );
    let refs = crate::db::to_sql_refs(&params);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut injected = HashMap::new();
    for row in rows {
        let (memory_id, item_id) = row?;
        injected.entry(memory_id).or_insert(item_id);
    }
    Ok(injected)
}

pub(crate) fn query_memory_usage_feedback_stats(
    conn: &rusqlite::Connection,
) -> Result<MemoryUsageFeedbackStats> {
    let total_events = count_memory_usage_table_rows(conn, "memory_citation_events")?;
    if total_events == 0 {
        return Ok(MemoryUsageFeedbackStats {
            total_events: 0,
            parsed_events: 0,
            matched_events: 0,
            inserted_events: 0,
            no_citation_events: 0,
            unmatched_events: 0,
            usage_events: 0,
        });
    }

    let (parsed_events, matched_events, inserted_events, no_citation_events, unmatched_events): (
        i64,
        i64,
        i64,
        i64,
        i64,
    ) = conn.query_row(
        "SELECT
             COALESCE(SUM(CASE WHEN citation_line_present > 0 THEN 1 ELSE 0 END), 0),
             COALESCE(SUM(CASE WHEN matched_count > 0 THEN 1 ELSE 0 END), 0),
             COALESCE(SUM(CASE WHEN inserted_count > 0 THEN 1 ELSE 0 END), 0),
             COALESCE(SUM(CASE WHEN status = 'no_citation' THEN 1 ELSE 0 END), 0),
             COALESCE(SUM(CASE WHEN status = 'unmatched' THEN 1 ELSE 0 END), 0)
         FROM memory_citation_events",
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
    let usage_events = count_memory_usage_table_rows(conn, "memory_usage_events")?;
    Ok(MemoryUsageFeedbackStats {
        total_events,
        parsed_events,
        matched_events,
        inserted_events,
        no_citation_events,
        unmatched_events,
        usage_events,
    })
}

fn count_memory_usage_table_rows(conn: &rusqlite::Connection, table: &str) -> Result<i64> {
    let exists = memory_usage_table_exists(conn, table)?;
    if !exists {
        return Ok(0);
    }
    Ok(
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })?,
    )
}

fn memory_usage_table_exists(conn: &rusqlite::Connection, table: &str) -> Result<bool> {
    let mut stmt =
        conn.prepare("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1")?;
    let count: i64 = stmt.query_row([table], |row| row.get(0))?;
    Ok(count > 0)
}

#[cfg(test)]
mod tests;
