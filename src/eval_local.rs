//! Local eval: checks memory quality against the real database.
//! Run via `remem eval-local` to get a health report.

use anyhow::Result;
use rusqlite::{params, Connection};
use std::collections::HashMap;

pub struct EvalReport {
    pub total_memories: i64,
    pub dedup: DedupReport,
    pub project_leak: ProjectLeakReport,
    pub title_quality: TitleQualityReport,
    pub self_retrieval: SelfRetrievalReport,
}

pub struct DedupReport {
    pub duplicate_groups: usize,
    pub duplicate_count: i64,
    pub duplicate_rate: f64,
    pub worst_groups: Vec<(String, i64)>, // (title_preview, count)
}

pub struct ProjectLeakReport {
    pub total_tested: usize,
    pub leaked: usize,
    pub leak_rate: f64,
}

pub struct TitleQualityReport {
    pub total: i64,
    pub bullet_prefix: i64,
    pub too_long: i64,
    pub bullet_rate: f64,
}

pub struct SelfRetrievalReport {
    pub total_tested: usize,
    pub found: usize,
    pub retrieval_rate: f64,
}

impl std::fmt::Display for EvalReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "=== remem eval-local ({} memories) ===\n",
            self.total_memories
        )?;

        // Dedup
        writeln!(
            f,
            "[dedup] {} duplicates in {} groups ({:.1}%)",
            self.dedup.duplicate_count,
            self.dedup.duplicate_groups,
            self.dedup.duplicate_rate * 100.0
        )?;
        if !self.dedup.worst_groups.is_empty() {
            writeln!(f, "  worst:")?;
            for (preview, count) in &self.dedup.worst_groups {
                writeln!(f, "    {}x  {}", count, preview)?;
            }
        }

        // Project leak
        writeln!(
            f,
            "\n[project_filter] tested {} entities, {} leaked ({:.1}%)",
            self.project_leak.total_tested,
            self.project_leak.leaked,
            self.project_leak.leak_rate * 100.0
        )?;

        // Title quality
        writeln!(
            f,
            "\n[title_quality] {:.1}% start with bullet, {:.1}% too long (>{} chars)",
            self.title_quality.bullet_rate * 100.0,
            if self.title_quality.total > 0 {
                self.title_quality.too_long as f64 / self.title_quality.total as f64 * 100.0
            } else {
                0.0
            },
            MAX_GOOD_TITLE_LEN
        )?;

        // Self-retrieval
        writeln!(
            f,
            "\n[self_retrieval] {}/{} ({:.1}%)",
            self.self_retrieval.found,
            self.self_retrieval.total_tested,
            self.self_retrieval.retrieval_rate * 100.0
        )?;

        // Overall score
        let score = self.overall_score();
        writeln!(f, "\n--- overall: {:.1}/5.0 ---", score)?;
        Ok(())
    }
}

const MAX_GOOD_TITLE_LEN: usize = 120;

impl EvalReport {
    pub fn overall_score(&self) -> f64 {
        // Weight: dedup 30%, project_filter 25%, title 15%, self_retrieval 30%
        let dedup_score = (1.0 - self.dedup.duplicate_rate).max(0.0) * 5.0;
        let leak_score = (1.0 - self.project_leak.leak_rate).max(0.0) * 5.0;
        let title_score = (1.0 - self.title_quality.bullet_rate).max(0.0) * 5.0;
        let retrieval_score = self.self_retrieval.retrieval_rate * 5.0;

        dedup_score * 0.30 + leak_score * 0.25 + title_score * 0.15 + retrieval_score * 0.30
    }
}

/// Run all eval checks against the database.
pub fn run_eval(conn: &Connection) -> Result<EvalReport> {
    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE status = 'active'",
        [],
        |r| r.get(0),
    )?;

    let dedup = check_dedup(conn)?;
    let project_leak = check_project_leak(conn)?;
    let title_quality = check_title_quality(conn)?;
    let self_retrieval = check_self_retrieval(conn)?;

    Ok(EvalReport {
        total_memories: total,
        dedup,
        project_leak,
        title_quality,
        self_retrieval,
    })
}

/// Check for near-duplicate memories by comparing normalized content hashes.
fn check_dedup(conn: &Connection) -> Result<DedupReport> {
    use std::hash::{Hash, Hasher};

    let mut stmt =
        conn.prepare("SELECT id, title, content FROM memories WHERE status = 'active'")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;

    let mut hash_groups: HashMap<u64, Vec<(i64, String)>> = HashMap::new();
    for row in rows {
        let (id, title, content) = row?;
        // Normalize: lowercase, strip whitespace variations, take first 200 chars
        let normalized: String = content
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == ' ')
            .take(200)
            .collect();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        normalized.hash(&mut hasher);
        let h = hasher.finish();
        hash_groups.entry(h).or_default().push((id, title));
    }

    let mut duplicate_count: i64 = 0;
    let mut duplicate_groups = 0;
    let mut worst: Vec<(String, i64)> = Vec::new();

    for (_hash, entries) in &hash_groups {
        if entries.len() > 1 {
            let count = entries.len() as i64;
            duplicate_groups += 1;
            duplicate_count += count - 1; // excess copies
            let preview: String = entries[0].1.chars().take(60).collect();
            worst.push((preview, count));
        }
    }
    worst.sort_by(|a, b| b.1.cmp(&a.1));
    worst.truncate(5);

    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE status = 'active'",
        [],
        |r| r.get(0),
    )?;
    let rate = if total > 0 {
        duplicate_count as f64 / total as f64
    } else {
        0.0
    };

    Ok(DedupReport {
        duplicate_groups,
        duplicate_count,
        duplicate_rate: rate,
        worst_groups: worst,
    })
}

