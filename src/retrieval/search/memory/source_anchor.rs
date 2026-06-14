use std::collections::HashMap;

use anyhow::Result;
use rusqlite::Connection;

use crate::memory::{self, Memory};

const STALE_SCORE_FACTOR: f64 = 0.25;

pub(super) fn apply_score_demotions(
    conn: &Connection,
    fused: &[(i64, f64)],
    ordered: Vec<Memory>,
) -> Result<(Vec<Memory>, Vec<(i64, f64)>)> {
    if fused.is_empty() || ordered.is_empty() {
        return Ok((ordered, fused.to_vec()));
    }
    let now_epoch = chrono::Utc::now().timestamp();
    let labels = memory::memory_staleness_labels_for_memories(conn, &ordered, now_epoch)?;
    let original_rank = fused
        .iter()
        .enumerate()
        .map(|(rank, (id, _))| (*id, rank))
        .collect::<HashMap<_, _>>();
    let mut demoted = fused
        .iter()
        .map(|(id, score)| {
            let score = if labels
                .get(id)
                .is_some_and(|label| label.source_anchor == "verify-before-trust")
            {
                score * STALE_SCORE_FACTOR
            } else {
                *score
            };
            (*id, score)
        })
        .collect::<Vec<_>>();
    demoted.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                original_rank
                    .get(&left.0)
                    .copied()
                    .unwrap_or(usize::MAX)
                    .cmp(&original_rank.get(&right.0).copied().unwrap_or(usize::MAX))
            })
    });
    let id_to_memory = ordered
        .into_iter()
        .map(|memory| (memory.id, memory))
        .collect::<HashMap<_, _>>();
    let ordered = demoted
        .iter()
        .filter_map(|(id, _)| id_to_memory.get(id).cloned())
        .collect::<Vec<_>>();
    Ok((ordered, demoted))
}

pub(super) fn label_for_memory(
    conn: &Connection,
    memory: &Memory,
    now_epoch: i64,
) -> memory::MemoryStalenessLabel {
    memory::memory_staleness_label_with_conn(conn, memory, now_epoch)
        .unwrap_or_else(|_| memory::memory_staleness_label(memory, now_epoch))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    #[test]
    fn apply_score_demotions_ranks_verify_before_trust_lower() -> anyhow::Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        seed_memory(&conn, 1, "stale-session", r#"["src/stale.rs"]"#)?;
        seed_memory(&conn, 2, "fresh-session", r#"["src/fresh.rs"]"#)?;
        link_commit(
            &conn,
            1,
            "source-stale",
            100,
            &["src/stale.rs"],
            "stale-session",
        )?;
        insert_commit(&conn, 2, "later-stale", 200, &["src/stale.rs"])?;
        link_commit(
            &conn,
            3,
            "source-fresh",
            100,
            &["src/fresh.rs"],
            "fresh-session",
        )?;

        let ordered = memory::get_memories_by_ids(&conn, &[1, 2], None)?;
        let fused = vec![(1, 1.0), (2, 0.9)];

        let (ordered, demoted) = apply_score_demotions(&conn, &fused, ordered)?;

        assert_eq!(
            ordered.iter().map(|memory| memory.id).collect::<Vec<_>>(),
            vec![2, 1]
        );
        assert_eq!(demoted[0].0, 2);
        assert!(demoted[1].1 < demoted[0].1);
        Ok(())
    }

    fn seed_memory(
        conn: &Connection,
        id: i64,
        session_id: &str,
        files: &str,
    ) -> anyhow::Result<()> {
        conn.execute(
            "INSERT INTO memories
             (id, session_id, project, topic_key, title, content, memory_type, files,
              created_at_epoch, updated_at_epoch, status, branch, scope)
             VALUES (?1, ?2, 'proj', ?3, ?4, ?5, 'decision', ?6, 100, 100,
                     'active', 'main', 'project')",
            params![
                id,
                session_id,
                format!("topic-{id}"),
                format!("Memory {id}"),
                format!("Memory {id} content"),
                files
            ],
        )?;
        Ok(())
    }

    fn link_commit(
        conn: &Connection,
        id: i64,
        sha: &str,
        epoch: i64,
        changed_files: &[&str],
        memory_session_id: &str,
    ) -> anyhow::Result<()> {
        insert_commit(conn, id, sha, epoch, changed_files)?;
        conn.execute(
            "INSERT INTO git_commit_sessions
             (commit_id, session_id, memory_session_id, source, linked_at_epoch)
             VALUES (?1, ?2, ?3, 'test', ?4)",
            params![id, format!("content-{id}"), memory_session_id, epoch],
        )?;
        Ok(())
    }

    fn insert_commit(
        conn: &Connection,
        id: i64,
        sha: &str,
        epoch: i64,
        changed_files: &[&str],
    ) -> anyhow::Result<()> {
        let changed_files = serde_json::to_string(changed_files)?;
        conn.execute(
            "INSERT INTO git_commits
             (id, project, repo_path, sha, short_sha, branch, message,
              authored_at_epoch, changed_files, created_at_epoch, updated_at_epoch)
             VALUES (?1, 'proj', '/repo', ?2, ?2, 'main', NULL, ?3, ?4, ?3, ?3)",
            params![id, sha, epoch, changed_files],
        )?;
        Ok(())
    }
}