/// Check if entity search leaks across projects.
fn check_project_leak(conn: &Connection) -> Result<ProjectLeakReport> {
    // Get the top 5 projects by memory count
    let mut stmt = conn.prepare(
        "SELECT project, COUNT(*) as cnt FROM memories
         WHERE status = 'active' AND project != ''
         GROUP BY project ORDER BY cnt DESC LIMIT 5",
    )?;
    let projects: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .flatten()
        .collect();

    if projects.len() < 2 {
        return Ok(ProjectLeakReport {
            total_tested: 0,
            leaked: 0,
            leak_rate: 0.0,
        });
    }

    let mut total_tested = 0;
    let mut leaked = 0;

    // For each project, pick a distinctive entity and search with project filter
    for proj in &projects {
        // Find entities unique to this project
        let mut estmt = conn.prepare(
            "SELECT DISTINCT e.canonical_name FROM memory_entities me
             JOIN entities e ON e.id = me.entity_id
             JOIN memories m ON m.id = me.memory_id
             WHERE m.project = ?1 AND m.status = 'active'
             LIMIT 3",
        )?;
        let entities: Vec<String> = estmt
            .query_map(params![proj], |r| r.get::<_, String>(0))?
            .flatten()
            .collect();

        for entity in &entities {
            // Search with project filter
            let results = crate::entity::search_by_entity(conn, entity, Some(proj), 20)?;
            // Check if any returned memory is from a different project
            for mid in &results {
                let mem_proj: String = conn
                    .query_row(
                        "SELECT project FROM memories WHERE id = ?1",
                        params![mid],
                        |r| r.get(0),
                    )
                    .unwrap_or_default();
                if !crate::project_id::project_matches(Some(&mem_proj), proj) {
                    leaked += 1;
                }
            }
            total_tested += 1;
        }
    }

    let rate = if total_tested > 0 {
        leaked as f64 / total_tested.max(1) as f64
    } else {
        0.0
    };
    Ok(ProjectLeakReport {
        total_tested,
        leaked,
        leak_rate: rate,
    })
}

/// Check title quality: bullet-prefixed titles are bad for FTS.
fn check_title_quality(conn: &Connection) -> Result<TitleQualityReport> {
    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE status = 'active'",
        [],
        |r| r.get(0),
    )?;

    let bullet_prefix: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE status = 'active'
         AND (title LIKE '• %' OR title LIKE '- %' OR title LIKE '* %'
              OR title LIKE 'Preference: %')",
        [],
        |r| r.get(0),
    )?;

    let too_long: i64 = conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM memories WHERE status = 'active' AND LENGTH(title) > {}",
            MAX_GOOD_TITLE_LEN
        ),
        [],
        |r| r.get(0),
    )?;

    let rate = if total > 0 {
        bullet_prefix as f64 / total as f64
    } else {
        0.0
    };
    Ok(TitleQualityReport {
        total,
        bullet_prefix,
        too_long,
        bullet_rate: rate,
    })
}

/// Self-retrieval: search for recent memories by their own key terms.
fn check_self_retrieval(conn: &Connection) -> Result<SelfRetrievalReport> {
    let mut stmt = conn.prepare(
        "SELECT id, title, project FROM memories
         WHERE status = 'active' AND LENGTH(title) > 20
         ORDER BY updated_at_epoch DESC LIMIT 20",
    )?;
    let recent: Vec<(i64, String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .flatten()
        .collect();

    let mut found = 0;
    let total = recent.len();

    for (id, title, project) in &recent {
        // Extract 2-3 key words from title for search
        let words: Vec<&str> = title
            .split_whitespace()
            .filter(|w| w.len() > 3 && !w.starts_with('—') && !w.starts_with('['))
            .take(3)
            .collect();
        if words.is_empty() {
            continue;
        }
        let query = words.join(" ");
        let results = crate::search::search(conn, Some(&query), Some(project), None, 20, 0, true)?;
        if results.iter().any(|m| m.id == *id) {
            found += 1;
        }
    }

    let rate = if total > 0 {
        found as f64 / total as f64
    } else {
        0.0
    };
    Ok(SelfRetrievalReport {
        total_tested: total,
        found,
        retrieval_rate: rate,
    })
}
